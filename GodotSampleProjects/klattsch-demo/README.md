# klattsch sample project

A how-to for the [klattsch](https://github.com/SomeMixedVegetables/klattschGodotPlugin)
speech synthesizer addon for Godot 4.7+: every feature, one tab each, with
scripts written to be copied from.

## Run it

1. Download `klattsch-godot-vX.Y.Z.zip` from the
   [latest release](https://github.com/SomeMixedVegetables/klattschGodotPlugin/releases/latest)
   and unzip it into this folder, so it contains `addons/klattsch/`.
2. Open this folder's `project.godot` in Godot 4.7 or newer.
3. Press Play (F5) and try each tab; then read the matching script.

## Where to look

| Tab | Script | Shows |
|---|---|---|
| Speak | `tabs/speak.gd` | `say` (English), `say_japanese`, `say_phonemes`; interrupt or queue; `stop`; `english_to_phonemes`, `japanese_to_phonemes`, `english_words` |
| Voice | `tabs/voice.gd` | The KlattschStream Voice properties (pitch, speed, vocal tract size, vibrato, tremolo, breathiness, brightness, effort), presets made from them, the voice bank, intonation |
| Live control | `tabs/live.gd` | `set_param` and `set_params`: steering pitch, formants and the rest live, with glides; `silence`, `reset`; a siren in a few lines |
| Words & voices | `tabs/words.gd` | `add_word`, `remove_word`, `load_word_list`, `{phones}` in braces; custom voices with `load_bank` and `register_bank_json` |
| Pre-render | `tabs/prerender.gd` | `compile_with`, `render` to a `KlattschSample`, game and exact modes, seeking, `to_audio_stream_wav`, `save_wav`, phrase timing |
| Positional | `tabs/positional.gd` | A KlattschPlayer whose `target` is an `AudioStreamPlayer2D`, panned by position |
| (side panel) | `main.gd`, `mouth.gd` | The signals `utterance_started`, `phrase_started`, `utterance_finished`, `error`: a lip-sync mouth, the phoneme being heard highlighted in its line, and a log; `Klattsch.dictionary_loaded` |

`main.tscn` holds the klattsch nodes the way you would set them up in your
own scenes: **Voice** (a KlattschPlayer with a KlattschStream), **Voice2D**
(a KlattschPlayer targeting **Speaker2D**, an AudioStreamPlayer2D) and
**SamplePlayer** (an AudioStreamPlayer for pre-rendered lines). `ui.gd` only
builds the controls.

## The basics, in one script

```gdscript
@onready var voice: KlattschPlayer = $Voice

func _ready() -> void:
	Klattsch.load_dictionary()                     # parses in the background
	await Klattsch.dictionary_loaded                # until then, words are guessed
	voice.phrase_started.connect(_on_phrase)        # lip-sync, subtitles
	voice.say("Halt! Who goes there?")

func _on_phrase(id: int, index: int, phoneme: String, from: int, to: int) -> void:
	print(phoneme)                                  # e.g. pick a mouth sprite
```

## In the editor

Select the **Voice** node: the Inspector starts with an **Audition** panel.
Type a line, press Play, and hear it in that node's voice without running
the game; change the Voice settings and play it again. For text files of
phoneme strings, the script editor has Edit > Syntax Highlighter >
klattsch phonemes.

## Exporting

The CMU dictionary is exported automatically. Other files the game reads
at run time with `FileAccess` (here `words/game-words.txt` and
`voices/whisper.voice`) are not resources, so add them in Project > Export
> Resources > "Filters to export non-resource files/folders", for example
`*.txt, *.voice`.

## Licenses

The scripts here are MIT, like klattsch; use them as you like. See
`addons/klattsch/THIRD-PARTY-NOTICES.md` for what a game shipping the addon
must credit.
