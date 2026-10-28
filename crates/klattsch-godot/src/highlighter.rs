// Syntax highlighting for klattsch phoneme strings: KlattschSyntaxHighlighter
// for any TextEdit or CodeEdit (in a game, a tool, or the audition panel),
// and KlattschEditorSyntaxHighlighter for the script editor (Edit > Syntax
// Highlighter > "klattsch phonemes", for text files of phoneme strings).
//
// klattsch_core::build_highlight classifies a whole text at once (a block
// comment or a [bank=...] switch reaches later lines), so the text is
// re-highlighted whenever it changes and split into lines. Godot asks for
// one line at a time and caches lines itself, so after a re-highlight its
// cache is cleared (deferred) to repaint lines an edit elsewhere recoloured.

use std::cell::RefCell;

use godot::classes::{EditorSyntaxHighlighter, IEditorSyntaxHighlighter, ISyntaxHighlighter, SyntaxHighlighter, TextEdit};
use godot::prelude::*;
use klattsch_core::{banks, build_highlight, registry_phonemes_for, HighlightOptions, HighlightSpan, DEFAULT_BANK};

// Highlight spans, checking phonemes against `bank` (empty: the default bank).
pub fn spans(source: &str, bank: &str) -> Vec<HighlightSpan> {
    let phonemes_for = registry_phonemes_for(banks());
    let bank = if bank.is_empty() { DEFAULT_BANK } else { bank };
    build_highlight(source, &HighlightOptions { phonemes_for: Some(&phonemes_for), initial_bank: Some(bank) })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Plain,
    Phoneme,
    Stress,
    Shift,
    Transient,
    Pause,
    Bracket,
    Bank,
    Comment,
    Error,
    Pitch,
    Timing,
    Scale,
    Modulation,
    VoiceQuality,
}

fn kind(class: Option<&str>) -> Kind {
    let Some(class) = class else { return Kind::Plain };
    let mut words = class.split(' ');
    match words.next().unwrap_or_default() {
        "hl-phoneme" => Kind::Phoneme,
        "hl-badphoneme" | "hl-unknown" => Kind::Error,
        "hl-stress" => Kind::Stress,
        "hl-pitch" => Kind::Shift,
        "hl-transient" => Kind::Transient,
        "hl-pause" => Kind::Pause,
        "hl-paren" => Kind::Bracket,
        "hl-bank" => Kind::Bank,
        "hl-comment" => Kind::Comment,
        // "hl-directive hl-d-pitch", "hl-value hl-v-time", ...; no family
        // for engine-specific brackets like [OQ=0.6].
        "hl-directive" | "hl-value" => match words.next().and_then(|w| w.rsplit('-').next()) {
            Some("pitch") => Kind::Pitch,
            Some("time") => Kind::Timing,
            Some("scale") => Kind::Scale,
            Some("mod") => Kind::Modulation,
            Some("voice") => Kind::VoiceQuality,
            _ => Kind::Bracket,
        },
        _ => Kind::Plain,
    }
}

// Colours by Kind (Plain is the TextEdit's font colour).
#[derive(Clone, Copy)]
struct Palette {
    phoneme: Color,
    stress: Color,
    shift: Color,
    transient: Color,
    pause: Color,
    bracket: Color,
    bank: Color,
    comment: Color,
    error: Color,
    pitch: Color,
    timing: Color,
    scale: Color,
    modulation: Color,
    voice_quality: Color,
}

