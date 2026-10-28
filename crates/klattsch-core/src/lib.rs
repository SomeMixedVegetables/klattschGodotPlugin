// Public surface of the klattsch synthesis engine (Rust port).
//
// Typical usage:
//
//   use klattsch_core::*;
//   let sr = 48000.0;
//   let compiled = compile_string("HH AH L OW", &CompileOptions::default())?;
//   let buf = render_to_buffer(sr, compiled.schedule(), Some(compiled.total_ms), None)?;
//   let wav = encode_wav(&buf, 48000, &WavOptions::default());
//   // ...write wav.bytes to a .wav file
//
//   dsp        - low-level biquad/glottal/noise/softclip
//   synth      - FormantSynth
//   player     - multi-voice real-time playback (the AudioWorklet's logic)
//   banks      - phoneme bank registry (bundled + runtime-loaded banks)
//   sequencer  - text-to-schedule compiler
//   wav        - WAV encoder
//   kana       - Japanese kana/romaji to phonemes
//   pronounce  - English words to ARPABET via the CMU dictionary
//   english    - English text to a phoneme string (pauses, numbers, melody)
//   lts        - NRL letter-to-sound rules for words not in the dictionary
//   highlight  - syntax-highlight lexer for editors

pub mod banks;
pub mod dsp;
pub mod english;
pub mod error;
mod fdlibm;
pub mod highlight;
#[doc(hidden)]
pub mod js;
pub mod kana;
pub mod lts;
pub mod player;
pub mod pronounce;
pub mod sequencer;
pub mod synth;
pub mod value;
pub mod wav;

pub use banks::{
    banks, phoneme_keys, phonemes, register_bank, resolve_bank, BankRef, BankRegistry, ResolvedBank,
    DEFAULT_BANK,
};
pub use dsp::{glottal_pulse, soft_clip, xorshift, BandpassBiquad};
pub use english::{english_to_phonemes, EnglishOptions, EnglishResult, EnglishWord, WordSource};
pub use error::{Error, Result};
pub use highlight::{build_highlight, registry_phonemes_for, HighlightOptions, HighlightSpan};
pub use kana::{is_kana, japanese_to_phonemes, kana_to_phonemes, romaji_to_phonemes};
pub use player::{phrase_at, player, Command, FormantProcessor, Player, PlayerHandle, ProcessorOptions};
pub use pronounce::{CmuDict, Lexicon, Pronounce, WordPronunciation};
pub use sequencer::{
    compile, compile_string, compile_string_with_registry, compile_with_registry, tokenize,
    CompileOptions, CompileResult, Phrase, ScheduleEvent, Token, TokenKind, Tokenized, VoiceResult,
};
pub use synth::{
    render_mix, render_to_buffer, FormantSynth, Param, Params, PartialParams, PreparedSchedule, SynthEvent, DEFAULT,
    PARAMS,
};
pub use value::{Map, Value};
pub use wav::{encode_wav, EncodedWav, WavMetadata, WavOptions, SOFTWARE};

// A phoneme code with its stress flag, as produced by the kana and
// pronunciation helpers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phone {
    pub code: String,
    pub stressed: bool,
}

impl Phone {
    pub fn new(code: &str) -> Self {
        Self { code: code.into(), stressed: false }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "code": self.code, "stressed": self.stressed })
    }
}
