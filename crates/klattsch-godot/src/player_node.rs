// KlattschPlayer: the node most game code uses. It drives a stock
// AudioStreamPlayer / 2D / 3D (the `target`, or an internal
// AudioStreamPlayer) through a KlattschStream, speaks utterances by id, and
// emits timing signals for what the listener hears.
//
// Timing: each utterance is sent tagged with its id. The playback publishes
// which tag voice 0 is playing and the exact frame its time zero fell on, so
// the node knows every utterance's real start (after interrupts and gapless
// appends alike). Each frame it estimates the frame being heard as
//   frames rendered + time since the last mix - output latency
// (the first term frozen while paused, and never moving backwards), and
// emits the signals that position has passed. Nothing is emitted from the
// audio thread.

use std::collections::VecDeque;

use godot::classes::{AudioServer, AudioStreamPlayer, INode, Node};
use godot::prelude::*;
use klattsch_core::{compile_string, BankRef, CompileOptions, CompileResult, EnglishOptions, WordSource};

use crate::klattsch::{self, Klattsch};
use crate::stream::KlattschStream;
use crate::utterance::KlattschUtterance;

#[derive(GodotConvert, Var, Export, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[godot(via = i64)]
pub enum QueueMode {
    // A new utterance cuts off the current one.
    #[default]
    Interrupt = 0,
    // A new utterance waits for the ones before it, then follows without a gap.
    Queue = 1,
}

struct PhraseInfo {
    t_start_ms: f64,
    t_end_ms: f64,
    phoneme: GString,
    src_start: i64,
    src_end: i64,
}

struct Record {
    id: i64,
    total_ms: f64,
    phrases: Vec<PhraseInfo>,
    // Kept until sent (queue mode sends one at a time).
    pending: Option<CompileResult>,
    // The frame the utterance's time zero fell on, once it is playing.
    start: Option<f64>,
    // Where a later utterance (or stop) cut it off.
    cut: f64,
    started: bool,
    next_phrase: usize,
    done: bool,
}

enum Event {
    Started(i64),
    Phrase(i64, i64, GString, i64, i64),
    Finished(i64),
    Error(String),
}

#[derive(GodotClass)]
#[class(base = Node)]
pub struct KlattschPlayer {
    // An AudioStreamPlayer, AudioStreamPlayer2D or AudioStreamPlayer3D to
    // play through; empty creates an internal AudioStreamPlayer child.
    #[export]
    target: NodePath,
    // Created if empty (or taken from the target) and assigned to the target.
    #[export]
    stream: Option<Gd<KlattschStream>>,
    // Play the target when ready, so the stream runs silently and say()
    // starts at once.
    #[export]
    autostart: bool,
    #[export]
    queue_mode: QueueMode,
    // say(): add pitch movement (sentence melody) to English.
    #[export]
    intonation: bool,
    dictionary_requested: bool,
    audio: Option<Gd<Node>>,
    records: VecDeque<Record>,
    events: Vec<Event>,
    next_id: i64,
    last_frames: u64,
    heard: f64,
    mix_rate: f32,
    current_phrase: i64,
    reported_dead: bool,
    base: Base<Node>,
}

#[godot_api]
impl INode for KlattschPlayer {
    fn init(base: Base<Node>) -> Self {
        Self {
            target: NodePath::default(),
            stream: None,
            autostart: true,
            queue_mode: QueueMode::Interrupt,
            intonation: true,
            dictionary_requested: false,
            audio: None,
            records: VecDeque::new(),
            events: Vec::new(),
            next_id: 1,
            last_frames: 0,
            heard: 0.0,
            mix_rate: 0.0,
            current_phrase: -1,
            reported_dead: false,
            base,
        }
    }