// For Godot's default (dark) editor theme.
const PALETTE: Palette = Palette {
    phoneme: Color::from_rgb(0.74, 0.88, 1.0),
    stress: Color::from_rgb(1.0, 0.55, 0.35),
    shift: Color::from_rgb(0.63, 1.0, 0.88),
    transient: Color::from_rgb(0.4, 0.86, 0.75),
    pause: Color::from_rgb(1.0, 0.93, 0.63),
    bracket: Color::from_rgb(0.67, 0.79, 1.0),
    bank: Color::from_rgb(0.26, 1.0, 0.76),
    comment: Color::from_rgba(0.8, 0.81, 0.82, 0.5),
    error: Color::from_rgb(1.0, 0.47, 0.47),
    pitch: Color::from_rgb(0.78, 0.6, 1.0),
    timing: Color::from_rgb(1.0, 0.82, 0.4),
    scale: Color::from_rgb(0.45, 0.85, 1.0),
    modulation: Color::from_rgb(1.0, 0.55, 0.8),
    voice_quality: Color::from_rgb(0.65, 0.9, 0.45),
};

impl Palette {
    fn color(&self, kind: Kind, plain: Color) -> Color {
        match kind {
            Kind::Plain => plain,
            Kind::Phoneme => self.phoneme,
            Kind::Stress => self.stress,
            Kind::Shift => self.shift,
            Kind::Transient => self.transient,
            Kind::Pause => self.pause,
            Kind::Bracket => self.bracket,
            Kind::Bank => self.bank,
            Kind::Comment => self.comment,
            Kind::Error => self.error,
            Kind::Pitch => self.pitch,
            Kind::Timing => self.timing,
            Kind::Scale => self.scale,
            Kind::Modulation => self.modulation,
            Kind::VoiceQuality => self.voice_quality,
        }
    }
}

// The highlighted text, split into lines of (column, kind) runs.
#[derive(Default)]
struct Cache {
    valid: bool,
    version: u32,
    bank: String,
    lines: Vec<String>,
    runs: Vec<Vec<(i64, Kind)>>,
}

impl Cache {
    // Re-highlight if the text (or bank) changed since the last call.
    // Returns true if it did.
    fn refresh(&mut self, edit: &Gd<TextEdit>, line: i32, bank: &str) -> bool {
        let fresh = self.valid
            && self.version == edit.get_version()
            && self.bank == bank
            && self.lines.len() == edit.get_line_count() as usize
            && self.lines.get(line as usize).is_some_and(|l| edit.get_line(line) == l.as_str());
        if fresh {
            return false;
        }
        let text = edit.get_text().to_string();
        self.lines = text.split('\n').map(str::to_owned).collect();
        self.runs = runs_by_line(&text, &spans(&text, bank), self.lines.len());
        self.version = edit.get_version();
        self.bank = bank.to_owned();
        self.valid = true;
        true
    }

    fn line(&self, line: i32, palette: &Palette, plain: Color) -> VarDictionary {
        let mut out = VarDictionary::new();
        for &(column, kind) in self.runs.get(line as usize).map(Vec::as_slice).unwrap_or_default() {
            out.set(column, &vdict! { "color" => palette.color(kind, plain) });
        }
        out
    }
}

// Split spans (character offsets into `text`) into per-line runs, merging
// neighbours of the same kind.
fn runs_by_line(text: &str, spans: &[HighlightSpan], line_count: usize) -> Vec<Vec<(i64, Kind)>> {
    let chars: Vec<char> = text.chars().collect();
    let mut runs: Vec<Vec<(i64, Kind)>> = vec![Vec::new(); line_count];
    let (mut line, mut column) = (0usize, 0i64);
    let mut pos = 0usize;
    for span in spans {
        let k = kind(span.class.as_deref());
        while pos < span.end {
            if pos >= span.start && chars[pos] != '\n' {
                let row = &mut runs[line];
                if row.last().is_none_or(|&(_, last)| last != k) {
                    row.push((column, k));
                }
            }
            if chars[pos] == '\n' {
                line += 1;
                column = 0;
            } else {
                column += 1;
            }
            pos += 1;
        }
    }
    runs
}

// Colours of the TextEdit being highlighted.
fn plain_color(edit: &Gd<TextEdit>) -> Color {
    edit.get_theme_color("font_color")
}

