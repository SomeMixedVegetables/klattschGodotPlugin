// FormantSynth: the klattsch synthesis engine, free of any audio-API dependency.
//
// Usage:
//
//   let mut synth = FormantSynth::new(48000.0, None)?;
//   synth.queue_schedule(&compiled.voices[0].schedule, 0.0);
//   let mut buf = vec![0.0f32; 48000 * 2]; // 2 seconds
//   synth.process(&mut buf);
//
// The schedule is a list of { at_ms, target, transition_ms } events; the synth
// applies them in time order. Or drive it live with set_target()

use std::ops::{Index, IndexMut};

use crate::dsp::{glottal_pulse, soft_clip, xorshift, BandpassBiquad};
use crate::error::{Error, Result};
use crate::js;
use crate::sequencer::{CompileResult, ScheduleEvent};
use crate::value::Map;

pub const PARAM_COUNT: usize = 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum Param {
    F0,
    Voicing,
    F1,
    BW1,
    A1,
    F2,
    BW2,
    A2,
    F3,
    BW3,
    A3,
    Gain,
    VibratoDepth, // Hz peak deviation
    VibratoRate,  // Hz LFO rate
    TremoloDepth, // 0..1 amplitude modulation depth
    TremoloRate,  // Hz tremolo LFO rate
    Aspiration,   // 0..1 noise mixed into voiced source (breathiness)
    Tilt,         // -0.95..0.95 spectral tilt (positive = brighter)
    Effort,       // 0..1 glottal pulse shape (0=lax, 1=tense)
}

impl Param {
    pub const ALL: [Param; PARAM_COUNT] = [
        Param::F0, Param::Voicing,
        Param::F1, Param::BW1, Param::A1,
        Param::F2, Param::BW2, Param::A2,
        Param::F3, Param::BW3, Param::A3,
        Param::Gain,
        Param::VibratoDepth, Param::VibratoRate,
        Param::TremoloDepth, Param::TremoloRate,
        Param::Aspiration, Param::Tilt, Param::Effort,
    ];

    pub fn name(self) -> &'static str {
        PARAMS[self as usize]
    }

    pub fn from_name(name: &str) -> Option<Param> {
        PARAMS.iter().position(|p| *p == name).map(|i| Param::ALL[i])
    }
}

// Schedule-target key names, in the same order as `Param`.
pub const PARAMS: [&str; PARAM_COUNT] = [
    "F0", "voicing",
    "F1", "BW1", "A1",
    "F2", "BW2", "A2",
    "F3", "BW3", "A3",
    "gain",
    "vibratoDepth",
    "vibratoRate",
    "tremoloDepth",
    "tremoloRate",
    "aspiration",
    "tilt",
    "effort",
];

// A full set of synth parameter values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params(pub [f64; PARAM_COUNT]);

pub const DEFAULT: Params = Params([
    120.0, 0.0, // F0, voicing
    500.0, 80.0, 0.0, // F1, BW1, A1
    1500.0, 120.0, 0.0, // F2, BW2, A2
    2500.0, 160.0, 0.0, // F3, BW3, A3
    3.5, // gain
    0.0, // vibratoDepth
    5.0, // vibratoRate
    0.0, // tremoloDepth
    5.0, // tremoloRate
    0.0, // aspiration
    0.0, // tilt
    0.5, // effort
]);

impl Default for Params {
    fn default() -> Self {
        DEFAULT
    }
}

impl Index<Param> for Params {
    type Output = f64;
    fn index(&self, p: Param) -> &f64 {
        &self.0[p as usize]
    }
}

impl IndexMut<Param> for Params {
    fn index_mut(&mut self, p: Param) -> &mut f64 {
        &mut self.0[p as usize]
    }
}

impl Params {
    // `{ ...DEFAULT, ...init }`
    pub fn with_overrides(init: Option<&PartialParams>) -> Params {
        let mut p = DEFAULT;
        if let Some(init) = init {
            p.apply(init);
        }
        p
    }

    pub fn apply(&mut self, partial: &PartialParams) {
        for (dst, src) in self.0.iter_mut().zip(partial.0.iter()) {
            if let Some(v) = src {
                *dst = *v;
            }
        }
    }
}