    fn ready(&mut self) {
        let audio = if self.target.is_empty() {
            let mut player = AudioStreamPlayer::new_alloc();
            player.set_name("AudioStreamPlayer");
            self.base_mut().add_child(&player);
            player.upcast::<Node>()
        } else {
            let Some(node) = self.base().get_node_or_null(&self.target) else {
                self.events.push(Event::Error(format!("KlattschPlayer: no node at target {}", self.target)));
                return;
            };
            let ok = ["AudioStreamPlayer", "AudioStreamPlayer2D", "AudioStreamPlayer3D"]
                .iter()
                .any(|c| node.is_class(*c));
            if !ok {
                let class = node.get_class();
                self.events.push(Event::Error(format!(
                    "KlattschPlayer: target {} is a {class}, not an audio stream player",
                    self.target
                )));
                return;
            }
            // Process (and pause) together with the player we drive.
            let mode = node.get_process_mode();
            self.base_mut().set_process_mode(mode);
            node
        };
        if self.stream.is_none() {
            self.stream = audio.get("stream").try_to::<Gd<KlattschStream>>().ok();
        }
        let stream = self.stream.get_or_insert_with(KlattschStream::new_gd).clone();
        let mut audio = audio;
        audio.set("stream", &stream.to_variant());
        self.audio = Some(audio);
        if self.autostart {
            self.play();
        }
    }

    fn process(&mut self, _delta: f64) {
        self.update();
        self.flush();
    }
}

impl KlattschPlayer {
    fn play(&mut self) {
        if let Some(audio) = self.audio.as_mut()
            && audio.is_inside_tree()
        {
            audio.call("play", &[]);
            self.mix_rate = AudioServer::singleton().get_mix_rate();
            self.last_frames = 0;
            self.heard = 0.0;
        }
    }

    // The stream's voice (compile options), or klattsch's defaults.
    fn voice(&self) -> CompileOptions {
        self.stream.as_ref().map(|s| s.bind().compile_options()).unwrap_or_default()
    }

    fn compile_and_say(&mut self, phonemes: &str, opts: &CompileOptions, what: &str) -> i64 {
        match compile_string(phonemes, opts) {
            Ok(result) => self.enqueue(result),
            Err(e) => {
                self.events.push(Event::Error(format!("KlattschPlayer.{what}: {e}")));
                -1
            }
        }
    }

    fn stream_active(&self) -> bool {
        self.stream.as_ref().is_some_and(|s| s.bind().clock().is_some_and(|c| c.playing))
    }

    fn enqueue(&mut self, result: CompileResult) -> i64 {
        if self.audio.is_none() {
            self.events.push(Event::Error("KlattschPlayer: say() before the node is ready, or without a valid target".into()));
            return -1;
        }
        for w in &result.warnings {
            godot_warn!("KlattschPlayer: {w}");
        }
        if !self.stream_active() {
            self.play();
        }
        let id = self.next_id;
        self.next_id += 1;
        let phrases = result.voices[0]
            .phrases
            .iter()
            .map(|p| PhraseInfo {
                t_start_ms: p.t_start_ms,
                t_end_ms: p.t_end_ms,
                phoneme: p.phoneme.as_deref().unwrap_or("").into(),
                src_start: p.src_start as i64,
                src_end: p.src_end as i64,
            })
            .collect();
        let mut record = Record {
            id,
            total_ms: result.total_ms,
            phrases,
            pending: None,
            start: None,
            cut: f64::INFINITY,
            started: false,
            next_phrase: 0,
            done: false,
        };
        match self.queue_mode {
            QueueMode::Interrupt => {
                let stream = self.stream.clone().unwrap();
                if !stream.bind().speak_result(&result, true, -1.0, id as u64) {
                    self.events.push(Event::Error("KlattschPlayer: could not speak (command queue full?)".into()));
                    return -1;
                }
                // Waiting utterances will never play; playing ones are cut
                // when this one starts.
                for r in self.records.iter_mut().filter(|r| !r.done && r.pending.is_some()) {
                    r.done = true;
                    self.events.push(Event::Finished(r.id));
                }
            }
            QueueMode::Queue => record.pending = Some(result),
        }
        self.records.push_back(record);
        self.pump();
        id
    }

    // Queue mode: hand the next waiting utterance to the stream once every
    // utterance sent before it is playing (the core holds one appended
    // schedule per voice).
    fn pump(&mut self) {
        let Some(stream) = self.stream.clone() else { return };
        let mut ready = true;
        for r in self.records.iter_mut().filter(|r| !r.done) {
            match r.pending.take() {
                None => ready &= r.start.is_some(),
                Some(result) => {
                    // Sent, it waits in the core; otherwise try again next frame.
                    if !(ready && stream.bind().speak_result(&result, false, -1.0, r.id as u64)) {
                        r.pending = Some(result);
                    }
                    return;
                }
            }
        }
    }

