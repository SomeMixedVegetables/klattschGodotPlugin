// The playback objects Godot drives on its audio thread, and the plain-Rust
// core they share.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{fence, AtomicBool, AtomicU64, Ordering, Ordering::Relaxed};
use std::sync::Arc;
use std::time::Instant;

use godot::classes::native::AudioFrame;
use godot::classes::{AudioStreamPlayback, AudioStreamPlaybackResampled, IAudioStreamPlayback, IAudioStreamPlaybackResampled};
use godot::meta::conv::RawPtr;
use godot::prelude::*;
use klattsch_core::Player;

use crate::alloc_check;

// Playbacks dropped on the main thread and elsewhere (the spec's open risk:
// freeing a playback on the audio thread would free memory there).
pub static DROPS_ON_MAIN: AtomicU64 = AtomicU64::new(0);
pub static DROPS_ELSEWHERE: AtomicU64 = AtomicU64::new(0);

// Mono samples rendered per chunk; Godot asks for 512 frames per mix on
// Windows at 48 kHz, so one chunk is the norm.
const SCRATCH_FRAMES: usize = 4096;

// The only state the main thread and the audio thread share, besides the
// PlayerHandle's command channel.
pub struct SharedState {
    pub frames_rendered: AtomicU64,
    pub playing: AtomicBool,
    pub dead: AtomicBool,
    // Diagnostics, written by mix (and `command_sent` by the stream).
    pub mix_calls: AtomicU64,
    pub max_frames: AtomicU64,
    pub max_gap_ns: AtomicU64, // longest time between the starts of two mix calls
    pub max_mix_ns: AtomicU64, // longest time spent inside our mix body
    // Time from sending a command to the mix call that applies it.
    pub max_command_ns: AtomicU64,
    last_command_ns: AtomicU64,
    applied_command_ns: AtomicU64,
    last_mix_ns: AtomicU64,
    epoch: Instant,
    // Voice 0's utterance clock, published after every block as one
    // consistent snapshot (a sequence lock: odd while being written).
    clock_seq: AtomicU64,
    clock_frames: AtomicU64,
    clock_tag: AtomicU64,
    clock_start: AtomicU64, // f64 bits: the frame the tagged utterance's time zero fell on
    clock_speaking: AtomicBool,
}

// A consistent view of the audio clock and the utterance playing on voice 0.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    pub frames: u64,
    pub tag: u64,
    pub start: f64,
    pub speaking: bool,
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            frames_rendered: AtomicU64::new(0),
            playing: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            mix_calls: AtomicU64::new(0),
            max_frames: AtomicU64::new(0),
            max_gap_ns: AtomicU64::new(0),
            max_mix_ns: AtomicU64::new(0),
            max_command_ns: AtomicU64::new(0),
            last_command_ns: AtomicU64::new(0),
            applied_command_ns: AtomicU64::new(0),
            last_mix_ns: AtomicU64::new(0),
            epoch: Instant::now(),
            clock_seq: AtomicU64::new(0),
            clock_frames: AtomicU64::new(0),
            clock_tag: AtomicU64::new(0),
            clock_start: AtomicU64::new(0),
            clock_speaking: AtomicBool::new(false),
        }
    }

    // Audio thread only.
    fn publish_clock(&self, c: Clock) {
        let seq = self.clock_seq.load(Relaxed);
        self.clock_seq.store(seq + 1, Relaxed);
        fence(Ordering::Release);
        self.clock_frames.store(c.frames, Relaxed);
        self.clock_tag.store(c.tag, Relaxed);
        self.clock_start.store(c.start.to_bits(), Relaxed);
        self.clock_speaking.store(c.speaking, Relaxed);
        self.clock_seq.store(seq + 2, Ordering::Release);
    }

    pub fn clock(&self) -> Clock {
        loop {
            let before = self.clock_seq.load(Ordering::Acquire);
            if before % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let c = Clock {
                frames: self.clock_frames.load(Relaxed),
                tag: self.clock_tag.load(Relaxed),
                start: f64::from_bits(self.clock_start.load(Relaxed)),
                speaking: self.clock_speaking.load(Relaxed),
            };
            fence(Ordering::Acquire);
            if self.clock_seq.load(Relaxed) == before {
                return c;
            }
        }
    }

    fn now_ns(&self) -> u64 {
        self.epoch.elapsed().as_nanos() as u64
    }

    // Called on the main thread after sending commands.
    pub fn command_sent(&self) {
        self.last_command_ns.store(self.now_ns(), Relaxed);
    }
}

// A klattsch Player rendering mono and writing it to both channels.
pub struct PlaybackCore {
    shared: Arc<SharedState>,
    player: Player,
    rate: f64,
    voices: usize,
    scratch: Vec<f32>,
}

impl PlaybackCore {
    pub fn new(shared: Arc<SharedState>, player: Player, rate: f64, voices: usize) -> Self {
        Self { shared, player, rate, voices, scratch: vec![0.0; SCRATCH_FRAMES] }
    }

