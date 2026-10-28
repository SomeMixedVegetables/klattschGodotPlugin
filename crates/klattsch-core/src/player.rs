// Multi-voice real-time playback: the Rust equivalent of the JS engine's
// AudioWorklet wrapper (src/formant-worklet.js), with no audio API attached.
//
// FormantProcessor is the worklet's logic: one FormantSynth per voice,
// created on first use, driven by `frame` / `schedule` / `reset` messages and
// rendered one output block per voice. It keeps the worklet's semantics
// exactly, including `startTime` (a schedule that should have started at an
// earlier or later point on the audio clock).
//
// Player and PlayerHandle split a processor across threads for a real audio
// engine. The handle (game thread) prepares schedules and sends commands
// through a fixed-size queue; the player (audio thread) applies them at the
// start of each block and renders without allocating. Replaced schedules go
// back to the handle to be freed off the audio thread.
//
//   let (handle, mut player) = player(48000.0, ProcessorOptions::default(), 256)?;
//   // game thread:
//   handle.schedule(Some(0), compiled.schedule(), None)?;
//   // audio thread, per block:
//   player.process_mix(&mut block, 1);

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::Arc;

use crate::error::Result;
use crate::js;
use crate::sequencer::{Phrase, ScheduleEvent};
use crate::synth::{FormantSynth, PartialParams, PreparedSchedule, SynthEvent, DEFAULT_TRANSITION_MS};

// Voice slots reserved up front so the first message to a new voice does not
// allocate on the audio thread. Higher voice numbers still work; the first
// use of each allocates once.
pub const RESERVED_VOICES: usize = 16;

// A message to the processor, like the worklet's `port` messages. A `None`
// voice means voice 0, except for Reset, where it means every voice.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // `{ type: 'frame', target, transitionMs }`: glide to a target now.
    Frame { voice: Option<usize>, target: PartialParams, transition_ms: f64 },
    // `{ type: 'schedule', schedule, startTime }`: replace the voice's
    // schedule. `start_time` (seconds on the processor clock) is when the
    // schedule's time zero is; in the past, its early events apply at once.
    Schedule { voice: Option<usize>, schedule: PreparedSchedule, start_time: Option<f64> },
    // Not in the worklet: start the schedule when the voice's current
    // utterance ends (the end of its PreparedSchedule length), or now if it
    // is silent. One appended schedule waits per voice; appending again
    // replaces it, and a Schedule or Reset cancels it.
    Append { voice: Option<usize>, schedule: PreparedSchedule },
    // `{ type: 'reset', initialTarget }`
    Reset { voice: Option<usize>, initial_target: Option<PartialParams> },
}

// The worklet's `processorOptions`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcessorOptions {
    // Legacy single-voice options: seed voice 0.
    pub schedule: Option<Vec<SynthEvent>>,
    pub initial_target: Option<PartialParams>,
    // One schedule per voice; `None` or empty entries leave a voice unset.
    pub schedules: Vec<Option<Vec<SynthEvent>>>,
}

pub struct FormantProcessor {
    sample_rate: f64,
    synths: Vec<Option<FormantSynth>>, // index = voice
    frames: u64,
}

