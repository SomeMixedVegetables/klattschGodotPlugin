// Editor extras, active whenever the addon is installed (gdext registers
// the plugin itself; there is nothing to enable):
//   - the script editor's "klattsch phonemes" syntax highlighter;
//   - an audition panel at the top of the Inspector for a KlattschPlayer or
//     KlattschStream: type English, Japanese or phonemes, press Play, and
//     hear the line in that node's voice without running the game.
//
// The panel pre-renders the line (Klattsch.render) and plays it on an
// AudioStreamPlayer the plugin owns, so it keeps playing while the
// Inspector rebuilds. KlattschStream and KlattschPlayer are runtime classes
// (placeholders in the editor), so their properties are read with get().

use std::cell::RefCell;

use godot::classes::text_edit::LineWrappingMode;
use godot::classes::text_server::AutowrapMode;
use godot::classes::{
    AudioServer, AudioStreamPlayer, Button, CodeEdit, EditorInspectorPlugin, EditorInterface, EditorPlugin,
    HBoxContainer, IEditorInspectorPlugin, IEditorPlugin, IVBoxContainer, InputEvent, InputEventKey, Label, Node,
    OptionButton, VBoxContainer,
};
use godot::global::Key;
use godot::prelude::*;
use klattsch_core::{compile_string, render_mix, BankRef, CompileOptions, EnglishOptions, WordSource, DEFAULT_BANK};

use crate::highlighter::{KlattschEditorSyntaxHighlighter, KlattschSyntaxHighlighter};
use crate::klattsch::{self, Klattsch};
use crate::sample::KlattschSample;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    English = 0,
    Japanese = 1,
    Phonemes = 2,
}

impl Mode {
    fn from_index(i: i64) -> Self {
        match i {
            1 => Mode::Japanese,
            2 => Mode::Phonemes,
            _ => Mode::English,
        }
    }
}

// The panel's text and mode, kept for the editor session (the Inspector
// makes a new panel every time it shows a node).
struct Saved {
    mode: Mode,
    texts: [String; 3],
}

thread_local! {
    static SAVED: RefCell<Saved> = RefCell::new(Saved {
        mode: Mode::English,
        texts: ["Hello! This is how I sound.".into(), "こんにちは".into(), String::new()],
    });
}

const JAPANESE_BANK: &str = "ja-mokhtari-2000";

// KlattschStream's Voice properties, named as klattsch::compile_options reads them.
const VOICE_KEYS: [&str; 11] = [
    "bank",
    "base_f0",
    "rate",
    "scale",
    "vibrato_depth",
    "vibrato_rate",
    "tremolo_depth",
    "tremolo_rate",
    "aspiration",
    "tilt",
    "effort",
];

struct Voice {
    options: CompileOptions,
    intonation: bool,
}

impl Voice {
    fn bank(&self) -> String {
        match &self.options.bank {
            Some(BankRef::Name(name)) => name.clone(),
            _ => DEFAULT_BANK.into(),
        }
    }
}

// The voice a KlattschPlayer or KlattschStream would speak with: the
// player's stream, else its target's KlattschStream, else klattsch's
// defaults (what the player creates at run time).
fn voice_of(target: &Gd<Object>) -> Voice {
    let mut intonation = true;
    let stream = if target.is_class("KlattschPlayer") {
        intonation = target.get("intonation").try_to::<bool>().unwrap_or(true);
        target.get("stream").try_to::<Gd<Object>>().ok().or_else(|| {
            let node = target.clone().try_cast::<Node>().ok()?;
            let path = node.get("target").try_to::<NodePath>().ok().filter(|p| !p.is_empty())?;
            let stream = node.get_node_or_null(&path)?.get("stream").try_to::<Gd<Object>>().ok()?;
            stream.is_class("KlattschStream").then_some(stream)
        })
    } else {
        Some(target.clone())
    };
    let mut values = VarDictionary::new();
    if let Some(stream) = stream {
        for key in VOICE_KEYS {
            let value = stream.get(key);
            if !value.is_nil() {
                values.set(key, &value);
            }
        }
    }
    Voice { options: klattsch::compile_options(&values), intonation }
}

