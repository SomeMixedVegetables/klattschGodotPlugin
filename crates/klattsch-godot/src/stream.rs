// KlattschStream: the AudioStream resource played by Godot's stock
// AudioStreamPlayer, AudioStreamPlayer2D and AudioStreamPlayer3D nodes, and
// the GDScript API for live control. Every method runs on the main thread and
// only sends commands; none blocks.

use std::sync::atomic::Ordering::Relaxed;
use std::sync::{Arc, Mutex};

use godot::classes::{AudioServer, AudioStream, AudioStreamPlayback, IAudioStream};
use godot::prelude::*;
use klattsch_core::{
    player, player::RESERVED_VOICES, BankRef, Command, CompileOptions, CompileResult, Param, PartialParams,
    PlayerHandle, PreparedSchedule,
};

use crate::alloc_check;
use crate::playback::{
    Clock, KlattschPlaybackExact, KlattschPlaybackResampled, PlaybackCore, SharedState, DROPS_ELSEWHERE, DROPS_ON_MAIN,
};
use crate::utterance::KlattschUtterance;

#[derive(GodotConvert, Var, Export, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[godot(via = i64)]
pub enum RenderMode {
    // Resampled by Godot; honors pitch scale and Doppler.
    #[default]
    Game = 0,
    // Untouched samples at the mix rate; ignores pitch scale and Doppler.
    Exact = 1,
}

// The playback the stream currently controls.
struct Active {
    handle: PlayerHandle,
    shared: Arc<SharedState>,
    rate: f64,
    voices: usize,
}

#[derive(GodotClass)]
#[class(base = AudioStream)]
pub struct KlattschStream {
    // Takes effect on the next play().
    #[export]
    render_mode: RenderMode,
    // Game mode only: the synth's sample rate; 0 means the mix rate. Takes
    // effect on the next play().
    #[export]
    internal_rate: i32,
    // Voices mixed (`[voice=N]` sections, 1-16). Takes effect on the next play().
    #[export]
    voices: i32,
    // Commands that can wait between two audio blocks. Takes effect on the
    // next play().
    #[export]
    command_capacity: i32,
    // The voice KlattschPlayer compiles text with (CompileOptions; these
    // defaults are klattsch's own). Applies to the next say().
    #[export_group(name = "Voice")]
    // A registered bank name (Klattsch.get_bank_names); empty = klatt1980-en.
    #[export]
    bank: GString,
    #[export]
    base_f0: f64,
    // Milliseconds per phoneme.
    #[export]
    rate: f64,
    #[export]
    scale: f64,
    #[export]
    vibrato_depth: f64,
    #[export]
    vibrato_rate: f64,
    #[export]
    tremolo_depth: f64,
    #[export]
    tremolo_rate: f64,
    #[export]
    aspiration: f64,
    #[export]
    tilt: f64,
    #[export]
    effort: f64,
    // instantiate_playback takes &self, hence the Mutex (only the main
    // thread locks it).
    current: Mutex<Option<Active>>,
    base: Base<AudioStream>,
}

#[godot_api]
impl IAudioStream for KlattschStream {
    fn init(base: Base<AudioStream>) -> Self {
        // One stream per player: a scene instanced twice gets two streams,
        // so two characters never share a voice.
        base.to_init_gd().set_local_to_scene(true);
        Self {
            render_mode: RenderMode::Game,
            internal_rate: 0,
            voices: 4,
            command_capacity: 256,
            bank: GString::new(),
            base_f0: 120.0,
            rate: 110.0,
            scale: 1.0,
            vibrato_depth: 0.0,
            vibrato_rate: 5.0,
            tremolo_depth: 0.0,
            tremolo_rate: 5.0,
            aspiration: 0.0,
            tilt: 0.0,
            effort: 0.5,
            current: Mutex::new(None),
            base,
        }
    }

    fn instantiate_playback(&self) -> Option<Gd<AudioStreamPlayback>> {
        let mix_rate = f64::from(AudioServer::singleton().get_mix_rate());
        let rate = match self.render_mode {
            RenderMode::Game if self.internal_rate > 0 => f64::from(self.internal_rate),
            _ => mix_rate,
        };
        let voices = self.voices.clamp(1, RESERVED_VOICES as i32) as usize;
        let capacity = self.command_capacity.max(1) as usize;
        let (handle, player) = match player(rate, Default::default(), capacity) {
            Ok(pair) => pair,
            Err(e) => {
                godot_error!("KlattschStream: {e}");
                return None;
            }
        };
        let shared = Arc::new(SharedState::new());
        let core = PlaybackCore::new(shared.clone(), player, rate, voices);
        let playback: Gd<AudioStreamPlayback> = match self.render_mode {
            RenderMode::Exact => KlattschPlaybackExact::create(core).upcast(),
            RenderMode::Game => KlattschPlaybackResampled::create(core, rate as f32).upcast(),
        };
        // Replacing the old handle disconnects the old playback, which keeps
        // rendering its last state until Godot drops it.
        *self.current.lock().unwrap() = Some(Active { handle, shared, rate, voices });
        Some(playback)
    }