impl FormantProcessor {
    pub fn new(sample_rate: f64, opts: &ProcessorOptions) -> Result<Self> {
        FormantSynth::new(sample_rate, None)?; // the worklet throws on first use instead
        let mut p = Self { sample_rate, synths: Vec::with_capacity(RESERVED_VOICES), frames: 0 };
        // Legacy single-voice construction options seed voice 0.
        if opts.schedule.is_some() || opts.initial_target.is_some() {
            let mut synth = FormantSynth::new(sample_rate, opts.initial_target.as_ref())?;
            synth.queue_events(opts.schedule.as_deref().unwrap_or_default(), 0.0);
            p.slot(0).replace(synth);
        }
        // Multi-voice construction.
        for (i, s) in opts.schedules.iter().enumerate() {
            if let Some(s) = s.as_ref().filter(|s| !s.is_empty()) {
                p.ensure(i).queue_events(s, 0.0);
            }
        }
        Ok(p)
    }

    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    // The processor clock in seconds: frames rendered / sample rate, like the
    // worklet's `currentTime`.
    pub fn current_time(&self) -> f64 {
        self.frames as f64 / self.sample_rate
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn voice(&self, voice: usize) -> Option<&FormantSynth> {
        self.synths.get(voice).and_then(Option::as_ref)
    }

    fn slot(&mut self, voice: usize) -> &mut Option<FormantSynth> {
        if voice >= self.synths.len() {
            self.synths.resize_with(voice + 1, || None);
        }
        &mut self.synths[voice]
    }

    fn ensure(&mut self, voice: usize) -> &mut FormantSynth {
        let sr = self.sample_rate;
        self.slot(voice).get_or_insert_with(|| FormantSynth::new(sr, None).expect("sample rate checked in new"))
    }

    // Apply one message. Returns the schedule a Schedule command replaced, so
    // the caller can free it where allocation is allowed.
    pub fn handle(&mut self, cmd: Command) -> Option<PreparedSchedule> {
        match cmd {
            Command::Frame { voice, target, transition_ms } => {
                self.ensure(voice.unwrap_or(0)).set_target(&target, transition_ms);
                None
            }
            Command::Schedule { voice, schedule, start_time } => {
                let start_counter = match start_time {
                    Some(t) => js::round((self.current_time() - t) * self.sample_rate),
                    None => 0.0,
                };
                Some(self.ensure(voice.unwrap_or(0)).queue_prepared(schedule, start_counter))
            }
            Command::Append { voice, schedule } => Some(self.ensure(voice.unwrap_or(0)).append_prepared(schedule)),
            Command::Reset { voice: None, initial_target } => {
                for synth in self.synths.iter_mut().flatten() {
                    synth.reset(initial_target.as_ref());
                }
                None
            }
            Command::Reset { voice: Some(v), initial_target } => {
                self.ensure(v).reset(initial_target.as_ref());
                None
            }
        }
    }

    // Render one block of `frames` samples: `outputs[v]` receives voice v.
    // Voices with no synth yet are silent; an empty output buffer (an output
    // with no channels) leaves its voice paused, as in the worklet. Copy a
    // voice into more channels yourself if needed.
    pub fn process(&mut self, frames: usize, outputs: &mut [&mut [f32]]) {
        for (v, out) in outputs.iter_mut().enumerate() {
            if out.is_empty() {
                continue;
            }
            let out = &mut out[..frames];
            match self.synths.get_mut(v).and_then(Option::as_mut) {
                Some(synth) => synth.process(out),
                None => out.fill(0.0),
            }
        }
        self.frames += frames as u64;
    }

    // Render voices 0..voices summed into one buffer, using `scratch` (at
    // least out.len() long) for each voice.
    pub fn process_mix(&mut self, out: &mut [f32], voices: usize, scratch: &mut [f32]) {
        out.fill(0.0);
        let scratch = &mut scratch[..out.len()];
        for synth in self.synths.iter_mut().take(voices).flatten() {
            synth.process(scratch);
            for (o, s) in out.iter_mut().zip(scratch.iter()) {
                *o = (f64::from(*o) + f64::from(*s)) as f32;
            }
        }
        self.frames += out.len() as u64;
    }
}

// The audio-thread half of a processor split across threads.
pub struct Player {
    processor: FormantProcessor,
    commands: Receiver<Command>,
    retired: SyncSender<PreparedSchedule>,
    frames: Arc<AtomicU64>,
    scratch: Vec<f32>,
}

// The game-thread half: sends commands and reads the audio clock.
pub struct PlayerHandle {
    commands: SyncSender<Command>,
    retired: Receiver<PreparedSchedule>,
    frames: Arc<AtomicU64>,
    sample_rate: f64,
}

// Mixing renders in chunks of this many samples.
const MIX_CHUNK: usize = 1024;

// Create a processor split across threads. `queue_capacity` bounds the
// commands that can wait between two audio blocks; sends fail when it is full.
pub fn player(sample_rate: f64, opts: ProcessorOptions, queue_capacity: usize) -> Result<(PlayerHandle, Player)> {
    let processor = FormantProcessor::new(sample_rate, &opts)?;
    let (cmd_tx, cmd_rx) = sync_channel(queue_capacity);
    let (old_tx, old_rx) = sync_channel(queue_capacity);
    let frames = Arc::new(AtomicU64::new(0));
    Ok((
        PlayerHandle { commands: cmd_tx, retired: old_rx, frames: frames.clone(), sample_rate },
        Player { processor, commands: cmd_rx, retired: old_tx, frames, scratch: vec![0.0; MIX_CHUNK] },
    ))
}

impl Player {
    pub fn processor(&self) -> &FormantProcessor {
        &self.processor
    }