    fn finish_all(&mut self) {
        for r in self.records.iter_mut().filter(|r| !r.done) {
            r.done = true;
            self.events.push(Event::Finished(r.id));
        }
        self.records.clear();
        self.current_phrase = -1;
    }

    fn update(&mut self) {
        let Some(stream) = self.stream.clone() else { return };
        let Some(c) = stream.bind().clock() else { return };
        if c.dead && !self.reported_dead {
            self.reported_dead = true;
            self.events.push(Event::Error("KlattschPlayer: the audio playback failed and is silent".into()));
        }
        let server = AudioServer::singleton();
        if self.mix_rate != 0.0 && server.get_mix_rate() != self.mix_rate {
            // The output device changed: rebuild the synth at the new rate.
            self.finish_all();
            self.play();
            return;
        }
        let rate = c.rate;
        let frames = c.clock.frames;
        if frames < self.last_frames {
            // A new playback (play() was called): its clock starts again.
            self.finish_all();
            self.heard = 0.0;
        }

        // The utterance voice 0 is playing has started: note its start frame;
        // it cuts off everything before it.
        if c.clock.tag != 0 {
            let tag = c.clock.tag as i64;
            if let Some(i) = self.records.iter().position(|r| r.id == tag && r.start.is_none()) {
                let s = c.clock.start;
                self.records[i].start = Some(s);
                for r in self.records.iter_mut().take(i).filter(|r| !r.done) {
                    if r.start.is_some() {
                        r.cut = r.cut.min(s);
                    } else {
                        // Sent but never played (replaced before it began).
                        r.done = true;
                        self.events.push(Event::Finished(r.id));
                    }
                }
            }
        }

        // The frame the listener hears now.
        let mut heard = frames as f64;
        if frames != self.last_frames {
            heard += server.get_time_since_last_mix() * rate;
        }
        heard -= server.get_output_latency() * rate;
        self.heard = self.heard.max(heard);
        self.last_frames = frames;
        let heard = self.heard;

        self.current_phrase = -1;
        for r in self.records.iter_mut().filter(|r| !r.done) {
            let Some(s) = r.start else { continue };
            let end = (s + r.total_ms * rate / 1000.0).min(r.cut);
            if heard < s {
                continue;
            }
            if !r.started && s < end {
                r.started = true;
                self.events.push(Event::Started(r.id));
            }
            let t_ms = (heard - s) / rate * 1000.0;
            while let Some(p) = r.phrases.get(r.next_phrase) {
                if p.t_start_ms > t_ms || s + p.t_start_ms * rate / 1000.0 >= end {
                    break;
                }
                self.events.push(Event::Phrase(r.id, r.next_phrase as i64, p.phoneme.clone(), p.src_start, p.src_end));
                r.next_phrase += 1;
            }
            if heard >= end {
                r.done = true;
                self.events.push(Event::Finished(r.id));
            } else if r.next_phrase > 0 && t_ms < r.phrases[r.next_phrase - 1].t_end_ms {
                self.current_phrase = r.next_phrase as i64 - 1;
            }
        }
        while self.records.front().is_some_and(|r| r.done) {
            self.records.pop_front();
        }
        self.pump();
    }

    // Emit collected signals. Handlers may call back into this node.
    fn flush(&mut self) {
        for event in std::mem::take(&mut self.events) {
            match event {
                Event::Started(id) => self.signals().utterance_started().emit(id),
                Event::Phrase(id, index, phoneme, a, b) => {
                    self.signals().phrase_started().emit(id, index, &phoneme, a, b)
                }
                Event::Finished(id) => self.signals().utterance_finished().emit(id),
                Event::Error(message) => {
                    godot_error!("{message}");
                    self.signals().error().emit(&GString::from(message.as_str()))
                }
            }
        }
    }
}

#[godot_api]
impl KlattschPlayer {
    #[constant]
    const INTERRUPT: i64 = 0;
    #[constant]
    const QUEUE: i64 = 1;

