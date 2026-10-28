// Syntax highlighting lexer for editor overlays.
//
// Mirrors the token grammar of the sequencer (tokenize/classify_part) but
// operates on the raw, un-normalized editor text so spans line up with what
// the editor displays, and emits every character of input (tokens,
// whitespace, comments) as spans. Offsets are code points.
//
// The JS version builds DOM nodes and appends a trailing "\n" text node to keep
// a textarea backdrop's height in sync; that sentinel is a DOM layout detail
// and is not emitted here.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;

use crate::banks::BankRegistry;
use crate::js;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    // CSS-style class list, e.g. "hl-directive hl-d-pitch"; `None` for plain
    // text (whitespace, or token text past the classified pieces).
    pub class: Option<String>,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

// Returns the set of codes valid in a bank, or `None` if the bank is unknown.
pub type PhonemesFor<'a> = dyn Fn(&str) -> Option<HashSet<String>> + 'a;

#[derive(Default)]
pub struct HighlightOptions<'a> {
    pub phonemes_for: Option<&'a PhonemesFor<'a>>,
    // The bank selected in the UI.
    pub initial_bank: Option<&'a str>,
}

// A phonemes_for lookup backed by a bank registry.
pub fn registry_phonemes_for(registry: &BankRegistry) -> impl Fn(&str) -> Option<HashSet<String>> + '_ {
    move |name| registry.get(name).ok().flatten().map(|b| b.phonemes.keys().cloned().collect())
}

fn is_pause_word(part: &str) -> bool {
    matches!(part, "," | ";" | ".")
}

fn is_directive_letter(l: &str) -> bool {
    matches!(l, "b" | "r" | "p" | "s" | "v" | "w" | "m" | "n" | "h" | "t" | "g")
}

// Directive color families: pitch, timing, formant scale, modulation, voice quality.
fn letter_family(l: &str) -> Option<&'static str> {
    Some(match l {
        "b" => "pitch",
        "r" | "p" => "time",
        "s" => "scale",
        "v" | "w" | "m" | "n" => "mod",
        "h" | "t" | "g" => "voice",
        _ => return None,
    })
}

fn key_family(k: &str) -> Option<&'static str> {
    Some(match k {
        "base" => "pitch",
        "rate" | "pause" => "time",
        "scale" => "scale",
        "vibrato" | "vibratoRate" | "tremolo" | "tremoloRate" => "mod",
        "aspiration" | "tilt" | "effort" => "voice",
        "voice" => "voice",
        _ => return None,
    })
}

fn dir_cls(family: Option<&str>) -> String {
    match family {
        Some(f) => format!("hl-directive hl-d-{f}"),
        None => "hl-directive".into(),
    }
}

fn val_cls(family: Option<&str>) -> String {
    match family {
        Some(f) => format!("hl-value hl-v-{f}"),
        None => "hl-value".into(),
    }
}

static RE_BANK_SWITCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[bank=([A-Za-z0-9_.\-]+)\]$").unwrap());
static RE_BRACKET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[([A-Za-z0-9_]+)=(-?[0-9]+(?:\.[0-9]+)?)\]$").unwrap());
static RE_NOTE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(b)(=?)([A-G][b#]?-?[0-9]+)$").unwrap());
static RE_COMPACT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([a-z])((=)?(([+-])?[0-9]+(?:\.[0-9]+)?))?$").unwrap());
static RE_PHONEME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Z]+)(['!]?)(\([+-][0-9]+(?:\.[0-9]+)?\)|[+-][0-9]+(?:\.[0-9]+)?)?$").unwrap()
});

struct State<'a> {
    phonemes_for: Option<&'a PhonemesFor<'a>>,
    initial_set: Option<HashSet<String>>,
    active_set: Option<HashSet<String>>,
}

type Piece = (String, String);

fn piece(cls: impl Into<String>, text: impl Into<String>) -> Piece {
    (cls.into(), text.into())
}