// Highlights klattsch phoneme strings in a TextEdit or CodeEdit (assign it
// to syntax_highlighter). Phonemes the bank lacks are shown in error_color.
#[derive(GodotClass)]
#[class(tool, base = SyntaxHighlighter)]
pub struct KlattschSyntaxHighlighter {
    // The bank to check phonemes against (empty: the default bank);
    // [bank=...] switches in the text take over from there.
    #[export]
    #[var(set = set_bank)]
    bank: GString,
    #[export_group(name = "Colors")]
    #[export]
    #[var(set = set_phoneme_color)]
    phoneme_color: Color,
    // ' and ! marks.
    #[export]
    #[var(set = set_stress_color)]
    stress_color: Color,
    // Pitch offsets on a phoneme: AA+20.
    #[export]
    #[var(set = set_shift_color)]
    shift_color: Color,
    // Transient pitch offsets: AA(+20).
    #[export]
    #[var(set = set_transient_color)]
    transient_color: Color,
    // , ; .
    #[export]
    #[var(set = set_pause_color)]
    pause_color: Color,
    // ( ) and engine-specific brackets like [OQ=0.6].
    #[export]
    #[var(set = set_bracket_color)]
    bracket_color: Color,
    #[export]
    #[var(set = set_bank_color)]
    bank_color: Color,
    #[export]
    #[var(set = set_comment_color)]
    comment_color: Color,
    // Phonemes the bank lacks, unknown banks, and unrecognised words.
    #[export]
    #[var(set = set_error_color)]
    error_color: Color,
    // Directives by family: b, [base=]; r, p, [rate=], [pause=]; s, [scale=];
    // v, w, m, n and vibrato/tremolo; h, t, g, [aspiration=], [tilt=],
    // [effort=], [voice=].
    #[export]
    #[var(set = set_pitch_color)]
    pitch_color: Color,
    #[export]
    #[var(set = set_timing_color)]
    timing_color: Color,
    #[export]
    #[var(set = set_scale_color)]
    scale_color: Color,
    #[export]
    #[var(set = set_modulation_color)]
    modulation_color: Color,
    #[export]
    #[var(set = set_voice_quality_color)]
    voice_quality_color: Color,
    cache: RefCell<Cache>,
    base: Base<SyntaxHighlighter>,
}

macro_rules! color_setters {
    ($($set:ident => $field:ident),* $(,)?) => {
        #[godot_api]
        impl KlattschSyntaxHighlighter {
            #[func]
            fn set_bank(&mut self, bank: GString) {
                self.bank = bank;
                self.base_mut().clear_highlighting_cache();
            }
            $(
                #[func]
                fn $set(&mut self, color: Color) {
                    self.$field = color;
                    self.base_mut().clear_highlighting_cache();
                }
            )*
        }
    };
}

color_setters! {
    set_phoneme_color => phoneme_color,
    set_stress_color => stress_color,
    set_shift_color => shift_color,
    set_transient_color => transient_color,
    set_pause_color => pause_color,
    set_bracket_color => bracket_color,
    set_bank_color => bank_color,
    set_comment_color => comment_color,
    set_error_color => error_color,
    set_pitch_color => pitch_color,
    set_timing_color => timing_color,
    set_scale_color => scale_color,
    set_modulation_color => modulation_color,
    set_voice_quality_color => voice_quality_color,
}

impl KlattschSyntaxHighlighter {
    fn palette(&self) -> Palette {
        Palette {
            phoneme: self.phoneme_color,
            stress: self.stress_color,
            shift: self.shift_color,
            transient: self.transient_color,
            pause: self.pause_color,
            bracket: self.bracket_color,
            bank: self.bank_color,
            comment: self.comment_color,
            error: self.error_color,
            pitch: self.pitch_color,
            timing: self.timing_color,
            scale: self.scale_color,
            modulation: self.modulation_color,
            voice_quality: self.voice_quality_color,
        }
    }
}

// Godot's per-line cache misses lines whose colour changed through an
// edit on another line; clear it once the new highlighting is in.
fn repaint_later(highlighter: Gd<impl Inherits<SyntaxHighlighter>>, mut edit: Gd<TextEdit>) {
    highlighter.upcast::<SyntaxHighlighter>().call_deferred("clear_highlighting_cache", &[]);
    edit.call_deferred("queue_redraw", &[]);
}