    fn apply_commands(&mut self) {
        while let Ok(cmd) = self.commands.try_recv() {
            if let Some(old) = self.processor.handle(cmd) {
                // If the return queue is full, freeing here is the fallback.
                if let Err(TrySendError::Full(old) | TrySendError::Disconnected(old)) = self.retired.try_send(old) {
                    drop(old);
                }
            }
        }
    }

    // Apply pending commands, then render one block per voice (see
    // FormantProcessor::process). Does not allocate.
    pub fn process(&mut self, frames: usize, outputs: &mut [&mut [f32]]) {
        self.apply_commands();
        self.processor.process(frames, outputs);
        self.frames.store(self.processor.frames(), Ordering::Release);
    }

    // Apply pending commands, then render voices 0..voices mixed into `out`.
    // Does not allocate.
    pub fn process_mix(&mut self, out: &mut [f32], voices: usize) {
        self.apply_commands();
        for chunk in out.chunks_mut(MIX_CHUNK) {
            self.processor.process_mix(chunk, voices, &mut self.scratch);
        }
        self.frames.store(self.processor.frames(), Ordering::Release);
    }
}

// A full queue hands the (few hundred byte) command back by value, on the
// game thread, so the caller can retry it.
#[allow(clippy::result_large_err)]
impl PlayerHandle {
    pub fn sample_rate(&self) -> f64 {
        self.sample_rate
    }

    // The audio clock as of the last rendered block, in seconds.
    pub fn current_time(&self) -> f64 {
        self.frames.load(Ordering::Acquire) as f64 / self.sample_rate
    }

    // Send a command; it applies at the start of the next audio block. On a
    // full queue the command is handed back.
    pub fn send(&self, cmd: Command) -> std::result::Result<(), Command> {
        self.collect_garbage();
        self.commands.try_send(cmd).map_err(|e| match e {
            TrySendError::Full(c) | TrySendError::Disconnected(c) => c,
        })
    }

    // Speak a compiled schedule on a voice. `start_time` as in Command::Schedule.
    pub fn schedule(
        &self,
        voice: Option<usize>,
        schedule: &[ScheduleEvent],
        start_time: Option<f64>,
    ) -> std::result::Result<(), Command> {
        let schedule = PreparedSchedule::from_schedule(self.sample_rate, schedule);
        self.send(Command::Schedule { voice, schedule, start_time })
    }

    // Speak a compiled schedule on a voice right after its current utterance
    // (see Command::Append). `total_ms` is the new utterance's length, where
    // a later append will start.
    pub fn append(
        &self,
        voice: Option<usize>,
        schedule: &[ScheduleEvent],
        total_ms: f64,
    ) -> std::result::Result<(), Command> {
        let schedule = PreparedSchedule::from_schedule(self.sample_rate, schedule).with_duration_ms(total_ms);
        self.send(Command::Append { voice, schedule })
    }

    // Glide a voice to new parameter values (live control).
    pub fn frame(
        &self,
        voice: Option<usize>,
        target: PartialParams,
        transition_ms: Option<f64>,
    ) -> std::result::Result<(), Command> {
        let transition_ms = transition_ms.unwrap_or(DEFAULT_TRANSITION_MS);
        self.send(Command::Frame { voice, target, transition_ms })
    }

    // Reset one voice, or every voice with `None`.
    pub fn reset(&self, voice: Option<usize>, initial_target: Option<PartialParams>) -> std::result::Result<(), Command> {
        self.send(Command::Reset { voice, initial_target })
    }

    // Free schedules the audio thread has replaced. Sending does this too.
    pub fn collect_garbage(&self) {
        while self.retired.try_recv().is_ok() {}
    }
}

// The phrase being spoken `t_ms` after a schedule started (for lip-sync or
// highlighting): the last phrase starting at or before `t_ms` that has not
// ended. Phrases are in time order, as the compiler emits them.
pub fn phrase_at(phrases: &[Phrase], t_ms: f64) -> Option<usize> {
    let i = phrases.partition_point(|p| p.t_start_ms <= t_ms);
    let i = i.checked_sub(1)?;
    (t_ms < phrases[i].t_end_ms).then_some(i)
}
