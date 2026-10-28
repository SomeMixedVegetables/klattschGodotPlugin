// The Klattsch engine singleton: text to phonemes (English, Japanese), the
// CMU dictionary (loaded in the background), the runtime lexicon, voice
// banks, and compiling phoneme strings into KlattschUtterances.
//
// Language state lives in a process-wide Mutex so KlattschPlayer can use it
// too; only the main thread touches Godot. The dictionary is read with
// FileAccess (which sees res:// inside an exported .pck) on the main thread
// and parsed on a plain Rust thread.

use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use godot::classes::file_access::ModeFlags;
use godot::classes::{AudioServer, Engine, FileAccess, Object, SceneTree};
use godot::global::Error as GodotError;
use godot::prelude::*;
use klattsch_core::{
    banks, encode_wav, english_to_phonemes, japanese_to_phonemes, render_mix, BankRef, CmuDict, CompileOptions,
    EnglishOptions, EnglishResult, Lexicon, Phone, WavMetadata, WavOptions, WordSource, SOFTWARE,
};

use crate::sample::KlattschSample;
use crate::utterance::KlattschUtterance;

pub const DEFAULT_DICTIONARY: &str = "res://addons/klattsch/cmudict.dict";

#[derive(Default)]
struct Lang {
    // Set once by the parse thread.
    dictionary: Arc<OnceLock<Arc<CmuDict>>>,
    loading: bool,
    // add_word entries, checked first.
    words: Arc<CmuDict>,
    // load_word_list files, earlier first, checked before the CMU dictionary.
    lists: Vec<Arc<CmuDict>>,
}

static LANG: LazyLock<Mutex<Lang>> = LazyLock::new(Default::default);

// The lexicon to pronounce English with, and whether the CMU dictionary is in it.
pub fn lexicon() -> (Lexicon, bool) {
    let lang = LANG.lock().unwrap();
    let mut lex = Lexicon::new();
    lex.push_layer(lang.words.clone());
    for list in &lang.lists {
        lex.push_layer(list.clone());
    }
    let cmu = lang.dictionary.get().cloned();
    let ready = cmu.is_some();
    if let Some(cmu) = cmu {
        lex.push_layer(cmu);
    }
    (lex, ready)
}

pub fn dictionary_ready() -> bool {
    LANG.lock().unwrap().dictionary.get().is_some()
}

pub fn english(text: &str, opts: &EnglishOptions) -> EnglishResult {
    let (lex, _) = lexicon();
    english_to_phonemes(text, &lex, opts)
}

