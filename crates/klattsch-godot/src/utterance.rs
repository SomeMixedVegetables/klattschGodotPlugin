// KlattschUtterance: a compiled phoneme string (see Klattsch.compile).

use godot::classes::AudioServer;
use godot::prelude::*;
use klattsch_core::{render_mix, CompileResult};

// A compiled utterance: one schedule per `[voice=N]` section, plus timing.
#[derive(GodotClass)]
#[class(tool, base = RefCounted, no_init)]
pub struct KlattschUtterance {
    pub result: CompileResult,
    base: Base<RefCounted>,
}

impl KlattschUtterance {
    pub fn create(result: CompileResult) -> Gd<Self> {
        Gd::from_init_fn(|base| Self { result, base })
    }
}

#[godot_api]
impl KlattschUtterance {
    #[func]
    fn get_phonemes(&self) -> GString {
        self.result.source.as_str().into()
    }

    #[func]
    fn get_total_ms(&self) -> f64 {
        self.result.total_ms
    }

    #[func]
    fn get_voice_count(&self) -> i64 {
        self.result.voices.len() as i64
    }

    #[func]
    fn get_warnings(&self) -> PackedStringArray {
        self.result.warnings.iter().map(|w| GString::from(w.as_str())).collect()
    }

    // The whole utterance rendered offline, all voices mixed (render_mix):
    // `sample_rate` 0 means the audio server's mix rate. This is exactly what
    // an exact-mode KlattschStream plays for it, from a fresh playback.
    #[func]
    fn render(&self, #[opt(default = 0.0)] sample_rate: f64) -> PackedFloat32Array {
        let rate = if sample_rate > 0.0 { sample_rate } else { f64::from(AudioServer::singleton().get_mix_rate()) };
        match render_mix(&self.result, rate) {
            Ok(samples) => PackedFloat32Array::from(samples),
            Err(e) => {
                godot_error!("KlattschUtterance.render: {e}");
                PackedFloat32Array::new()
            }
        }
    }

    // Every voice's phrases: src_start, src_end, t_start_ms, t_end_ms, kind,
    // phoneme (empty for pauses) and voice.
    #[func]
    fn get_phrases(&self) -> Array<VarDictionary> {
        let mut out = Array::new();
        for (v, voice) in self.result.voices.iter().enumerate() {
            for p in &voice.phrases {
                out.push(&vdict! {
                    "src_start" => p.src_start as i64,
                    "src_end" => p.src_end as i64,
                    "t_start_ms" => p.t_start_ms,
                    "t_end_ms" => p.t_end_ms,
                    "kind" => p.kind,
                    "phoneme" => p.phoneme.as_deref().unwrap_or(""),
                    "voice" => v as i64,
                });
            }
        }
        out
    }
}
