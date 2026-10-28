// KlattschSample: a pre-rendered utterance (Klattsch.render) as an
// AudioStream for any stock player. Samples are mono f32 shared between the
// resource and its playbacks; mix only copies them (no allocation, no
// Godot calls), like the live stream.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::Arc;

use godot::classes::native::AudioFrame;
use godot::classes::{
    AudioServer, AudioStream, AudioStreamPlayback, AudioStreamPlaybackResampled, AudioStreamWav, IAudioStream,
    IAudioStreamPlayback, IAudioStreamPlaybackResampled,
};
use godot::classes::audio_stream_wav::Format;
use godot::meta::conv::RawPtr;
use godot::prelude::*;
use klattsch_core::{encode_wav, WavOptions};

use crate::stream::RenderMode;

// Position and state shared by a sample playback's Godot calls.
struct SampleCore {
    samples: Arc<[f32]>,
    rate: f64,
    position: AtomicU64, // next sample to play
    playing: AtomicBool,
}

impl SampleCore {
    // Safety: `buffer` must point to `frames` writable AudioFrames.
    unsafe fn mix(&self, buffer: *mut AudioFrame, frames: i32) -> i32 {
        let Ok(n) = usize::try_from(frames) else { return 0 };
        if buffer.is_null() || n == 0 {
            return frames;
        }
        // SAFETY: Godot passes a buffer of `frames` frames.
        let out = unsafe { std::slice::from_raw_parts_mut(buffer, n) };
        let silence = AudioFrame { left: 0.0, right: 0.0 };
        if !self.playing.load(Relaxed) {
            out.fill(silence);
            return frames;
        }
        let pos = (self.position.load(Relaxed) as usize).min(self.samples.len());
        let take = n.min(self.samples.len() - pos);
        for (frame, &s) in out.iter_mut().zip(&self.samples[pos..pos + take]) {
            *frame = AudioFrame { left: s, right: s };
        }
        out[take..].fill(silence);
        self.position.store((pos + take) as u64, Relaxed);
        if take < n {
            // The end. Godot ends a stream (and the player emits `finished`)
            // when mix returns fewer frames than it asked for.
            self.playing.store(false, Relaxed);
            return take as i32;
        }
        frames
    }

    fn start(&self, from_pos: f64) {
        self.seek(from_pos);
        self.playing.store(true, Relaxed);
    }

    fn seek(&self, position: f64) {
        let p = (position.max(0.0) * self.rate).round() as u64;
        self.position.store(p.min(self.samples.len() as u64), Relaxed);
    }

    fn position(&self) -> f64 {
        self.position.load(Relaxed) as f64 / self.rate
    }
}

#[derive(GodotClass)]
#[class(tool, base = AudioStreamPlayback, no_init)]
pub struct KlattschSamplePlaybackExact {
    core: SampleCore,
    base: Base<AudioStreamPlayback>,
}

#[godot_api]
impl IAudioStreamPlayback for KlattschSamplePlaybackExact {
    unsafe fn mix_rawptr(&mut self, buffer: RawPtr<*mut AudioFrame>, _rate_scale: f32, frames: i32) -> i32 {
        // SAFETY: Godot's buffer holds `frames` frames.
        unsafe { self.core.mix(buffer.ptr(), frames) }
    }

    fn start(&mut self, from_pos: f64) {
        self.core.start(from_pos);
    }

    fn stop(&mut self) {
        self.core.playing.store(false, Relaxed);
    }

    fn is_playing(&self) -> bool {
        self.core.playing.load(Relaxed)
    }

    fn get_playback_position(&self) -> f64 {
        self.core.position()
    }

    fn get_loop_count(&self) -> i32 {
        0
    }

    fn seek(&mut self, position: f64) {
        self.core.seek(position);
    }
}

#[derive(GodotClass)]
#[class(tool, base = AudioStreamPlaybackResampled, no_init)]
pub struct KlattschSamplePlaybackResampled {
    core: SampleCore,
    rate: f32,
    base: Base<AudioStreamPlaybackResampled>,
}