// Phones as a klattsch phoneme string, stress marked with `'`.
pub fn phone_string(phones: &[Phone]) -> String {
    phones
        .iter()
        .map(|p| if p.stressed { format!("{}'", p.code) } else { p.code.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn japanese(text: &str) -> Option<String> {
    japanese_to_phonemes(text).map(|p| phone_string(&p))
}

// Start reading and parsing a dictionary file. Errors are returned as text.
pub fn start_dictionary_load(path: &str) -> Result<(), String> {
    let mut lang = LANG.lock().unwrap();
    if lang.loading || lang.dictionary.get().is_some() {
        return Ok(());
    }
    let bytes = FileAccess::get_file_as_bytes(path);
    if bytes.is_empty() {
        return Err(format!("cannot read the dictionary at {path} ({:?})", FileAccess::get_open_error()));
    }
    let bytes = bytes.to_vec();
    let slot = lang.dictionary.clone();
    lang.loading = true;
    std::thread::spawn(move || {
        let _ = slot.set(Arc::new(CmuDict::from_bytes(&bytes)));
    });
    Ok(())
}

fn number(options: &VarDictionary, key: &str) -> Option<f64> {
    options.get(key).and_then(|v| v.try_to::<f64>().ok())
}

fn flag(options: &VarDictionary, key: &str) -> Option<bool> {
    options.get(key).and_then(|v| v.try_to::<bool>().ok())
}

// CompileOptions from a Dictionary: base_f0, rate, scale, vibrato_depth,
// vibrato_rate, tremolo_depth, tremolo_rate, aspiration, tilt, effort, bank.
pub fn compile_options(options: &VarDictionary) -> CompileOptions {
    CompileOptions {
        base_f0: number(options, "base_f0"),
        rate: number(options, "rate"),
        scale: number(options, "scale"),
        vibrato_depth: number(options, "vibrato_depth"),
        vibrato_rate: number(options, "vibrato_rate"),
        tremolo_depth: number(options, "tremolo_depth"),
        tremolo_rate: number(options, "tremolo_rate"),
        aspiration: number(options, "aspiration"),
        tilt: number(options, "tilt"),
        effort: number(options, "effort"),
        bank: options.get("bank").map(|b| BankRef::Name(b.to_string())).filter(|b| *b != BankRef::Name(String::new())),
        ..CompileOptions::default()
    }
}

// EnglishOptions from a Dictionary with the field names of EnglishOptions.
fn english_options(options: &VarDictionary) -> EnglishOptions {
    let d = EnglishOptions::default();
    EnglishOptions {
        intonation: flag(options, "intonation").unwrap_or(d.intonation),
        unstress_function_words: flag(options, "unstress_function_words").unwrap_or(d.unstress_function_words),
        sound_out_unknown: flag(options, "sound_out_unknown").unwrap_or(d.sound_out_unknown),
        declination_hz: number(options, "declination_hz").unwrap_or(d.declination_hz),
        final_fall_hz: number(options, "final_fall_hz").unwrap_or(d.final_fall_hz),
        question_rise_hz: number(options, "question_rise_hz").unwrap_or(d.question_rise_hz),
        wh_questions_fall: flag(options, "wh_questions_fall").unwrap_or(d.wh_questions_fall),
        comma_rise_hz: number(options, "comma_rise_hz").unwrap_or(d.comma_rise_hz),
    }
}

fn source_name(source: &WordSource) -> String {
    match source {
        WordSource::Dictionary => "dictionary".into(),
        WordSource::Number => "number".into(),
        WordSource::Acronym => "spelled".into(),
        WordSource::Expansion(w) => format!("expansion: {}", w.join(" ")),
        WordSource::Inline => "inline".into(),
        WordSource::Parts(p) => format!("parts: {}", p.join(" + ")),
        WordSource::LetterToSound => "sounded out".into(),
        WordSource::Skipped => "skipped".into(),
    }
}

#[derive(GodotClass)]
#[class(tool, init, singleton)]
pub struct Klattsch {
    polling: bool,
    base: Base<Object>,
}

#[godot_api]
impl Klattsch {
    // Emitted once the CMU dictionary has been parsed.
    #[signal]
    fn dictionary_loaded();

    // --- the dictionary and lexicon ---

    // Start loading the CMU dictionary (read now, parsed in the background).
    // English works before it is ready, from the lexicon and letter-to-sound
    // rules only. Returns false if the file cannot be read. Add the file's
    // extension (*.dict) to the export preset's non-resource filters.
    #[func]
    pub fn load_dictionary(&mut self, #[opt(default = DEFAULT_DICTIONARY)] path: GString) -> bool {
        if let Err(e) = start_dictionary_load(&path.to_string()) {
            godot_error!("Klattsch.load_dictionary: {e}");
            return false;
        }
        self.poll_until_loaded();
        true
    }

    #[func]
    fn is_dictionary_ready(&self) -> bool {
        dictionary_ready()
    }

    // Add (or replace) a word for this run of the game, checked before every
    // word list and the dictionary. `arpabet` uses CMU notation with optional
    // stress digits: "Z IH1 K S".
    #[func]
    fn add_word(&self, word: GString, arpabet: GString) -> bool {
        let mut lang = LANG.lock().unwrap();
        match Arc::make_mut(&mut lang.words).insert(&word.to_string(), &arpabet.to_string()) {
            Ok(()) => true,
            Err(e) => {
                godot_error!("Klattsch.add_word: {e}");
                false
            }
        }
    }

    #[func]
    fn remove_word(&self, word: GString) -> bool {
        Arc::make_mut(&mut LANG.lock().unwrap().words).remove(&word.to_string())
    }

    // Load a word list in CMU format (";;;" comments, "WORD  PH1 PH2 ...")
    // through FileAccess (res:// or user://). Lists loaded earlier win.
    #[func]
    fn load_word_list(&self, path: GString) -> bool {
        let bytes = FileAccess::get_file_as_bytes(&path);
        if bytes.is_empty() {
            godot_error!("Klattsch.load_word_list: cannot read {path} ({:?})", FileAccess::get_open_error());
            return false;
        }
        LANG.lock().unwrap().lists.push(Arc::new(CmuDict::from_bytes(bytes.as_slice())));
        true
    }

    // --- text ---

    #[func]
    fn english_to_phonemes(&self, text: GString) -> GString {
        english(&text.to_string(), &EnglishOptions::default()).phonemes.as_str().into()
    }

    // With options named like EnglishOptions: intonation,
    // unstress_function_words, sound_out_unknown, declination_hz,
    // final_fall_hz, question_rise_hz, wh_questions_fall, comma_rise_hz.
    #[func]
    fn english_to_phonemes_with(&self, text: GString, options: VarDictionary) -> GString {
        english(&text.to_string(), &english_options(&options)).phonemes.as_str().into()
    }

    // How each word of an English text was pronounced: text, source
    // ("dictionary", "number", "spelled", "sounded out", ...) and phonemes.
    #[func]
    fn english_words(&self, text: GString) -> Array<VarDictionary> {
        let mut out = Array::new();
        for w in english(&text.to_string(), &EnglishOptions::default()).words {
            out.push(&vdict! {
                "text" => w.text.as_str(),
                "source" => source_name(&w.source).as_str(),
                "phonemes" => phone_string(&w.phones).as_str(),
            });
        }
        out
    }

    // Kana or romaji to klattsch phonemes (speak them with a ja-* bank);
    // empty if the text is neither.
    #[func]
    fn japanese_to_phonemes(&self, text: GString) -> GString {
        japanese(&text.to_string()).unwrap_or_default().as_str().into()
    }

    // --- banks ---

    // Register a voice bank from JSON text; returns its name, or "" on error.
    #[func]
    fn register_bank_json(&self, json: GString) -> StringName {
        match banks().register_json(&json.to_string()) {
            Ok(name) => StringName::from(name.as_str()),
            Err(e) => {
                godot_error!("Klattsch.register_bank_json: {e}");
                StringName::default()
            }
        }
    }

    // Register a voice bank file through FileAccess (res:// or user://).
    #[func]
    fn load_bank(&self, path: GString) -> StringName {
        let json = FileAccess::get_file_as_string(&path);
        if json.is_empty() {
            godot_error!("Klattsch.load_bank: cannot read {path} ({:?})", FileAccess::get_open_error());
            return StringName::default();
        }
        self.register_bank_json(json)
    }

    #[func]
    fn get_bank_names(&self) -> PackedStringArray {
        banks().list().iter().map(|n| GString::from(n.as_str())).collect()
    }

    // --- highlighting ---

    // Syntax-highlight spans for a phoneme string, covering every character:
    // start, end (character offsets), text, and class ("hl-phoneme",
    // "hl-directive hl-d-pitch", ...; "" for whitespace). Phonemes that
    // `bank` (empty: klatt1980-en, the default) lacks are classed
    // "hl-badphoneme"; [bank=...] in the text switches banks. See
    // KlattschSyntaxHighlighter for a ready-made TextEdit highlighter.
    #[func]
    fn highlight(&self, phonemes: GString, #[opt(default = "")] bank: GString) -> Array<VarDictionary> {
        let mut out = Array::new();
        for span in crate::highlighter::spans(&phonemes.to_string(), &bank.to_string()) {
            out.push(&vdict! {
                "start" => span.start as i64,
                "end" => span.end as i64,
                "text" => span.text.as_str(),
                "class" => span.class.as_deref().unwrap_or(""),
            });
        }
        out
    }

    // --- compiling ---

    // Compile a klattsch phoneme string with default options. Returns null
    // on error, after reporting it.
    #[func]
    fn compile(phonemes: GString) -> Option<Gd<KlattschUtterance>> {
        Self::compile_with(phonemes, VarDictionary::new())
    }

    // Compile with options mirroring CompileOptions (see compile_options).
    // (gdext allows no default value for a Dictionary parameter, hence two
    // methods.)
    #[func]
    fn compile_with(phonemes: GString, options: VarDictionary) -> Option<Gd<KlattschUtterance>> {
        match klattsch_core::compile_string(&phonemes.to_string(), &compile_options(&options)) {
            Ok(result) => Some(KlattschUtterance::create(result)),
            Err(e) => {
                godot_error!("Klattsch.compile: {e}");
                None
            }
        }
    }

    // --- pre-rendering ---

    // Render an utterance (all voices mixed) into a KlattschSample.
    // `sample_rate` 0 means the audio server's mix rate.
    #[func]
    fn render(&self, utterance: Gd<KlattschUtterance>, #[opt(default = 0.0)] sample_rate: f64) -> Option<Gd<KlattschSample>> {
        let rate = if sample_rate > 0.0 { sample_rate } else { f64::from(AudioServer::singleton().get_mix_rate()) };
        match render_mix(&utterance.bind().result, rate) {
            Ok(samples) => Some(KlattschSample::create(samples, rate)),
            Err(e) => {
                godot_error!("Klattsch.render: {e}");
                None
            }
        }
    }

    // Write an utterance as a WAV file exactly as klattsch-cli does: 16-bit
    // PCM, peak-normalized to 0.95, with klattsch's software tag and
    // `comment` (empty: the utterance's phoneme string; klattsch-cli writes
    // the text it was given, so pass the English text for --english). Writes
    // through FileAccess, so user:// works.
    #[func]
    fn save_wav(
        &self,
        utterance: Gd<KlattschUtterance>,
        path: GString,
        #[opt(default = "")] comment: GString,
        #[opt(default = 48000)] sample_rate: i64,
    ) -> GodotError {
        let u = utterance.bind();
        let Ok(rate) = u32::try_from(sample_rate) else { return GodotError::ERR_INVALID_PARAMETER };
        let samples = match render_mix(&u.result, f64::from(rate)) {
            Ok(s) => s,
            Err(e) => {
                godot_error!("Klattsch.save_wav: {e}");
                return GodotError::ERR_INVALID_DATA;
            }
        };
        let comment = if comment.is_empty() { u.result.source.clone() } else { comment.to_string() };
        let metadata = WavMetadata { software: Some(SOFTWARE.into()), comment: Some(comment) };
        let wav = encode_wav(&samples, rate, &WavOptions { metadata: Some(metadata), ..WavOptions::default() });
        let Some(mut file) = FileAccess::open(&path, ModeFlags::WRITE) else {
            let err = FileAccess::get_open_error();
            godot_error!("Klattsch.save_wav: cannot write {path} ({err:?})");
            return err;
        };
        file.store_buffer(&PackedByteArray::from(wav.bytes.as_slice()));
        file.close();
        GodotError::OK
    }

    // Called every frame while the dictionary loads.
    #[func]
    fn _poll_dictionary(&mut self) {
        if !dictionary_ready() {
            return;
        }
        self.stop_polling();
        self.signals().dictionary_loaded().emit();
    }
}

impl Klattsch {
    // Drop every connection to the singleton's signals while the main loop
    // shuts down. The singleton outlives GDScript (it is freed when the
    // library unloads), and freeing it with a GDScript lambda still
    // connected crashes an exported game on exit.
    pub fn disconnect_all() {
        let mut me = Self::singleton();
        for signal in me.get_signal_list().iter_shared() {
            let Some(name) = signal.get("name").and_then(|n| n.try_to::<GString>().ok()) else { continue };
            let name = StringName::from(&name);
            for conn in me.get_signal_connection_list(&name).iter_shared() {
                if let Some(callable) = conn.get("callable").and_then(|c| c.try_to::<Callable>().ok()) {
                    me.disconnect(&name, &callable);
                }
            }
        }
    }

    // Watch SceneTree.process_frame until the parse thread finishes (if it
    // already has, dictionary_loaded is still emitted, on the next frame).
    pub fn poll_until_loaded(&mut self) {
        if !self.polling {
            self.polling = true;
            self.connect_poll();
        }
    }

    fn tree() -> Option<Gd<SceneTree>> {
        Engine::singleton().get_main_loop().and_then(|m| m.try_cast::<SceneTree>().ok())
    }

    fn connect_poll(&mut self) {
        let callable = Callable::from_object_method(&self.to_gd(), "_poll_dictionary");
        if let Some(mut tree) = Self::tree() {
            tree.connect("process_frame", &callable);
        }
    }

    fn stop_polling(&mut self) {
        self.polling = false;
        let callable = Callable::from_object_method(&self.to_gd(), "_poll_dictionary");
        if let Some(mut tree) = Self::tree()
            && tree.is_connected("process_frame", &callable)
        {
            tree.disconnect("process_frame", &callable);
        }
    }
}