#[godot_api]
impl ISyntaxHighlighter for KlattschSyntaxHighlighter {
    fn init(base: Base<SyntaxHighlighter>) -> Self {
        let p = PALETTE;
        Self {
            bank: GString::new(),
            phoneme_color: p.phoneme,
            stress_color: p.stress,
            shift_color: p.shift,
            transient_color: p.transient,
            pause_color: p.pause,
            bracket_color: p.bracket,
            bank_color: p.bank,
            comment_color: p.comment,
            error_color: p.error,
            pitch_color: p.pitch,
            timing_color: p.timing,
            scale_color: p.scale,
            modulation_color: p.modulation,
            voice_quality_color: p.voice_quality,
            cache: RefCell::default(),
            base,
        }
    }

    fn get_line_syntax_highlighting(&self, line: i32) -> AnyDictionary {
        let Some(edit) = self.base().get_text_edit() else { return VarDictionary::new().upcast_any_dictionary() };
        let mut cache = self.cache.borrow_mut();
        if cache.refresh(&edit, line, &self.bank.to_string()) {
            repaint_later(self.to_gd(), edit.clone());
        }
        cache.line(line, &self.palette(), plain_color(&edit)).upcast_any_dictionary()
    }
}

// The script editor's highlighter for phoneme strings, with the default
// palette and bank. Registered by the klattsch editor plugin; pick it with
// Edit > Syntax Highlighter > klattsch phonemes.
#[derive(GodotClass)]
#[class(tool, init, base = EditorSyntaxHighlighter)]
pub struct KlattschEditorSyntaxHighlighter {
    cache: RefCell<Cache>,
    base: Base<EditorSyntaxHighlighter>,
}

#[godot_api]
impl IEditorSyntaxHighlighter for KlattschEditorSyntaxHighlighter {
    fn get_name(&self) -> GString {
        "klattsch phonemes".into()
    }

    fn get_supported_languages(&self) -> PackedStringArray {
        PackedStringArray::new()
    }

    fn create(&self) -> Option<Gd<EditorSyntaxHighlighter>> {
        Some(Self::new_gd().upcast())
    }

    fn get_line_syntax_highlighting(&self, line: i32) -> AnyDictionary {
        let Some(edit) = self.base().get_text_edit() else { return VarDictionary::new().upcast_any_dictionary() };
        let mut cache = self.cache.borrow_mut();
        if cache.refresh(&edit, line, "") {
            repaint_later(self.to_gd(), edit.clone());
        }
        cache.line(line, &PALETTE, plain_color(&edit)).upcast_any_dictionary()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_split_by_line_and_merge() {
        let text = "HH AH' /* two\nlines */ ZZ\n# note";
        let spans = spans(text, "");
        let runs = runs_by_line(text, &spans, 3);
        assert_eq!(
            runs[0],
            vec![(0, Kind::Phoneme), (2, Kind::Plain), (3, Kind::Phoneme), (5, Kind::Stress), (6, Kind::Plain), (7, Kind::Comment)]
        );
        assert_eq!(runs[1], vec![(0, Kind::Comment), (8, Kind::Plain), (9, Kind::Error)]);
        assert_eq!(runs[2], vec![(0, Kind::Comment)]);
    }

    #[test]
    fn bank_checks_phonemes() {
        // klatt1980-en has no Japanese-only codes; an unknown bank is an error.
        let kinds = |s: &str| spans(s, "").iter().map(|sp| kind(sp.class.as_deref())).collect::<Vec<_>>();
        assert_eq!(kinds("AA [bank=nope]"), vec![Kind::Phoneme, Kind::Plain, Kind::Error]);
        assert_eq!(kinds("b220 r90"), vec![Kind::Pitch, Kind::Pitch, Kind::Plain, Kind::Timing, Kind::Timing]);
    }
}
