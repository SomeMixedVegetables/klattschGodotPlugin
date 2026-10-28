# klattsch for Godot

A small parallel-formant speech synthesizer for Godot 4.7+, built from a
bit-exact Rust port of [klattsch](https://github.com/tgies/klattsch). It
speaks English, Japanese and klattsch phoneme strings in real time through
Godot's own audio players (2D and 3D included), with live control of every
synth parameter and signals timed to what the player hears.

Windows (x86_64), Linux (x86_64) and macOS (Apple Silicon and Intel).

## Install

1. Unzip the release into your project folder, so it contains
   `addons/klattsch/` (or copy that folder into your project's `addons/`).
2. Restart the editor (GDExtensions load at startup).

That's all: exported games include the CMU dictionary
(`res://addons/klattsch/cmudict.dict`) automatically.

macOS: if the editor refuses to load the library from a downloaded zip,
remove the quarantine flag first:

```
xattr -dr com.apple.quarantine addons/klattsch
```

## Quick start

```gdscript
@onready var voice: KlattschPlayer = $KlattschPlayer  # target: an AudioStreamPlayer3D, or empty

func _ready() -> void:
	Klattsch.load_dictionary()           # parses in the background
	voice.phrase_started.connect(func(id, i, phoneme, a, b): print(phoneme))
	voice.say("Halt! Who goes there?")

func _on_alarm() -> void:
	voice.set_param(&"F0", 220.0, 80.0)  # live pitch glide
```

- `KlattschPlayer` (node): `say`, `say_japanese`, `say_phonemes`,
  `say_utterance`, interrupt or queue mode, `stop`, `set_param`; signals
  `utterance_started`, `phrase_started`, `utterance_finished`, `error`.
- `KlattschStream` (AudioStream): the live voice, usable with any stock
  player; Voice properties (bank, pitch, rate, ...), `speak`, `set_param`,
  `set_params`, `reset`, `silence`. Game render mode honors pitch scale and
  Doppler; Exact mode plays klattsch's samples untouched.
- `Klattsch` (singleton): the dictionary, `add_word` and `load_word_list`
  for your own pronunciations, `load_bank` for custom voices,
  `english_to_phonemes`, `compile`, `render` (a `KlattschSample` to replay)
  and `save_wav`.

Custom voices are klattsch bank JSON files. Load them with
`Klattsch.load_bank("user://my-voice.json")`; for a bank shipped in the
project, give it an extension Godot does not import (for example
`.voice`) and add that to the export filter too, since Godot converts
`.json` files on import.

## In the editor

These need no setup; they appear once the addon is installed.

- **Audition:** select a `KlattschPlayer`, or open a `KlattschStream`, and
  the Inspector starts with an Audition panel. Pick English, Japanese or
  Phonemes, type a line and press Play (or Ctrl+Enter) to hear it in that
  node's voice: its stream's bank, pitch, rate and the rest, as the game
  would speak it. Tweak a Voice property and play again. "To Phonemes"
  turns English or Japanese into a phoneme string you can hand-tune (stress,
  pitch, timing) and paste into `say_phonemes`. The status line shows the
  length, the bank, words sounded out because the dictionary lacks them, and
  any warnings.
- **Highlighting:** phoneme strings in the panel are coloured, with
  phonemes the voice's bank lacks in red. For text files of phoneme
  strings, choose Edit > Syntax Highlighter > klattsch phonemes in the
  script editor. In your own tools or in-game editors, assign a
  `KlattschSyntaxHighlighter` to a `TextEdit` or `CodeEdit` (its `bank` and
  colours are properties); `Klattsch.highlight(text, bank)` returns the raw
  spans.

## Licenses

MIT (see `LICENSE`). `THIRD-PARTY-NOTICES.md` lists everything a game
shipping this addon must credit: klattsch, V8's fdlibm, the CMU
Pronouncing Dictionary, the NRL letter-to-sound rules, gdext (MPL-2.0) and
the Rust crates compiled in.
