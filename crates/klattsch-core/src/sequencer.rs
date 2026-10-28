// Phoneme-string parser and schedule compiler
//
// Source offsets (`src_start`, `src_end`, ...) count Unicode code points of
// the normalized source, which is how Godot strings index. The JS engine
// counts UTF-16 code units; the two agree for text without astral characters.

use std::sync::{Arc, LazyLock};

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::banks::{banks, BankRef, BankRegistry, ResolvedBank};
use crate::error::Result;
use crate::js;
use crate::value::{map_to_json, num_json, Map, Value};

fn pause_ms(part: &str) -> Option<f64> {
    match part {
        "," => Some(100.0),
        ";" => Some(200.0),
        "." => Some(300.0),
        _ => None,
    }
}

// Convert note names like "C4", "C#5", "Db3", "A-1" to Hz.
static NOTE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([A-G])([b#]?)(-?[0-9]+)$").unwrap());

pub fn note_to_hz(name: &str) -> Option<f64> {
    let m = NOTE_RE.captures(name)?;
    let mut semi: f64 = match &m[1] {
        "C" => 0.0,
        "D" => 2.0,
        "E" => 4.0,
        "F" => 5.0,
        "G" => 7.0,
        "A" => 9.0,
        _ => 11.0,
    };
    match &m[2] {
        "#" => semi += 1.0,
        "b" => semi -= 1.0,
        _ => {}
    }
    let octave: f64 = m[3].parse().ok()?;
    let midi = (octave + 1.0) * 12.0 + semi;
    Some(440.0 * js::pow(2.0, (midi - 69.0) / 12.0))
}

pub mod defaults {
    pub const BASE_F0: f64 = 120.0;
    pub const RATE: f64 = 110.0; // ms per phoneme
    pub const STRESS_DURATION_FACTOR: f64 = 1.5;
    pub const STRESS_F0_LIFT: f64 = 8.0; // Hz
    pub const STOP_BURST_MS: f64 = 25.0;
    pub const DEFAULT_TRANSITION_MS: f64 = 35.0;
    pub const SENTENCE_FINAL_HOLD_MS: f64 = 0.0;
    pub const FADE_OUT_MS: f64 = 100.0;
    pub const TRAIL_OFF_MS: f64 = 150.0;
}

fn homoglyph(c: char) -> char {
    match c {
        // Greek uppercase that look like Latin uppercase
        'Α' => 'A', 'Β' => 'B', 'Ε' => 'E', 'Η' => 'H', 'Ι' => 'I', 'Κ' => 'K',
        'Μ' => 'M', 'Ν' => 'N', 'Ο' => 'O', 'Ρ' => 'P', 'Τ' => 'T', 'Υ' => 'Y', 'Ζ' => 'Z',
        // Cyrillic uppercase that look like Latin uppercase
        'А' => 'A', 'В' => 'B', 'С' => 'C', 'Е' => 'E', 'Н' => 'H', 'К' => 'K',
        'М' => 'M', 'О' => 'O', 'Р' => 'P', 'Т' => 'T',
        // Cyrillic lowercase
        'а' => 'a', 'с' => 'c', 'е' => 'e', 'о' => 'o', 'р' => 'p',
        _ => c,
    }
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

pub fn normalize(input: &str) -> String {
    input.nfkc().filter(|c| !is_zero_width(*c)).map(homoglyph).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Directive {
    pub key: String,
    // `None` for a reset (bare letter or `[KEY]`).
    pub value: Option<f64>,
    pub relative: bool,
}

impl Directive {
    pub fn is_reset(&self) -> bool {
        self.value.is_none()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PhonemeToken {
    pub code: String,
    pub stressed: bool,
    pub pitch_delta: f64,
    pub transient: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    SyllableOpen,
    SyllableClose,
    Pause { ms: f64 },
    BankSwitch { name: String },
    BankReset,
    EngineSwitch { name: String },
    EngineReset,
    Directive(Directive),
    Phoneme(PhonemeToken),
    Unknown { text: String },
}

impl TokenKind {
    // The JS token `type` string.
    pub fn type_name(&self) -> &'static str {
        match self {
            TokenKind::SyllableOpen => "syllable_open",
            TokenKind::SyllableClose => "syllable_close",
            TokenKind::Pause { .. } => "pause",
            TokenKind::BankSwitch { .. } => "bank_switch",
            TokenKind::BankReset => "bank_reset",
            TokenKind::EngineSwitch { .. } => "engine_switch",
            TokenKind::EngineReset => "engine_reset",
            TokenKind::Directive(_) => "directive",
            TokenKind::Phoneme(_) => "phoneme",
            TokenKind::Unknown { .. } => "unknown",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub src_start: usize,
    pub src_end: usize,
}

impl Token {
    // The token in the JS engine's object shape.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::json;
        let mut o = serde_json::Map::new();
        o.insert("type".into(), json!(self.kind.type_name()));
        match &self.kind {
            TokenKind::Pause { ms } => {
                o.insert("ms".into(), num_json(*ms));
            }
            TokenKind::BankSwitch { name } | TokenKind::EngineSwitch { name } => {
                o.insert("name".into(), json!(name));
            }
            TokenKind::Directive(d) => {
                o.insert("key".into(), json!(d.key));
                match d.value {
                    Some(v) => {
                        o.insert("value".into(), num_json(v));
                        o.insert("relative".into(), json!(d.relative));
                    }
                    None => {
                        o.insert("reset".into(), json!(true));
                    }
                }
            }
            TokenKind::Phoneme(p) => {
                o.insert("code".into(), json!(p.code));
                o.insert("stressed".into(), json!(p.stressed));
                o.insert("pitchDelta".into(), num_json(p.pitch_delta));
                o.insert("transient".into(), json!(p.transient));
            }
            TokenKind::Unknown { text } => {
                o.insert("text".into(), json!(text));
            }
            _ => {}
        }
        o.insert("srcStart".into(), json!(self.src_start));
        o.insert("srcEnd".into(), json!(self.src_end));
        serde_json::Value::Object(o)
    }
}

enum Classified {
    Token(TokenKind),
    StressMark,
    Skip,
}

static BANK_SWITCH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[bank=([A-Za-z0-9_.\-]+)\]$").unwrap());
static ENGINE_SWITCH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[engine=([A-Za-z0-9_.\-]+)\]$").unwrap());
static BRACKET_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[([A-Za-z0-9_]+)=(-?[0-9]+(?:\.[0-9]+)?)\]$").unwrap());
static BRACKET_RESET_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[([A-Z][A-Za-z0-9_]*)\]$").unwrap());
static NOTE_FORM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(b)(=)?([A-G][b#]?-?[0-9]+)$").unwrap());
static COMPACT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-z])(?:(=)?(([+-])?[0-9]+(?:\.[0-9]+)?))?$").unwrap());
static PHONEME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Z]+)(['!])?(?:\(([+-][0-9]+(?:\.[0-9]+)?)\)|([+-][0-9]+(?:\.[0-9]+)?))?$").unwrap()
});

fn num(s: &str) -> f64 {
    s.parse().unwrap_or(f64::NAN)
}

fn compact_key(letter: &str) -> Option<&'static str> {
    Some(match letter {
        "b" => "base",
        "r" => "rate",
        "p" => "pause",
        "s" => "scale",
        "v" => "vibrato",
        "w" => "vibratoRate",
        "m" => "tremolo",
        "n" => "tremoloRate",
        "h" => "aspiration",
        "t" => "tilt",
        "g" => "effort",
        _ => return None,
    })
}

fn directive(key: &str, value: Option<f64>, relative: bool) -> Classified {
    Classified::Token(TokenKind::Directive(Directive { key: key.into(), value, relative }))
}

fn classify_part(part: &str) -> Classified {
    use Classified::Token as T;
    if part == "(" {
        return T(TokenKind::SyllableOpen);
    }
    if part == ")" {
        return T(TokenKind::SyllableClose);
    }
    if let Some(ms) = pause_ms(part) {
        return T(TokenKind::Pause { ms });
    }
    if part == "!" || part == "'" {
        return Classified::StressMark;
    }

    if let Some(m) = BANK_SWITCH_RE.captures(part) {
        return T(TokenKind::BankSwitch { name: m[1].into() });
    }
    if part == "[bank]" {
        return T(TokenKind::BankReset);
    }

    if let Some(m) = ENGINE_SWITCH_RE.captures(part) {
        return T(TokenKind::EngineSwitch { name: m[1].into() });
    }
    if part == "[engine]" {
        return T(TokenKind::EngineReset);
    }

    if let Some(m) = BRACKET_RE.captures(part) {
        return directive(&m[1], Some(num(&m[2])), false);
    }

    // Bare uppercase bracket resets an extended (engine-specific) directive to its
    // default, e.g. `[FNZ]`. The value form `[FNZ=450]` is caught by the generic
    // bracket above; lowercase brackets are not directives.
    if let Some(m) = BRACKET_RESET_RE.captures(part) {
        return directive(&m[1], None, false);
    }

    if let Some(m) = NOTE_FORM_RE.captures(part) {
        if let Some(hz) = note_to_hz(&m[3]) {
            return directive("base", Some(hz), false);
        }
    }

    if let Some(m) = COMPACT_RE.captures(part) {
        if let Some(key) = compact_key(&m[1]) {
            return match m.get(3) {
                // Bare letter reset to initial value, drop bare `p`
                None if key != "pause" => directive(key, None, false),
                None => Classified::Skip,
                Some(full) => {
                    let relative = m.get(2).is_none() && m.get(4).is_some();
                    directive(key, Some(num(full.as_str())), relative)
                }
            };
        }
    }

    if let Some(m) = PHONEME_RE.captures(part) {
        let transient_delta = m.get(3).map(|d| num(d.as_str()));
        let sticky_delta = m.get(4).map(|d| num(d.as_str()));
        return T(TokenKind::Phoneme(PhonemeToken {
            code: m[1].into(),
            stressed: m.get(2).is_some(),
            pitch_delta: transient_delta.or(sticky_delta).unwrap_or(0.0),
            transient: transient_delta.is_some(),
        }));
    }

    T(TokenKind::Unknown { text: part.into() })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tokenized {
    pub tokens: Vec<Token>,
    // The normalized source the offsets refer to.
    pub source: String,
}

pub fn tokenize(raw_input: &str) -> Tokenized {
    let source = normalize(raw_input);
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0;

    let at = |j: usize| chars.get(j).copied();
    let find_block_end = |start: usize| {
        let mut j = start + 2;
        while j + 1 < len {
            if chars[j] == '*' && chars[j + 1] == '/' {
                return j + 2;
            }
            j += 1;
        }
        len
    };

    while i < len {
        let c = chars[i];
        if js::is_space(c) {
            i += 1;
            continue;
        }
        // Line comment: # only at boundary (start of input or after whitespace).
        if c == '#' && (i == 0 || js::is_space(chars[i - 1])) {
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // Block comment
        if c == '/' && at(i + 1) == Some('*') {
            i = find_block_end(i);
            continue;
        }

        let src_start = i;
        let mut part = String::new();
        while i < len && !js::is_space(chars[i]) {
            if chars[i] == '/' && at(i + 1) == Some('*') {
                i = find_block_end(i);
                continue;
            }
            part.push(chars[i]);
            i += 1;
        }
        let src_end = i;
        if part.is_empty() {
            continue;
        }

        let kind = match classify_part(&part) {
            Classified::Skip => continue,
            Classified::StressMark => {
                if let Some(TokenKind::Phoneme(p)) =
                    tokens.iter_mut().rev().map(|t| &mut t.kind).find(|k| matches!(k, TokenKind::Phoneme(_)))
                {
                    p.stressed = true;
                }
                continue;
            }
            Classified::Token(kind) => kind,
        };
        tokens.push(Token { kind, src_start, src_end });
    }

    Tokenized { tokens, source }
}

// One timed parameter target. `target` keeps every field the phoneme bank
// defines, plus extended directives, so other engines can read them.
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleEvent {
    pub at_ms: f64,
    pub target: Map,
    pub transition_ms: f64,
}

impl ScheduleEvent {
    pub fn to_json(&self) -> serde_json::Value {
        let mut o = serde_json::Map::new();
        o.insert("atMs".into(), num_json(self.at_ms));
        o.insert("target".into(), map_to_json(&self.target));
        o.insert("transitionMs".into(), num_json(self.transition_ms));
        serde_json::Value::Object(o)
    }
}

// A span of source text and the time its sound occupies, for highlighting
// and lip-sync.
#[derive(Clone, Debug, PartialEq)]
pub struct Phrase {
    pub src_start: usize,
    pub src_end: usize,
    // the audible token
    pub token_src_start: usize,
    pub t_start_ms: f64,
    pub t_end_ms: f64,
    // JS token type: "phoneme", "pause" or "directive"
    pub kind: &'static str,
    pub phoneme: Option<String>,
}

impl Phrase {
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "srcStart": self.src_start,
            "srcEnd": self.src_end,
            "tokenSrcStart": self.token_src_start,
            "tStartMs": num_json(self.t_start_ms),
            "tEndMs": num_json(self.t_end_ms),
            "kind": self.kind,
            "phoneme": self.phoneme,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompileOptions {
    pub base_f0: Option<f64>,
    pub rate: Option<f64>,
    pub scale: Option<f64>,
    pub vibrato_depth: Option<f64>,
    pub vibrato_rate: Option<f64>,
    pub tremolo_depth: Option<f64>,
    pub tremolo_rate: Option<f64>,
    pub aspiration: Option<f64>,
    pub tilt: Option<f64>,
    pub effort: Option<f64>,
    pub bank: Option<BankRef>,
    pub engine: Option<String>,
    pub extras: Map,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VoiceResult {
    pub schedule: Vec<ScheduleEvent>,
    pub total_ms: f64,
    pub phrases: Vec<Phrase>,
    pub engine: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompileResult {
    // One entry per `[voice=N]` section; there is always at least one.
    pub voices: Vec<VoiceResult>,
    // Longest section.
    pub total_ms: f64,
    // All sections' warnings.
    pub warnings: Vec<String>,
    pub source: String,
}

impl CompileResult {
    // Voice 0's schedule (the backward-compatible top level).
    pub fn schedule(&self) -> &[ScheduleEvent] {
        &self.voices[0].schedule
    }

    pub fn phrases(&self) -> &[Phrase] {
        &self.voices[0].phrases
    }

    pub fn engine(&self) -> Option<&str> {
        self.voices[0].engine.as_deref()
    }

    // The result in the JS engine's object shape.
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::json;
        let sched = |s: &[ScheduleEvent]| s.iter().map(ScheduleEvent::to_json).collect::<Vec<_>>();
        let phr = |p: &[Phrase]| p.iter().map(Phrase::to_json).collect::<Vec<_>>();
        json!({
            "schedule": sched(self.schedule()),
            "totalMs": num_json(self.total_ms),
            "warnings": self.warnings,
            "phrases": phr(self.phrases()),
            "engine": self.engine(),
            "source": self.source,
            "voices": self.voices.iter().map(|v| json!({
                "schedule": sched(&v.schedule),
                "totalMs": num_json(v.total_ms),
                "phrases": phr(&v.phrases),
                "engine": v.engine,
            })).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone, Copy)]
struct VoiceState {
    f0: f64,
    rate: f64,
    scale: f64,
    vibrato: f64,
    vibrato_rate: f64,
    tremolo: f64,
    tremolo_rate: f64,
    aspiration: f64,
    tilt: f64,
    effort: f64,
}

struct Section<'a> {
    initial: VoiceState,
    st: VoiceState,
    initial_phonemes: Arc<ResolvedBank>,
    phonemes: Arc<ResolvedBank>,
    // Extended engine-specific directives (uppercase bracket forms like [OQ=0.6]).
    // klattsch does not interpret these; they ride into every subsequent schedule
    // target for a consuming engine to read.
    extras: Map,
    schedule: Vec<ScheduleEvent>,
    warnings: Vec<String>,
    phrases: Vec<Phrase>,
    time_ms: f64,
    // phrase covers [phrase_src_start .. token.src_end) of source and time
    // [phrase_time_start .. time_ms] when the next sound finishes
    phrase_src_start: usize,
    phrase_time_start: f64,
    in_syllable: bool,
    syllable_queue: Vec<&'a Token>,
}

fn is_higher_formant_key(k: &str) -> bool {
    let b = k.as_bytes();
    match b {
        [b'F', d] | [b'B', b'W', d] => (b'4'..=b'9').contains(d),
        _ => false,
    }
}

fn get_num(m: &Map, key: &str) -> f64 {
    m.get(key).map_or(f64::NAN, Value::to_number)
}

impl<'a> Section<'a> {
    fn emit_phrase(&mut self, t: &Token) {
        self.phrases.push(Phrase {
            src_start: self.phrase_src_start,
            src_end: t.src_end,
            token_src_start: t.src_start,
            t_start_ms: self.phrase_time_start,
            t_end_ms: self.time_ms,
            kind: t.kind.type_name(),
            phoneme: match &t.kind {
                TokenKind::Phoneme(p) => Some(p.code.clone()),
                _ => None,
            },
        });
        self.phrase_src_start = t.src_end;
        self.phrase_time_start = self.time_ms;
    }

    // Apply the running formant scale to a phoneme parameter set.
    // `glide_to` overrides the formant fields for diphthong endpoints
    fn scaled(&self, p: &Map, f0_hz: f64, glide_to: Option<&Map>) -> Map {
        let mut src = p.clone();
        if let Some(g) = glide_to {
            for (k, v) in g {
                src.insert(k.clone(), v.clone());
            }
        }
        let scale = self.st.scale;
        let mut out = src.clone();
        out.insert("F0".into(), Value::Num(f0_hz));
        for key in ["F1", "F2", "F3", "BW1", "BW2", "BW3"] {
            out.insert(key.into(), Value::Num(get_num(&src, key) * scale));
        }
        // Scale any higher formant fields a bank defines (F4/BW4 and up) so an
        // engine reading them tracks the running formant scale like F1-3 do.
        for (k, v) in &src {
            if is_higher_formant_key(k) {
                out.insert(k.clone(), Value::Num(v.to_number() * scale));
            }
        }
        out
    }

    fn emit(&mut self, mut target: Map, transition_ms: f64) {
        for (k, v) in &self.extras {
            target.insert(k.clone(), v.clone());
        }
        let st = self.st;
        for (k, v) in [
            ("vibratoDepth", st.vibrato),
            ("vibratoRate", st.vibrato_rate),
            ("tremoloDepth", st.tremolo),
            ("tremoloRate", st.tremolo_rate),
            ("aspiration", st.aspiration),
            ("tilt", st.tilt),
            ("effort", st.effort),
        ] {
            target.insert(k.into(), Value::Num(v));
        }
        self.schedule.push(ScheduleEvent { at_ms: self.time_ms, target, transition_ms });
    }

    fn silence(&mut self, transition_ms: f64) {
        let mut t = Map::new();
        t.insert("A1".into(), Value::Num(0.0));
        t.insert("A2".into(), Value::Num(0.0));
        t.insert("A3".into(), Value::Num(0.0));
        self.emit(t, transition_ms);
    }

    fn render_phoneme(&mut self, t: &PhonemeToken, slot_ms: f64) -> f64 {
        let bank = self.phonemes.clone();
        let Some(entry) = bank.phoneme(&t.code) else {
            self.warnings.push(format!("unknown phoneme: {}", t.code));
            return 0.0;
        };
        // Entries are spread like JS objects, so malformed (non-object) entries
        // contribute the same keys they do in the JS engine.
        let p = &entry.spread();
        let start_f0 = if t.stressed { self.st.f0 + defaults::STRESS_F0_LIFT } else { self.st.f0 };
        let end_f0 = start_f0 + t.pitch_delta;
        let glide_to = p.get("glideTo").filter(|g| g.truthy()).map(Value::spread);
        let glide_to = glide_to.as_ref();
        if p.get("isStop").is_some_and(Value::truthy) {
            let burst_ms = js::min(defaults::STOP_BURST_MS, slot_ms * 0.3);
            let silence_ms = slot_ms - burst_ms;
            self.silence(js::min(20.0, silence_ms * 0.4));
            self.time_ms += silence_ms;
            let target = self.scaled(p, start_f0, None);
            self.emit(target, js::min(5.0, burst_ms * 0.2));
            self.time_ms += burst_ms;
        } else if let Some(glide_to) = glide_to {
            let onset = slot_ms * 0.25;
            let glide = slot_ms * 0.50;
            let offset = slot_ms * 0.25;
            let target = self.scaled(p, start_f0, None);
            self.emit(target, js::min(20.0, onset));
            self.time_ms += onset;
            let target = self.scaled(p, end_f0, Some(glide_to));
            self.emit(target, glide);
            self.time_ms += glide + offset;
        } else if t.pitch_delta != 0.0 {
            let target = self.scaled(p, start_f0, None);
            self.emit(target, js::min(25.0, slot_ms * 0.25));
            self.time_ms += slot_ms * 0.25;
            let target = self.scaled(p, end_f0, None);
            self.emit(target, slot_ms * 0.6);
            self.time_ms += slot_ms * 0.75;
        } else {
            let trans = js::min(defaults::DEFAULT_TRANSITION_MS, slot_ms * 0.4);
            let target = self.scaled(p, start_f0, None);
            self.emit(target, trans);
            self.time_ms += slot_ms;
        }
        slot_ms
    }

    fn flush_syllable(&mut self) {
        if self.syllable_queue.is_empty() {
            self.in_syllable = false;
            return;
        }
        let queue = std::mem::take(&mut self.syllable_queue);
        let slot = self.st.rate / queue.len() as f64;
        for t in queue {
            let TokenKind::Phoneme(p) = &t.kind else { continue };
            self.render_phoneme(p, slot);
            self.emit_phrase(t);
            if !p.transient {
                self.st.f0 += p.pitch_delta;
            }
        }
        self.in_syllable = false;
    }

    fn apply_directive(&mut self, t: &Token, d: &Directive) {
        let (init, st) = (self.initial, &mut self.st);
        let field = match d.key.as_str() {
            "base" | "pitch" => Some((&mut st.f0, init.f0)),
            "rate" => Some((&mut st.rate, init.rate)),
            "scale" => Some((&mut st.scale, init.scale)),
            "vibrato" => Some((&mut st.vibrato, init.vibrato)),
            "vibratoRate" => Some((&mut st.vibrato_rate, init.vibrato_rate)),
            "tremolo" => Some((&mut st.tremolo, init.tremolo)),
            "tremoloRate" => Some((&mut st.tremolo_rate, init.tremolo_rate)),
            "aspiration" => Some((&mut st.aspiration, init.aspiration)),
            "tilt" => Some((&mut st.tilt, init.tilt)),
            "effort" => Some((&mut st.effort, init.effort)),
            _ => None,
        };
        if let Some((var, initial)) = field {
            match d.value {
                None => *var = initial,
                Some(v) if d.relative => *var += v,
                Some(v) => *var = v,
            }
            return;
        }
        if d.key == "pause" {
            self.silence(30.0);
            self.time_ms += d.value.unwrap_or(f64::NAN).abs();
            self.emit_phrase(t);
            return;
        }
        // Uppercase directive keys (e.g. [OQ=0.6], [FNZ=450]) are extended,
        // engine-specific state. Accumulate them into `extras` so they ride
        // into every subsequent target; a bare [OQ] clears the override.
        if d.key.starts_with(|c: char| c.is_ascii_uppercase()) {
            match d.value {
                None => {
                    self.extras.shift_remove(&d.key);
                }
                Some(v) => {
                    self.extras.insert(d.key.clone(), Value::Num(v));
                }
            }
        } else {
            self.warnings.push(format!("unknown directive: {}", d.key));
        }
    }
}

// Compile one voice section (a run of tokens with no [voice=N] marker) into a
// standalone schedule. Each section derives its initial state from opts.
fn compile_section(
    tokens: &[Token],
    opts: &CompileOptions,
    registry: &BankRegistry,
    section_src_start: usize,
) -> Result<VoiceResult> {
    let initial = VoiceState {
        f0: opts.base_f0.unwrap_or(defaults::BASE_F0),
        rate: opts.rate.unwrap_or(defaults::RATE),
        scale: opts.scale.unwrap_or(1.0),
        vibrato: opts.vibrato_depth.unwrap_or(0.0),
        vibrato_rate: opts.vibrato_rate.unwrap_or(5.0),
        tremolo: opts.tremolo_depth.unwrap_or(0.0),
        tremolo_rate: opts.tremolo_rate.unwrap_or(5.0),
        aspiration: opts.aspiration.unwrap_or(0.0),
        tilt: opts.tilt.unwrap_or(0.0),
        effort: opts.effort.unwrap_or(0.5),
    };
    let initial_phonemes = registry.resolve(opts.bank.as_ref())?;
    let mut s = Section {
        initial,
        st: initial,
        phonemes: initial_phonemes.clone(),
        initial_phonemes,
        extras: opts.extras.clone(),
        schedule: Vec::new(),
        warnings: Vec::new(),
        phrases: Vec::new(),
        time_ms: 0.0,
        phrase_src_start: section_src_start,
        phrase_time_start: 0.0,
        in_syllable: false,
        syllable_queue: Vec::new(),
    };
    // Bare `b` / `r` / `s` / `v` / `h` / `t` / `g` reset to opts values
    let mut engine = opts.engine.clone();

    for t in tokens {
        match &t.kind {
            TokenKind::Unknown { text } => {
                s.warnings.push(format!("unknown token: {text}"));
            }
            TokenKind::BankSwitch { name } => match registry.get(name)? {
                Some(bank) => s.phonemes = bank,
                None => s.warnings.push(format!("unknown bank: {name}")),
            },
            TokenKind::BankReset => s.phonemes = s.initial_phonemes.clone(),
            TokenKind::EngineSwitch { name } => engine = Some(name.clone()),
            TokenKind::EngineReset => engine = opts.engine.clone(),
            TokenKind::SyllableOpen => {
                if s.in_syllable {
                    s.warnings.push("nested ( ignored".into());
                    continue;
                }
                s.in_syllable = true;
                s.syllable_queue.clear();
            }
            TokenKind::SyllableClose => {
                if !s.in_syllable {
                    s.warnings.push("unmatched )".into());
                    continue;
                }
                s.flush_syllable();
            }
            TokenKind::Directive(d) => s.apply_directive(t, d),
            TokenKind::Pause { ms } => {
                s.silence(30.0);
                s.time_ms += ms;
                s.emit_phrase(t);
            }
            TokenKind::Phoneme(p) => {
                // phoneme: defer to the group buffer if inside (...), otherwise
                // render straight to the schedule
                if s.in_syllable {
                    s.syllable_queue.push(t);
                    continue;
                }
                let phone_rate =
                    if p.stressed { s.st.rate * defaults::STRESS_DURATION_FACTOR } else { s.st.rate };
                s.render_phoneme(p, phone_rate);
                s.emit_phrase(t);
                if !p.transient {
                    s.st.f0 += p.pitch_delta;
                }
            }
        }
    }

    if s.in_syllable {
        s.warnings.push("unclosed (".into());
        s.flush_syllable();
    }

    s.time_ms += defaults::SENTENCE_FINAL_HOLD_MS;
    s.silence(defaults::FADE_OUT_MS);
    s.time_ms += defaults::TRAIL_OFF_MS;

    // Hold the final phrase highlighted
    if let Some(last) = s.phrases.last_mut() {
        last.t_end_ms = s.time_ms;
    }

    Ok(VoiceResult {
        schedule: s.schedule,
        total_ms: s.time_ms,
        phrases: s.phrases,
        engine,
        warnings: s.warnings,
    })
}

pub fn compile_with_registry(
    parsed: &Tokenized,
    opts: &CompileOptions,
    registry: &BankRegistry,
) -> Result<CompileResult> {
    // Partition tokens into voice sections at each [voice=N] marker. Sections are
    // positional. Text before the first marker is voice 0. Each section compiles
    // independently.
    let mut sections: Vec<Vec<Token>> = vec![Vec::new()];
    let mut section_starts = vec![0usize];
    for t in &parsed.tokens {
        if let TokenKind::Directive(d) = &t.kind {
            if d.key == "voice" {
                sections.push(Vec::new());
                section_starts.push(t.src_end);
                continue;
            }
        }
        sections.last_mut().expect("at least one section").push(t.clone());
    }

    let voices = sections
        .iter()
        .zip(&section_starts)
        .map(|(toks, start)| compile_section(toks, opts, registry, *start))
        .collect::<Result<Vec<_>>>()?;

    Ok(CompileResult {
        total_ms: voices.iter().fold(f64::NEG_INFINITY, |m, v| js::max(m, v.total_ms)),
        warnings: voices.iter().flat_map(|v| v.warnings.iter().cloned()).collect(),
        source: parsed.source.clone(),
        voices,
    })
}

pub fn compile(parsed: &Tokenized, opts: &CompileOptions) -> Result<CompileResult> {
    compile_with_registry(parsed, opts, banks())
}

pub fn compile_string(input: &str, opts: &CompileOptions) -> Result<CompileResult> {
    compile(&tokenize(input), opts)
}

pub fn compile_string_with_registry(
    input: &str,
    opts: &CompileOptions,
    registry: &BankRegistry,
) -> Result<CompileResult> {
    compile_with_registry(&tokenize(input), opts, registry)
}