    // Fill `frames` stereo frames at `buffer`. Never panics out, allocates
    // or calls Godot.
    //
    // Safety: `buffer` must point to `frames` writable AudioFrames.
    unsafe fn mix(&mut self, buffer: *mut AudioFrame, frames: i32) -> i32 {
        let Ok(n) = usize::try_from(frames) else { return 0 };
        if buffer.is_null() || n == 0 {
            return frames;
        }
        // SAFETY: Godot passes a buffer of `frames` frames.
        let out = unsafe { std::slice::from_raw_parts_mut(buffer, n) };
        alloc_check::enter_mix();
        let start = Instant::now();
        let shared = self.shared.clone(); // a refcount bump, no allocation
        let now = shared.now_ns();
        let last = shared.last_mix_ns.swap(now, Relaxed);
        if last != 0 {
            shared.max_gap_ns.fetch_max(now - last, Relaxed);
        }
        // Commands sent since the last mix are applied by this one.
        let sent = shared.last_command_ns.load(Relaxed);
        if sent > shared.applied_command_ns.swap(sent, Relaxed) {
            shared.max_command_ns.fetch_max(now.saturating_sub(sent), Relaxed);
        }
        shared.mix_calls.fetch_add(1, Relaxed);
        shared.max_frames.fetch_max(n as u64, Relaxed);

        if shared.dead.load(Relaxed) || !shared.playing.load(Relaxed) {
            out.fill(AudioFrame { left: 0.0, right: 0.0 });
        } else if catch_unwind(AssertUnwindSafe(|| self.render(out))).is_err() {
            shared.dead.store(true, Relaxed);
            out.fill(AudioFrame { left: 0.0, right: 0.0 });
        }
        shared.frames_rendered.fetch_add(n as u64, Relaxed);
        shared.max_mix_ns.fetch_max(start.elapsed().as_nanos() as u64, Relaxed);
        alloc_check::leave_mix();
        frames
    }

    fn render(&mut self, out: &mut [AudioFrame]) {
        for chunk in out.chunks_mut(SCRATCH_FRAMES) {
            let mono = &mut self.scratch[..chunk.len()];
            self.player.process_mix(mono, self.voices);
            for (frame, &s) in chunk.iter_mut().zip(mono.iter()) {
                *frame = AudioFrame { left: s, right: s };
            }
        }
        let p = self.player.processor();
        let frames = p.frames();
        let clock = match p.voice(0) {
            Some(v) => Clock {
                frames,
                tag: v.utterance_tag(),
                start: frames as f64 - v.sample_counter(),
                speaking: v.speaking(),
            },
            None => Clock { frames, tag: 0, start: 0.0, speaking: false },
        };
        self.shared.publish_clock(clock);
    }

    fn start(&self) {
        self.shared.playing.store(true, Relaxed);
    }

    fn stop(&self) {
        self.shared.playing.store(false, Relaxed);
    }

    fn is_playing(&self) -> bool {
        self.shared.playing.load(Relaxed)
    }

    fn position(&self) -> f64 {
        self.shared.frames_rendered.load(Relaxed) as f64 / self.rate
    }
}

impl Drop for PlaybackCore {
    fn drop(&mut self) {
        if crate::on_main_thread() { &DROPS_ON_MAIN } else { &DROPS_ELSEWHERE }.fetch_add(1, Relaxed);
    }
}

// Exact mode: samples at the mix rate, untouched; pitch scale and Doppler
// (rate_scale) are ignored.
#[derive(GodotClass)]
#[class(base = AudioStreamPlayback, no_init)]
pub struct KlattschPlaybackExact {
    core: PlaybackCore,
    base: Base<AudioStreamPlayback>,
}

impl KlattschPlaybackExact {
    pub fn create(core: PlaybackCore) -> Gd<Self> {
        Gd::from_init_fn(|base| Self { core, base })
    }
}

#[godot_api]
impl IAudioStreamPlayback for KlattschPlaybackExact {
    unsafe fn mix_rawptr(&mut self, buffer: RawPtr<*mut AudioFrame>, _rate_scale: f32, frames: i32) -> i32 {
        // SAFETY: Godot's buffer holds `frames` frames.
        unsafe { self.core.mix(buffer.ptr(), frames) }
    }

    fn start(&mut self, _from_pos: f64) {
        self.core.start();
    }

    fn stop(&mut self) {
        self.core.stop();
    }

    fn is_playing(&self) -> bool {
        self.core.is_playing()
    }

    fn get_playback_position(&self) -> f64 {
        self.core.position()
    }

    fn get_loop_count(&self) -> i32 {
        0
    }

    fn seek(&mut self, _position: f64) {}
}

// Game mode: samples at the stream's own rate, resampled by Godot, which
// also applies pitch scale and Doppler.
#[derive(GodotClass)]
#[class(base = AudioStreamPlaybackResampled, no_init)]
pub struct KlattschPlaybackResampled {
    core: PlaybackCore,
    // Read by Godot's resampler on the audio thread: a plain field.
    rate: f32,
    base: Base<AudioStreamPlaybackResampled>,
}

impl KlattschPlaybackResampled {
    pub fn create(core: PlaybackCore, rate: f32) -> Gd<Self> {
        Gd::from_init_fn(|base| Self { core, rate, base })
    }
}

#[godot_api]
impl IAudioStreamPlaybackResampled for KlattschPlaybackResampled {
    unsafe fn mix_resampled_rawptr(&mut self, dst_buffer: RawPtr<*mut AudioFrame>, frame_count: i32) -> i32 {
        // SAFETY: Godot's buffer holds `frame_count` frames.
        unsafe { self.core.mix(dst_buffer.ptr(), frame_count) }
    }

    fn get_stream_sampling_rate(&self) -> f32 {
        self.rate
    }

    fn start(&mut self, _from_pos: f64) {
        // Built-in resampled playbacks reset the resampler on start.
        self.base_mut().begin_resample();
        self.core.start();
    }

    fn stop(&mut self) {
        self.core.stop();
    }

    fn is_playing(&self) -> bool {
        self.core.is_playing()
    }

    fn get_playback_position(&self) -> f64 {
        self.core.position()
    }

    fn get_loop_count(&self) -> i32 {
        0
    }

    fn seek(&mut self, _position: f64) {}
}