    #[signal]
    fn utterance_started(id: i64);
    // `index` into the utterance's voice-0 phrases; `phoneme` is empty for
    // pauses; src_start/src_end locate it in the phoneme string.
    #[signal]
    fn phrase_started(id: i64, index: i64, phoneme: GString, src_start: i64, src_end: i64);
    // Exactly once per id: when it has been heard to the end, cut off, or
    // dropped (stop, or interrupted before it began).
    #[signal]
    fn utterance_finished(id: i64);
    #[signal]
    fn error(message: GString);

    // Speak English text with the stream's voice. Returns the utterance id,
    // or -1. Before the CMU dictionary has loaded (Klattsch.load_dictionary,
    // started here if nobody has), words come from the lexicon and the
    // letter-to-sound rules only, and `error` names those sounded out.
    #[func]
    fn say(&mut self, text: GString) -> i64 {
        if !klattsch::dictionary_ready() && !self.dictionary_requested {
            self.dictionary_requested = true;
            Klattsch::singleton().bind_mut().load_dictionary(klattsch::DEFAULT_DICTIONARY.into());
        }
        let opts = EnglishOptions { intonation: self.intonation, ..EnglishOptions::default() };
        let (lexicon, ready) = klattsch::lexicon();
        let spoken = klattsch_core::english_to_phonemes(&text.to_string(), &lexicon, &opts);
        if !ready {
            let guessed: Vec<&str> = spoken
                .words
                .iter()
                .filter(|w| matches!(w.source, WordSource::LetterToSound | WordSource::Skipped))
                .map(|w| w.text.as_str())
                .collect();
            if !guessed.is_empty() {
                self.events.push(Event::Error(format!(
                    "KlattschPlayer.say: the dictionary is not loaded yet; sounded out: {}",
                    guessed.join(", ")
                )));
            }
        }
        let voice = self.voice();
        self.compile_and_say(&spoken.phonemes, &voice, "say")
    }

    // Speak kana or romaji with a Japanese bank: the stream's bank if it is
    // a ja-* bank, otherwise ja-mokhtari-2000. Returns the id, or -1.
    #[func]
    fn say_japanese(&mut self, text: GString) -> i64 {
        let Some(phonemes) = klattsch::japanese(&text.to_string()) else {
            self.events.push(Event::Error(format!("KlattschPlayer.say_japanese: not kana or romaji: {text}")));
            return -1;
        };
        let mut voice = self.voice();
        if !matches!(&voice.bank, Some(BankRef::Name(n)) if n.starts_with("ja")) {
            voice.bank = Some(BankRef::Name("ja-mokhtari-2000".into()));
        }
        self.compile_and_say(&phonemes, &voice, "say_japanese")
    }

    // Speak a klattsch phoneme string with the stream's voice. Returns the
    // utterance id, or -1.
    #[func]
    fn say_phonemes(&mut self, phonemes: GString) -> i64 {
        let voice = self.voice();
        self.compile_and_say(&phonemes.to_string(), &voice, "say_phonemes")
    }

    // Speak a compiled utterance. Returns the utterance id, or -1.
    #[func]
    fn say_utterance(&mut self, utterance: Gd<KlattschUtterance>) -> i64 {
        let result = utterance.bind().result.clone();
        self.enqueue(result)
    }

    // Stop speaking now (a short fade); every pending id finishes.
    #[func]
    fn stop(&mut self) {
        if let Some(stream) = self.stream.clone() {
            stream.bind().silence(-1);
        }
        self.finish_all();
    }

    #[func]
    fn is_speaking(&self) -> bool {
        self.records.iter().any(|r| !r.done)
    }

    // The index of the phrase being heard (see phrase_started), or -1.
    #[func]
    fn get_current_phrase(&self) -> i64 {
        self.current_phrase
    }

    // Glide a synth parameter live (see KlattschStream.set_param).
    #[func]
    fn set_param(
        &self,
        param: StringName,
        value: f64,
        #[opt(default = 50.0)] glide_ms: f64,
        #[opt(default = -1)] voice: i64,
    ) -> bool {
        self.stream.as_ref().is_some_and(|s| s.bind().set_param(param, value, glide_ms, voice))
    }

    // The audio stream player this node drives.
    #[func]
    fn get_audio_player(&self) -> Option<Gd<Node>> {
        self.audio.clone()
    }
}