    fn get_length(&self) -> f64 {
        0.0
    }

    fn is_monophonic(&self) -> bool {
        true
    }
}

fn voice_arg(voice: i64) -> Option<usize> {
    usize::try_from(voice).ok()
}

// The current playback's clock (see KlattschPlayer).
#[derive(Clone, Copy, Debug)]
pub struct StreamClock {
    pub clock: Clock,
    pub rate: f64,
    pub playing: bool,
    pub dead: bool,
}

impl KlattschStream {
    // The Voice properties as compile options.
    pub fn compile_options(&self) -> CompileOptions {
        CompileOptions {
            base_f0: Some(self.base_f0),
            rate: Some(self.rate),
            scale: Some(self.scale),
            vibrato_depth: Some(self.vibrato_depth),
            vibrato_rate: Some(self.vibrato_rate),
            tremolo_depth: Some(self.tremolo_depth),
            tremolo_rate: Some(self.tremolo_rate),
            aspiration: Some(self.aspiration),
            tilt: Some(self.tilt),
            effort: Some(self.effort),
            bank: (!self.bank.is_empty()).then(|| BankRef::Name(self.bank.to_string())),
            ..CompileOptions::default()
        }
    }

    pub fn clock(&self) -> Option<StreamClock> {
        let guard = self.current.lock().unwrap();
        let a = guard.as_ref()?;
        Some(StreamClock {
            clock: a.shared.clock(),
            rate: a.rate,
            playing: a.shared.playing.load(Relaxed),
            dead: a.shared.dead.load(Relaxed),
        })
    }

    // speak() for Rust callers, with a tag the clock reports while the
    // utterance plays (0 = untagged).
    pub fn speak_result(&self, result: &CompileResult, interrupt: bool, start_at: f64, tag: u64) -> bool {
        let (rate, voices) = match self.current.lock().unwrap().as_ref() {
            Some(a) => (a.rate, a.voices),
            None => return false,
        };
        if result.voices.len() > voices {
            godot_error!("KlattschStream.speak: the utterance has {} voices; the stream mixes {voices}", result.voices.len());
            return false;
        }
        let start_time = (start_at >= 0.0).then_some(start_at);
        let mut commands = Vec::new();
        for (v, section) in result.voices.iter().enumerate() {
            // All sections share the utterance length so they stay aligned
            // when appended.
            let schedule = PreparedSchedule::from_schedule(rate, &section.schedule)
                .with_duration_ms(result.total_ms)
                .with_tag(tag);
            commands.push(if interrupt {
                Command::Schedule { voice: Some(v), schedule, start_time }
            } else {
                Command::Append { voice: Some(v), schedule }
            });
        }
        if interrupt {
            for v in result.voices.len()..voices {
                commands.push(Command::Schedule { voice: Some(v), schedule: PreparedSchedule::default(), start_time: None });
                commands.push(Command::Frame { voice: Some(v), target: mute(), transition_ms: 30.0 });
            }
        }
        self.send(commands)
    }

    // Send commands to the current playback. False if there is none or the
    // queue filled up partway.
    fn send(&self, commands: impl IntoIterator<Item = Command>) -> bool {
        let guard = self.current.lock().unwrap();
        let Some(active) = guard.as_ref() else { return false };
        let ok = commands.into_iter().all(|c| active.handle.send(c).is_ok());
        active.shared.command_sent();
        ok
    }

    fn voice_count(&self) -> usize {
        self.current.lock().unwrap().as_ref().map_or(0, |a| a.voices)
    }

    // The voices a `voice` argument names: -1 means all of them.
    fn targets(&self, voice: i64) -> Vec<usize> {
        match voice_arg(voice) {
            Some(v) => vec![v],
            None => (0..self.voice_count()).collect(),
        }
    }

    pub fn frames(&self, target: PartialParams, glide_ms: f64, voice: i64) -> bool {
        let commands: Vec<Command> = self
            .targets(voice)
            .into_iter()
            .map(|v| Command::Frame { voice: Some(v), target, transition_ms: glide_ms })
            .collect();
        self.send(commands)
    }
}

// Fades a voice out: all three formant amplitudes to zero.
fn mute() -> PartialParams {
    PartialParams::new().with(Param::A1, 0.0).with(Param::A2, 0.0).with(Param::A3, 0.0)
}

#[godot_api]
impl KlattschStream {
    #[constant]
    const GAME: i64 = 0;
    #[constant]
    const EXACT: i64 = 1;