// Text in `mode` as phonemes for `voice`: the phonemes, the options to
// compile them with, and notes for the status line.
fn to_phonemes(mode: Mode, text: &str, voice: &Voice) -> Result<(String, CompileOptions, Vec<String>), String> {
    let mut options = voice.options.clone();
    let mut notes = Vec::new();
    let phonemes = match mode {
        Mode::Phonemes => text.to_owned(),
        Mode::English => {
            let spoken = klattsch::english(text, &EnglishOptions { intonation: voice.intonation, ..Default::default() });
            let guessed: Vec<&str> = spoken
                .words
                .iter()
                .filter(|w| matches!(w.source, WordSource::LetterToSound | WordSource::Skipped))
                .map(|w| w.text.as_str())
                .collect();
            if !guessed.is_empty() {
                notes.push(format!("sounded out: {}", guessed.join(", ")));
            }
            spoken.phonemes
        }
        Mode::Japanese => {
            let phonemes = klattsch::japanese(text).ok_or("Not kana or romaji.")?;
            // As KlattschPlayer.say_japanese: a ja-* bank, or the default one.
            if !voice.bank().starts_with("ja") {
                options.bank = Some(BankRef::Name(JAPANESE_BANK.into()));
            }
            phonemes
        }
    };
    Ok((phonemes, options, notes))
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Pending {
    #[default]
    None,
    Play,
    Convert,
}

// The Inspector panel.
#[derive(GodotClass)]
#[class(tool, internal, init, base = VBoxContainer)]
pub struct KlattschAuditionPanel {
    target: Option<InstanceId>,
    player: Option<Gd<AudioStreamPlayer>>,
    mode: Option<Gd<OptionButton>>,
    text: Option<Gd<CodeEdit>>,
    convert: Option<Gd<Button>>,
    status: Option<Gd<Label>>,
    highlighter: Option<Gd<KlattschSyntaxHighlighter>>,
    // Waiting for the CMU dictionary.
    pending: Pending,
    base: Base<VBoxContainer>,
}

#[godot_api]
impl IVBoxContainer for KlattschAuditionPanel {
    fn ready(&mut self) {
        self.base_mut().set_process(false);
    }

    fn process(&mut self, _delta: f64) {
        if klattsch::dictionary_ready() {
            self.base_mut().set_process(false);
            self.run_pending();
        }
    }
}

#[godot_api]
impl KlattschAuditionPanel {
    #[func]
    fn _on_mode(&mut self, index: i64) {
        let mode = Mode::from_index(index);
        let text = SAVED.with_borrow_mut(|s| {
            s.mode = mode;
            s.texts[mode as usize].clone()
        });
        if let Some(edit) = self.text.as_mut() {
            edit.set_text(&text);
        }
        self.show_mode();
    }

    #[func]
    fn _on_text_changed(&mut self) {
        let Some(edit) = self.text.as_ref() else { return };
        let text = edit.get_text().to_string();
        SAVED.with_borrow_mut(|s| {
            let mode = s.mode as usize;
            s.texts[mode] = text;
        });
    }

    // Ctrl+Enter (Cmd+Enter) plays.
    #[func]
    fn _on_text_input(&mut self, event: Gd<InputEvent>) {
        let Ok(key) = event.try_cast::<InputEventKey>() else { return };
        if key.is_pressed()
            && !key.is_echo()
            && matches!(key.get_keycode(), Key::ENTER | Key::KP_ENTER)
            && key.is_command_or_control_pressed()
        {
            if let Some(mut edit) = self.text.clone() {
                edit.accept_event();
            }
            self._on_play();
        }
    }

    #[func]
    fn _on_play(&mut self) {
        self.pending = Pending::Play;
        self.run_pending();
    }

    #[func]
    fn _on_stop(&mut self) {
        self.pending = Pending::None;
        if let Some(player) = self.player.as_mut() {
            player.stop();
        }
        self.set_status("");
    }

    #[func]
    fn _on_convert(&mut self) {
        self.pending = Pending::Convert;
        self.run_pending();
    }
}

impl KlattschAuditionPanel {
    pub fn create(target: &Gd<Object>, player: Gd<AudioStreamPlayer>) -> Gd<Self> {
        let mut panel = Self::new_alloc();
        {
            let mut p = panel.bind_mut();
            p.target = Some(target.instance_id());
            p.player = Some(player);
            p.build();
        }
        panel
    }

    fn build(&mut self) {
        let me = self.to_gd();
        let on = |method: &str| Callable::from_object_method(&me, method);
        let saved_mode = SAVED.with_borrow(|s| s.mode);

        let mut title = Label::new_alloc();
        title.set_text("Audition");
        title.set_h_size_flags(godot::classes::control::SizeFlags::EXPAND_FILL);

        let mut mode = OptionButton::new_alloc();
        for name in ["English", "Japanese", "Phonemes"] {
            mode.add_item(name);
        }
        mode.select(saved_mode as i32);
        mode.set_tooltip_text("What the text below is written in.");
        mode.connect("item_selected", &on("_on_mode"));

        let mut convert = Button::new_alloc();
        convert.set_text("To Phonemes");
        convert.set_tooltip_text("Turn the text into a klattsch phoneme string to edit.");
        convert.connect("pressed", &on("_on_convert"));

        let mut play = Button::new_alloc();
        play.set_text("Play");
        play.set_tooltip_text("Speak the text in this voice (Ctrl+Enter).");
        play.connect("pressed", &on("_on_play"));

        let mut stop = Button::new_alloc();
        stop.set_text("Stop");
        stop.connect("pressed", &on("_on_stop"));

        let mut row = HBoxContainer::new_alloc();
        for control in [title.upcast::<Node>(), mode.clone().upcast(), convert.clone().upcast(), play.upcast(), stop.upcast()] {
            row.add_child(&control);
        }

        let mut text = CodeEdit::new_alloc();
        text.set_custom_minimum_size(Vector2::new(0.0, 80.0));
        text.set_line_wrapping_mode(LineWrappingMode::BOUNDARY);
        text.set_text(&SAVED.with_borrow(|s| s.texts[saved_mode as usize].clone()));
        text.connect("text_changed", &on("_on_text_changed"));
        text.connect("gui_input", &on("_on_text_input"));

        let mut status = Label::new_alloc();
        status.set_autowrap_mode(AutowrapMode::WORD_SMART);
        status.set_modulate(Color::from_rgba(1.0, 1.0, 1.0, 0.7));

        let mut base = self.base_mut();
        base.add_child(&row);
        base.add_child(&text);
        base.add_child(&status);
        drop(base);

        self.mode = Some(mode);
        self.convert = Some(convert);
        self.text = Some(text);
        self.status = Some(status);
        self.highlighter = Some(KlattschSyntaxHighlighter::new_gd());
        self.show_mode();
    }

    fn mode(&self) -> Mode {
        SAVED.with_borrow(|s| s.mode)
    }

    fn target(&self) -> Option<Gd<Object>> {
        self.target.and_then(|id| Gd::try_from_instance_id(id).ok())
    }

    // Highlighting and the convert button for the current mode.
    fn show_mode(&mut self) {
        let mode = self.mode();
        let bank = self.target().map(|t| voice_of(&t).bank()).unwrap_or_else(|| DEFAULT_BANK.into());
        if let Some(h) = self.highlighter.as_mut() {
            h.set("bank", &bank.to_variant());
        }
        let highlighter = (mode == Mode::Phonemes).then(|| self.highlighter.clone()).flatten();
        if let Some(edit) = self.text.as_mut() {
            edit.set_syntax_highlighter(highlighter.as_ref());
            edit.set_placeholder(match mode {
                Mode::English => "English text to speak",
                Mode::Japanese => "Kana or romaji to speak",
                Mode::Phonemes => "A klattsch phoneme string (try To Phonemes from English)",
            });
        }
        if let Some(convert) = self.convert.as_mut() {
            convert.set_visible(mode != Mode::Phonemes);
        }
    }

    fn set_status(&mut self, text: &str) {
        if let Some(status) = self.status.as_mut() {
            status.set_text(text);
        }
    }

    fn run_pending(&mut self) {
        let action = self.pending;
        if action == Pending::None {
            return;
        }
        if self.mode() == Mode::English && !klattsch::dictionary_ready() {
            if !Klattsch::singleton().bind_mut().load_dictionary(klattsch::DEFAULT_DICTIONARY.into()) {
                self.pending = Pending::None;
                self.set_status(&format!("Cannot read the CMU dictionary at {}.", klattsch::DEFAULT_DICTIONARY));
                return;
            }
            self.set_status("Loading the CMU dictionary...");
            self.base_mut().set_process(true);
            return;
        }
        self.pending = Pending::None;
        match action {
            Pending::Play => self.play(),
            Pending::Convert => self.convert(),
            Pending::None => {}
        }
    }

    fn line_input(&self) -> Option<(Mode, String, Voice)> {
        let target = self.target()?;
        let text = self.text.as_ref()?.get_text().to_string();
        Some((self.mode(), text, voice_of(&target)))
    }

    fn play(&mut self) {
        let Some((mode, text, voice)) = self.line_input() else { return };
        if text.trim().is_empty() {
            self.set_status("Nothing to say.");
            return;
        }
        let (phonemes, options, mut notes) = match to_phonemes(mode, &text, &voice) {
            Ok(p) => p,
            Err(e) => return self.set_status(&e),
        };
        let result = match compile_string(&phonemes, &options) {
            Ok(r) => r,
            Err(e) => return self.set_status(&e.to_string()),
        };
        let rate = f64::from(AudioServer::singleton().get_mix_rate());
        let samples = match render_mix(&result, rate) {
            Ok(s) => s,
            Err(e) => return self.set_status(&e.to_string()),
        };
        if let Some(player) = self.player.as_mut() {
            player.set_stream(&KlattschSample::create(samples, rate));
            player.play();
        }
        let bank = match &options.bank {
            Some(BankRef::Name(name)) => name.clone(),
            _ => DEFAULT_BANK.into(),
        };
        notes.extend(result.warnings.iter().cloned());
        let mut status = format!("{:.2} s, {bank}", result.total_ms / 1000.0);
        for note in notes {
            status += &format!("\n{note}");
        }
        self.set_status(&status);
    }

    // English or Japanese text to an editable phoneme string (with a bank
    // switch if the voice's bank cannot speak it).
    fn convert(&mut self) {
        let Some((mode, text, voice)) = self.line_input() else { return };
        let (mut phonemes, options, notes) = match to_phonemes(mode, &text, &voice) {
            Ok(p) => p,
            Err(e) => return self.set_status(&e),
        };
        if options.bank != voice.options.bank
            && let Some(BankRef::Name(bank)) = &options.bank
        {
            phonemes = format!("[bank={bank}] {phonemes}");
        }
        SAVED.with_borrow_mut(|s| s.texts[Mode::Phonemes as usize] = phonemes);
        if let Some(mode) = self.mode.as_mut() {
            mode.select(Mode::Phonemes as i32);
        }
        self._on_mode(Mode::Phonemes as i64);
        self.set_status(&notes.join("\n"));
    }
}

// Adds the audition panel for KlattschPlayer and KlattschStream (when they
// are what the Inspector shows, not a sub-resource of it).
#[derive(GodotClass)]
#[class(tool, init, internal, base = EditorInspectorPlugin)]
pub struct KlattschAuditionInspector {
    player: Option<Gd<AudioStreamPlayer>>,
    base: Base<EditorInspectorPlugin>,
}

#[godot_api]
impl IEditorInspectorPlugin for KlattschAuditionInspector {
    fn can_handle(&self, object: Option<Gd<Object>>) -> bool {
        object.is_some_and(|o| o.is_class("KlattschPlayer") || o.is_class("KlattschStream"))
    }

    fn parse_begin(&mut self, object: Option<Gd<Object>>) {
        let (Some(object), Some(player)) = (object, self.player.clone()) else { return };
        let edited = EditorInterface::singleton().get_inspector().and_then(|i| i.get_edited_object());
        if edited.is_none_or(|e| e != object) {
            return;
        }
        let panel = KlattschAuditionPanel::create(&object, player);
        self.base_mut().add_custom_control(&panel);
    }
}

// Registers the highlighter and the audition panel.
#[derive(GodotClass)]
#[class(tool, init, base = EditorPlugin)]
pub struct KlattschEditorPlugin {
    highlighter: Option<Gd<KlattschEditorSyntaxHighlighter>>,
    inspector: Option<Gd<KlattschAuditionInspector>>,
    player: Option<Gd<AudioStreamPlayer>>,
    base: Base<EditorPlugin>,
}

#[godot_api]
impl IEditorPlugin for KlattschEditorPlugin {
    fn enter_tree(&mut self) {
        let player = AudioStreamPlayer::new_alloc();
        self.base_mut().add_child(&player);
        let mut inspector = KlattschAuditionInspector::new_gd();
        inspector.bind_mut().player = Some(player.clone());
        self.base_mut().add_inspector_plugin(&inspector);
        let highlighter = KlattschEditorSyntaxHighlighter::new_gd();
        if let Some(mut editor) = EditorInterface::singleton().get_script_editor() {
            editor.register_syntax_highlighter(&highlighter);
        }
        self.player = Some(player);
        self.inspector = Some(inspector);
        self.highlighter = Some(highlighter);
    }

    fn exit_tree(&mut self) {
        if let Some(inspector) = self.inspector.take() {
            self.base_mut().remove_inspector_plugin(&inspector);
        }
        if let Some(highlighter) = self.highlighter.take()
            && let Some(mut editor) = EditorInterface::singleton().get_script_editor()
        {
            editor.unregister_syntax_highlighter(&highlighter);
        }
        // Let go of the KlattschSample now: if the audio server still holds
        // its playback when the editor shuts down, it frees it after this
        // library is gone and the editor crashes.
        if let Some(mut player) = self.player.take() {
            player.stop();
            player.set_stream(Option::<&Gd<godot::classes::AudioStream>>::None);
            player.queue_free();
        }
    }
}