// A target that sets only some parameters; unset ones keep their current
// target (the JS engine's `k in target` check).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PartialParams(pub [Option<f64>; PARAM_COUNT]);

impl PartialParams {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, p: Param) -> Option<f64> {
        self.0[p as usize]
    }

    pub fn set(&mut self, p: Param, value: Option<f64>) {
        self.0[p as usize] = value;
    }

    pub fn with(mut self, p: Param, value: f64) -> Self {
        self.0[p as usize] = Some(value);
        self
    }

    // Read the synth parameters out of a schedule target. Values are coerced
    // with JS ToNumber; keys that are not synth parameters are ignored.
    pub fn from_map(target: &Map) -> Self {
        let mut out = Self::default();
        for (i, name) in PARAMS.iter().enumerate() {
            if let Some(v) = target.get(*name) {
                out.0[i] = Some(v.to_number());
            }
        }
        out
    }
}

pub const DEFAULT_TRANSITION_MS: f64 = 30.0;

// One event for the synth's own queue. `None` times take the JS defaults
// (`atMs ?? 0`, `transitionMs ?? 30`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SynthEvent {
    pub at_ms: Option<f64>,
    pub target: PartialParams,
    pub transition_ms: Option<f64>,
}

impl From<&ScheduleEvent> for SynthEvent {
    fn from(e: &ScheduleEvent) -> Self {
        Self {
            at_ms: Some(e.at_ms),
            target: PartialParams::from_map(&e.target),
            transition_ms: Some(e.transition_ms),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct QueuedEvent {
    at_sample: f64,
    target: PartialParams,
    transition_samples: f64,
}

// `transitionMs` as a sample count: `Math.max(1, Math.floor(ms * sr / 1000))`.
fn transition_samples(ms: f64, sr: f64) -> f64 {
    js::max(1.0, (ms * sr / 1000.0).floor())
}

// A schedule converted to sample times for one sample rate. Building one
// allocates; queueing it (FormantSynth::queue_prepared) does not, so a
// real-time audio thread can take schedules prepared elsewhere.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreparedSchedule {
    events: Vec<QueuedEvent>,
    sample_rate: f64,
    // Where the utterance ends on its own clock, in samples: the point an
    // appended schedule starts from (see FormantSynth::append_prepared).
    length: f64,
    // Caller's id for the utterance, reported by FormantSynth::utterance_tag
    // while it plays (0 = untagged).
    tag: u64,
}

impl PreparedSchedule {
    pub fn from_events(sample_rate: f64, events: &[SynthEvent]) -> Self {
        let events = events
            .iter()
            .map(|e| QueuedEvent {
                at_sample: (e.at_ms.unwrap_or(0.0) * sample_rate / 1000.0).floor(),
                target: e.target,
                transition_samples: transition_samples(
                    e.transition_ms.unwrap_or(DEFAULT_TRANSITION_MS),
                    sample_rate,
                ),
            })
            .collect::<Vec<_>>();
        // Without a known duration, the utterance ends when its last glide does.
        let length = events.iter().map(|e| e.at_sample + e.transition_samples).fold(0.0, f64::max);
        Self { events, sample_rate, length, tag: 0 }
    }

    // Tag the utterance with an id (for lip-sync and speech events: see
    // FormantSynth::utterance_tag).
    pub fn with_tag(mut self, tag: u64) -> Self {
        self.tag = tag;
        self
    }

    pub fn tag(&self) -> u64 {
        self.tag
    }

    // Set the utterance's duration (a CompileResult's `total_ms`), so a
    // schedule appended after this one starts exactly when it ends.
    pub fn with_duration_ms(mut self, total_ms: f64) -> Self {
        self.length = (total_ms * self.sample_rate / 1000.0).floor().max(0.0);
        self
    }

    // The utterance's length in samples.
    pub fn length(&self) -> f64 {
        self.length
    }

    pub fn from_schedule(sample_rate: f64, events: &[ScheduleEvent]) -> Self {
        let events: Vec<SynthEvent> = events.iter().map(SynthEvent::from).collect();
        Self::from_events(sample_rate, &events)
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct FormantSynth {
    sr: f64,
    current: Params,
    target: Params,
    increment: [f64; PARAM_COUNT],
    transition_samples: f64,
    glottal_phase: f64,
    lfsr: i32,
    vibrato_phase: f64,
    tremolo_phase: f64,
    tilt_prev: f64,
    bp1: BandpassBiquad,
    bp2: BandpassBiquad,
    bp3: BandpassBiquad,
    schedule: Vec<QueuedEvent>,
    schedule_idx: usize,
    sample_counter: f64,
    // End of the current utterance on the schedule clock.
    schedule_end: f64,
    // A schedule appended to start at schedule_end (when has_next), or a
    // spent one kept so the audio thread never frees it.
    next: Vec<QueuedEvent>,
    next_length: f64,
    has_next: bool,
    // Tags of the current and the appended utterance.
    tag: u64,
    next_tag: u64,
}

const LFSR_SEED: i32 = 0xACE1ACE1u32 as i32;

impl FormantSynth {
    pub fn new(sample_rate: f64, initial_target: Option<&PartialParams>) -> Result<Self> {
        if !(sample_rate > 0.0) {
            return Err(Error::new("FormantSynth requires a positive sampleRate"));
        }
        let current = Params::with_overrides(initial_target);
        Ok(Self {
            sr: sample_rate,
            current,
            target: current,
            increment: [0.0; PARAM_COUNT],
            transition_samples: 0.0,
            glottal_phase: 0.0,
            lfsr: LFSR_SEED,
            vibrato_phase: 0.0,
            tremolo_phase: 0.0,
            tilt_prev: 0.0,
            bp1: BandpassBiquad::new(),
            bp2: BandpassBiquad::new(),
            bp3: BandpassBiquad::new(),
            schedule: Vec::new(),
            schedule_idx: 0,
            sample_counter: 0.0,
            schedule_end: 0.0,
            next: Vec::new(),
            next_length: 0.0,
            has_next: false,
            tag: 0,
            next_tag: 0,
        })
    }

    // Equivalent to `new FormantSynth({ sampleRate, initialTarget, schedule })`.
    pub fn with_schedule(
        sample_rate: f64,
        initial_target: Option<&PartialParams>,
        schedule: &[ScheduleEvent],
    ) -> Result<Self> {
        let mut s = Self::new(sample_rate, initial_target)?;
        s.queue_schedule(schedule, 0.0);
        Ok(s)
    }

    pub fn sample_rate(&self) -> f64 {
        self.sr
    }

    // Parameter values as of the last rendered sample.
    pub fn current(&self) -> &Params {
        &self.current
    }

    // Values the synth is currently gliding toward.
    pub fn target(&self) -> &Params {
        &self.target
    }

    // Samples elapsed on the schedule clock.
    pub fn sample_counter(&self) -> f64 {
        self.sample_counter
    }

    // True once every queued schedule event has been applied.
    pub fn schedule_done(&self) -> bool {
        self.schedule_idx >= self.schedule.len()
    }

    // The tag of the utterance the schedule clock belongs to (the one
    // playing, or the last one played); 0 after reset or an untagged
    // schedule. Its time zero is `sample_counter()` samples ago.
    pub fn utterance_tag(&self) -> u64 {
        self.tag
    }

    // True while the current utterance has not reached its end (including
    // one queued to start later), or an appended one is waiting.
    pub fn speaking(&self) -> bool {
        self.has_next || self.sample_counter < self.schedule_end
    }

    // Schedule a new target. transition_ms samples are linearly interpolated
    // from current state to the new target
    pub fn set_target(&mut self, target: &PartialParams, transition_ms: f64) {
        let n = transition_samples(transition_ms, self.sr);
        self.transition_samples = n;
        self.retarget(target, n);
    }

    fn retarget(&mut self, target: &PartialParams, n: f64) {
        for k in 0..PARAM_COUNT {
            if let Some(v) = target.0[k] {
                self.target.0[k] = v;
            }
            self.increment[k] = (self.target.0[k] - self.current.0[k]) / n;
        }
    }

    // start_counter presets the schedule clock: negative delays the first event,
    // positive drains already-past events on the first process() call.
    pub fn queue_events(&mut self, events: &[SynthEvent], start_counter: f64) {
        self.queue_prepared(PreparedSchedule::from_events(self.sr, events), start_counter);
    }

    pub fn queue_schedule(&mut self, events: &[ScheduleEvent], start_counter: f64) {
        self.queue_prepared(PreparedSchedule::from_schedule(self.sr, events), start_counter);
    }

    // Queue a schedule prepared for this synth's sample rate, without
    // allocating. Returns the replaced schedule so the caller can free it
    // off the audio thread.
    pub fn queue_prepared(&mut self, schedule: PreparedSchedule, start_counter: f64) -> PreparedSchedule {
        debug_assert!(
            schedule.events.is_empty() || schedule.sample_rate == self.sr,
            "schedule prepared for {} Hz queued on a {} Hz synth",
            schedule.sample_rate,
            self.sr
        );
        let old = std::mem::replace(&mut self.schedule, schedule.events);
        self.schedule_idx = 0;
        self.sample_counter = start_counter;
        self.schedule_end = schedule.length;
        self.tag = schedule.tag;
        self.has_next = false; // a new utterance replaces anything appended
        PreparedSchedule { events: old, sample_rate: self.sr, length: 0.0, tag: 0 }
    }

    // Queue a schedule to start the moment the current utterance ends (its
    // PreparedSchedule length), without a gap or allocation. If nothing is
    // speaking it starts now, like queue_prepared. Only one schedule waits at
    // a time: appending again replaces a waiting one. Returns the schedule
    // this displaced (a waiting or spent one) for the caller to free.
    pub fn append_prepared(&mut self, schedule: PreparedSchedule) -> PreparedSchedule {
        if !self.speaking() {
            return self.queue_prepared(schedule, 0.0);
        }
        debug_assert!(
            schedule.events.is_empty() || schedule.sample_rate == self.sr,
            "schedule prepared for {} Hz appended on a {} Hz synth",
            schedule.sample_rate,
            self.sr
        );
        let old = std::mem::replace(&mut self.next, schedule.events);
        self.next_length = schedule.length;
        self.next_tag = schedule.tag;
        self.has_next = true;
        PreparedSchedule { events: old, sample_rate: self.sr, length: 0.0, tag: 0 }
    }

    pub fn reset(&mut self, initial_target: Option<&PartialParams>) {
        self.glottal_phase = 0.0;
        self.vibrato_phase = 0.0;
        self.tremolo_phase = 0.0;
        self.lfsr = LFSR_SEED;
        self.tilt_prev = 0.0;
        self.bp1.reset();
        self.bp2.reset();
        self.bp3.reset();
        self.current = Params::with_overrides(initial_target);
        self.target = self.current;
        self.increment = [0.0; PARAM_COUNT];
        self.transition_samples = 0.0;
        self.schedule.clear();
        self.schedule_idx = 0;
        self.sample_counter = 0.0;
        self.schedule_end = 0.0;
        self.has_next = false;
        self.tag = 0;
    }

    // Drain any baked-in schedule events whose time has arrived
    fn drain_events(&mut self) {
        while self.schedule_idx < self.schedule.len()
            && self.schedule[self.schedule_idx].at_sample <= self.sample_counter
        {
            let evt = self.schedule[self.schedule_idx];
            self.schedule_idx += 1;
            let n = evt.transition_samples;
            self.transition_samples = n;
            self.retarget(&evt.target, n);
        }
    }

    // The current utterance has ended: the appended schedule takes over, its
    // time zero at the old end. Swapping keeps the spent schedule's memory
    // in `next` instead of freeing it here.
    fn start_next(&mut self) {
        std::mem::swap(&mut self.schedule, &mut self.next);
        self.has_next = false;
        self.schedule_idx = 0;
        self.sample_counter -= self.schedule_end;
        self.schedule_end = self.next_length;
        self.tag = self.next_tag;
    }

    // Render `out.len()` samples into the given buffer
    pub fn process(&mut self, out: &mut [f32]) {
        use Param::*;
        let two_pi = 2.0 * std::f64::consts::PI;
        for sample in out.iter_mut() {
            self.drain_events();
            if self.has_next && self.sample_counter >= self.schedule_end {
                self.start_next();
                self.drain_events();
            }
            self.sample_counter += 1.0;

            if self.transition_samples > 0.0 {
                for k in 0..PARAM_COUNT {
                    self.current.0[k] += self.increment[k];
                }
                self.transition_samples -= 1.0;
                if self.transition_samples == 0.0 {
                    self.current = self.target;
                }
            }
            let cur = &self.current;

            // Vibrato LFO modulates F0 around its target value
            self.vibrato_phase += 2.0 * std::f64::consts::PI * cur[VibratoRate] / self.sr;
            self.vibrato_phase -= two_pi * (self.vibrato_phase / two_pi).floor();
            let eff_f0 = cur[F0] + cur[VibratoDepth] * js::sin(self.vibrato_phase);

            // Tremolo LFO modulates output amplitude
            self.tremolo_phase += 2.0 * std::f64::consts::PI * cur[TremoloRate] / self.sr;
            self.tremolo_phase -= two_pi * (self.tremolo_phase / two_pi).floor();
            let tremolo_mod = 1.0 - cur[TremoloDepth] * (0.5 + 0.5 * js::sin(self.tremolo_phase));

            let voicing = cur[Voicing];
            let v = if voicing < 0.0 { 0.0 } else if voicing > 1.0 { 1.0 } else { voicing };
            self.lfsr = xorshift(self.lfsr);
            let noise_sample = f64::from(self.lfsr) / 2147483648.0;
            let pulse_val = glottal_pulse(self.glottal_phase, cur[Effort]);
            let voiced_gain = 1.0 - cur[Aspiration] * 0.85;
            let exc = v * pulse_val * voiced_gain
                + (1.0 - v) * noise_sample * 0.35
                + cur[Aspiration] * noise_sample * 0.5;
            self.glottal_phase += eff_f0 / self.sr;
            self.glottal_phase -= self.glottal_phase.floor();

            self.bp1.set_freq(cur[F1], cur[BW1], self.sr);
            self.bp2.set_freq(cur[F2], cur[BW2], self.sr);
            self.bp3.set_freq(cur[F3], cur[BW3], self.sr);

            let y = (self.bp1.process(exc) * cur[A1]
                + self.bp2.process(exc) * cur[A2]
                + self.bp3.process(exc) * cur[A3])
                * cur[Gain]
                * tremolo_mod;

            let tilted = y - cur[Tilt] * self.tilt_prev;
            self.tilt_prev = y;

            *sample = soft_clip(tilted) as f32;
        }
    }
}

// Length of a Float32Array sized `Math.ceil(totalMs * sampleRate / 1000)`.
pub fn buffer_len(total_ms: f64, sample_rate: f64) -> Result<usize> {
    let n = (total_ms * sample_rate / 1000.0).ceil();
    if n.is_nan() {
        return Ok(0);
    }
    if n < 0.0 || n > (u32::MAX as f64) {
        return Err(Error::new(format!("Invalid typed array length: {}", js::number_to_string(n))));
    }
    Ok(n as usize)
}

// Convenience: render a complete utterance offline
pub fn render_to_buffer(
    sample_rate: f64,
    schedule: &[ScheduleEvent],
    total_ms: Option<f64>,
    initial_target: Option<&PartialParams>,
) -> Result<Vec<f32>> {
    let total_ms = match total_ms {
        Some(t) => t,
        None => match schedule.last() {
            Some(last) => last.at_ms + 200.0,
            None => return Err(Error::new("renderToBuffer needs totalMs or a non-empty schedule")),
        },
    };
    let mut buf = vec![0.0f32; buffer_len(total_ms, sample_rate)?];
    let mut synth = FormantSynth::with_schedule(sample_rate, initial_target, schedule)?;
    synth.process(&mut buf);
    Ok(buf)
}

// Render every voice section to its own buffer and sum, as the CLI does. The
// mix is not clipped; encode_wav peak-normalizes it.
pub fn render_mix(result: &CompileResult, sample_rate: f64) -> Result<Vec<f32>> {
    let mut buf = vec![0.0f32; buffer_len(result.total_ms, sample_rate)?];
    for v in &result.voices {
        if v.schedule.is_empty() {
            continue;
        }
        let vb = render_to_buffer(sample_rate, &v.schedule, Some(v.total_ms), None)?;
        for (dst, src) in buf.iter_mut().zip(vb.iter()) {
            *dst = (f64::from(*dst) + f64::from(*src)) as f32;
        }
    }
    Ok(buf)
}