    // Speak an utterance, each `[voice=N]` section on voice N.
    // interrupt: replace what is being said (other voices fall silent);
    // otherwise start right after the current utterance ends.
    // start_at: with interrupt, a time on the audio clock (get_audio_time)
    // for the utterance's start; -1 means now. Ignored when appending.
    // Returns false if no playback is active, a voice is out of range, or the
    // command queue is full.
    #[func]
    fn speak(
        &self,
        utterance: Gd<KlattschUtterance>,
        #[opt(default = true)] interrupt: bool,
        #[opt(default = -1.0)] start_at: f64,
    ) -> bool {
        self.speak_result(&utterance.bind().result, interrupt, start_at, 0)
    }

    // Glide one of the 19 synth parameters (F0, voicing, F1, BW1, A1, F2,
    // BW2, A2, F3, BW3, A3, gain, vibratoDepth, vibratoRate, tremoloDepth,
    // tremoloRate, aspiration, tilt, effort) on one voice, or all (-1).
    #[func]
    pub fn set_param(
        &self,
        param: StringName,
        value: f64,
        #[opt(default = 50.0)] glide_ms: f64,
        #[opt(default = -1)] voice: i64,
    ) -> bool {
        let Some(p) = Param::from_name(&param.to_string()) else {
            godot_error!("KlattschStream.set_param: unknown parameter {param}");
            return false;
        };
        self.frames(PartialParams::new().with(p, value), glide_ms, voice)
    }

    // Glide several parameters at once: {"F0": 180.0, "F1": 700.0}.
    #[func]
    fn set_params(&self, values: VarDictionary, #[opt(default = 50.0)] glide_ms: f64, #[opt(default = -1)] voice: i64) -> bool {
        let mut target = PartialParams::new();
        for (key, value) in values.iter_shared() {
            let (Some(p), Ok(v)) = (Param::from_name(&key.to_string()), value.try_to::<f64>()) else {
                godot_error!("KlattschStream.set_params: bad entry {key}: {value}");
                return false;
            };
            target.set(p, Some(v));
        }
        self.frames(target, glide_ms, voice)
    }

    // Reset a voice, or all (-1), to the synth's defaults: silent, schedule
    // and anything appended dropped, filters cleared.
    #[func]
    fn reset(&self, #[opt(default = -1)] voice: i64) -> bool {
        self.send([Command::Reset { voice: voice_arg(voice), initial_target: None }])
    }

    // Stop speaking (one voice or all) with a short fade, keeping the stream
    // and the voice's other settings.
    #[func]
    pub fn silence(&self, #[opt(default = -1)] voice: i64) -> bool {
        let commands: Vec<Command> = self
            .targets(voice)
            .into_iter()
            .flat_map(|v| {
                [
                    Command::Schedule { voice: Some(v), schedule: PreparedSchedule::default(), start_time: None },
                    Command::Frame { voice: Some(v), target: mute(), transition_ms: 30.0 },
                ]
            })
            .collect();
        self.send(commands)
    }

    // The audio clock: seconds rendered by the current playback, at the
    // synth's rate.
    #[func]
    fn get_audio_time(&self) -> f64 {
        self.current.lock().unwrap().as_ref().map_or(0.0, |a| a.handle.current_time())
    }

    #[func]
    fn is_active(&self) -> bool {
        self.current.lock().unwrap().as_ref().is_some_and(|a| a.shared.playing.load(Relaxed))
    }

    // True if mix panicked; the playback then stays silent.
    #[func]
    fn is_dead(&self) -> bool {
        self.current.lock().unwrap().as_ref().is_some_and(|a| a.shared.dead.load(Relaxed))
    }

    // Diagnostics for the current playback (used by the test project).
    #[func]
    fn get_stats(&self) -> VarDictionary {
        let mut d = vdict! {
            "drops_on_main" => DROPS_ON_MAIN.load(Relaxed) as i64,
            "drops_elsewhere" => DROPS_ELSEWHERE.load(Relaxed) as i64,
            "mix_allocations" => alloc_check::mix_allocations(),
        };
        if let Some(a) = self.current.lock().unwrap().as_ref() {
            let s = &a.shared;
            d.set("rate", a.rate);
            d.set("mix_calls", s.mix_calls.load(Relaxed) as i64);
            d.set("frames_rendered", s.frames_rendered.load(Relaxed) as i64);
            d.set("max_frames", s.max_frames.load(Relaxed) as i64);
            d.set("max_gap_ms", s.max_gap_ns.load(Relaxed) as f64 / 1e6);
            d.set("max_mix_us", s.max_mix_ns.load(Relaxed) as f64 / 1e3);
            d.set("max_command_ms", s.max_command_ns.load(Relaxed) as f64 / 1e6);
            d.set("dead", s.dead.load(Relaxed));
        }
        d
    }
}