// Classify one whitespace-delimited part into a list of (class, text) pieces.
fn classify(part: &str, state: &mut State) -> Vec<Piece> {
    if part == "(" || part == ")" {
        return vec![piece("hl-paren", part)];
    }
    if is_pause_word(part) {
        return vec![piece("hl-pause", part)];
    }
    if part == "!" || part == "'" {
        return vec![piece("hl-stress", part)];
    }

    if let Some(m) = RE_BANK_SWITCH.captures(part) {
        let known = state.phonemes_for.and_then(|f| f(&m[1]));
        if state.phonemes_for.is_some() && known.is_none() {
            return vec![piece("hl-unknown", part)];
        }
        state.active_set = known;
        return vec![piece("hl-bank", part)];
    }
    if part == "[bank]" {
        state.active_set = state.initial_set.clone();
        return vec![piece("hl-bank", part)];
    }

    if let Some(m) = RE_BRACKET.captures(part) {
        let fam = key_family(&m[1]);
        return vec![
            piece(dir_cls(fam), format!("[{}=", &m[1])),
            piece(val_cls(fam), &m[2]),
            piece(dir_cls(fam), "]"),
        ];
    }

    if let Some(m) = RE_NOTE.captures(part) {
        return vec![
            piece(dir_cls(Some("pitch")), format!("{}{}", &m[1], &m[2])),
            piece(val_cls(Some("pitch")), &m[3]),
        ];
    }

    if let Some(m) = RE_COMPACT.captures(part) {
        if is_directive_letter(&m[1]) {
            let fam = letter_family(&m[1]);
            return match m.get(2) {
                // Bare letter: reset directive (bare `p` is dropped by the engine).
                None => vec![piece(dir_cls(fam), &m[1])],
                Some(rest) => vec![piece(dir_cls(fam), &m[1]), piece(val_cls(fam), rest.as_str())],
            };
        }
    }

    if let Some(m) = RE_PHONEME.captures(part) {
        let code = &m[1];
        let known = state.active_set.as_ref().is_none_or(|s| s.contains(code));
        let mut pieces = vec![piece(if known { "hl-phoneme" } else { "hl-badphoneme" }, code)];
        if let Some(stress) = m.get(2).filter(|s| !s.as_str().is_empty()) {
            pieces.push(piece("hl-stress", stress.as_str()));
        }
        if let Some(delta) = m.get(3) {
            let cls = if delta.as_str().starts_with('(') { "hl-transient" } else { "hl-pitch" };
            pieces.push(piece(cls, delta.as_str()));
        }
        return pieces;
    }

    vec![piece("hl-unknown", part)]
}

struct Builder<'s> {
    chars: &'s [char],
    spans: Vec<HighlightSpan>,
}

impl Builder<'_> {
    fn push(&mut self, class: Option<String>, start: usize, end: usize) {
        let text = self.chars[start..end].iter().collect();
        self.spans.push(HighlightSpan { class, start, end, text });
    }
}

// Build highlight spans covering every character of `source`.
pub fn build_highlight(source: &str, opts: &HighlightOptions) -> Vec<HighlightSpan> {
    let chars: Vec<char> = source.chars().collect();
    let len = chars.len();
    let phonemes_for = opts.phonemes_for;
    let initial_set = match (phonemes_for, opts.initial_bank) {
        (Some(f), Some(bank)) if !bank.is_empty() => f(bank),
        _ => None,
    };
    let mut state = State { phonemes_for, active_set: initial_set.clone(), initial_set };
    let mut out = Builder { chars: &chars, spans: Vec::new() };

    let mut i = 0;
    let mut plain_start = 0; // start of pending whitespace run

    let flush_plain = |out: &mut Builder, plain_start: usize, end: usize| {
        if end > plain_start {
            out.push(None, plain_start, end);
        }
    };

    let block_end = |start: usize| {
        let mut j = start + 2;
        while j + 1 < len {
            if chars[j] == '*' && chars[j + 1] == '/' {
                return j + 2;
            }
            j += 1;
        }
        len
    };
    let comment = || Some("hl-comment".to_string());

    while i < len {
        let c = chars[i];
        if js::is_space(c) {
            i += 1;
            continue;
        }

        // Line comment: # at start of input or after whitespace.
        if c == '#' && (i == 0 || js::is_space(chars[i - 1])) {
            flush_plain(&mut out, plain_start, i);
            let start = i;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            out.push(comment(), start, i);
            plain_start = i;
            continue;
        }

        // Block comment between tokens.
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            flush_plain(&mut out, plain_start, i);
            let start = i;
            i = block_end(i);
            out.push(comment(), start, i);
            plain_start = i;
            continue;
        }

        // Token: runs to next whitespace; an embedded /* ... */ splits it into
        // segments that the engine concatenates back together.
        flush_plain(&mut out, plain_start, i);
        let mut segments: Vec<(usize, usize, bool)> = Vec::new(); // (start, end, is_comment)
        let mut seg_start = i;
        let mut part = String::new();
        while i < len && !js::is_space(chars[i]) {
            if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                if i > seg_start {
                    segments.push((seg_start, i, false));
                }
                let ce = block_end(i);
                segments.push((i, ce, true));
                i = ce;
                seg_start = i;
                continue;
            }
            part.push(chars[i]);
            i += 1;
        }
        if i > seg_start {
            segments.push((seg_start, i, false));
        }
        plain_start = i;
        if part.is_empty() {
            for (start, end, _) in segments {
                out.push(comment(), start, end);
            }
            continue;
        }

        let pieces = classify(&part, &mut state);
        // Distribute classified pieces across non-comment segments in order.
        let mut pi = 0; // current piece
        let mut consumed = 0; // chars of current piece already emitted
        for (start, end, is_comment) in segments {
            if is_comment {
                out.push(comment(), start, end);
                continue;
            }
            let mut pos = start;
            while pos < end && pi < pieces.len() {
                let (cls, text) = &pieces[pi];
                let text_len = text.chars().count();
                let take = (end - pos).min(text_len - consumed);
                out.push(Some(cls.clone()), pos, pos + take);
                pos += take;
                consumed += take;
                if consumed >= text_len {
                    pi += 1;
                    consumed = 0;
                }
            }
            if pos < end {
                out.push(None, pos, end);
            }
        }
    }
    flush_plain(&mut out, plain_start, len);

    out.spans
}