#[godot_api]
impl IAudioStreamPlaybackResampled for KlattschSamplePlaybackResampled {
    unsafe fn mix_resampled_rawptr(&mut self, dst_buffer: RawPtr<*mut AudioFrame>, frame_count: i32) -> i32 {
        // SAFETY: Godot's buffer holds `frame_count` frames.
        unsafe { self.core.mix(dst_buffer.ptr(), frame_count) }
    }

    fn get_stream_sampling_rate(&self) -> f32 {
        self.rate
    }

    fn start(&mut self, from_pos: f64) {
        self.base_mut().begin_resample();
        self.core.start(from_pos);
    }

    fn stop(&mut self) {
        self.core.playing.store(false, Relaxed);
    }

    fn is_playing(&self) -> bool {
        self.core.playing.load(Relaxed)
    }

    fn get_playback_position(&self) -> f64 {
        self.core.position()
    }

    fn get_loop_count(&self) -> i32 {
        0
    }

    fn seek(&mut self, position: f64) {
        self.core.seek(position);
    }
}

// A pre-rendered utterance. Make one with Klattsch.render.
#[derive(GodotClass)]
#[class(tool, base = AudioStream)]
pub struct KlattschSample {
    // Game: resampled by Godot to the mix rate, honoring pitch scale and
    // Doppler. Exact: the samples untouched, ignoring them; only possible
    // when the sample rate equals the mix rate (otherwise it plays in game
    // mode, with a warning).
    #[export]
    render_mode: RenderMode,
    samples: Arc<[f32]>,
    rate: f64,
    base: Base<AudioStream>,
}

#[godot_api]
impl IAudioStream for KlattschSample {
    fn init(base: Base<AudioStream>) -> Self {
        Self { render_mode: RenderMode::Game, samples: Arc::from([]), rate: 48000.0, base }
    }

    fn instantiate_playback(&self) -> Option<Gd<AudioStreamPlayback>> {
        let core = SampleCore {
            samples: self.samples.clone(),
            rate: self.rate,
            position: AtomicU64::new(0),
            playing: AtomicBool::new(false),
        };
        let mix_rate = f64::from(AudioServer::singleton().get_mix_rate());
        let exact = self.render_mode == RenderMode::Exact;
        if exact && self.rate != mix_rate {
            godot_warn!(
                "KlattschSample: rendered at {} Hz but mixing at {mix_rate} Hz, so it cannot play exactly; resampling",
                self.rate
            );
        }
        Some(if exact && self.rate == mix_rate {
            Gd::from_init_fn(|base| KlattschSamplePlaybackExact { core, base }).upcast()
        } else {
            let rate = self.rate as f32;
            Gd::from_init_fn(|base| KlattschSamplePlaybackResampled { core, rate, base }).upcast()
        })
    }

    fn get_length(&self) -> f64 {
        self.samples.len() as f64 / self.rate
    }

    fn is_monophonic(&self) -> bool {
        false
    }
}

impl KlattschSample {
    pub fn create(samples: Vec<f32>, rate: f64) -> Gd<Self> {
        let mut s = Self::new_gd();
        {
            let mut b = s.bind_mut();
            b.samples = Arc::from(samples);
            b.rate = rate;
        }
        s
    }
}

#[godot_api]
impl KlattschSample {
    #[constant]
    const GAME: i64 = 0;
    #[constant]
    const EXACT: i64 = 1;

    #[func]
    fn get_sample_rate(&self) -> f64 {
        self.rate
    }

    // The mono samples.
    #[func]
    fn get_samples(&self) -> PackedFloat32Array {
        PackedFloat32Array::from(&self.samples[..])
    }

    // A stock AudioStreamWAV with the same sound, quantized to 16 bits (no
    // normalization), for code that needs one.
    #[func]
    fn to_audio_stream_wav(&self) -> Gd<AudioStreamWav> {
        let wav = encode_wav(&self.samples, self.rate as u32, &WavOptions { peak_normalize: None, metadata: None });
        let mut stream = AudioStreamWav::new_gd();
        stream.set_format(Format::FORMAT_16_BITS);
        stream.set_mix_rate(self.rate as i32);
        stream.set_stereo(false);
        stream.set_data(&PackedByteArray::from(&wav.bytes[44..]));
        stream
    }
}
