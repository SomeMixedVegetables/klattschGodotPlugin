# klattsch for Godot

A small speech synthesizer for Godot. Your characters can say any English
or Japanese line out loud, with no recorded audio, in voices you shape
yourself, and with signals for lip-sync and subtitles.

**Download: [the latest release](https://github.com/SomeMixedVegetables/klattschGodotPlugin/releases/latest)**
(the file `klattsch-godot-vX.Y.Z.zip`)

Needs Godot 4.7 or newer, on Windows or Linux (64-bit). macOS is not
available yet.

## Add it to your project

1. Download `klattsch-godot-vX.Y.Z.zip` from the
   [latest release](https://github.com/SomeMixedVegetables/klattschGodotPlugin/releases/latest).
2. Unzip it into your project folder (the folder with `project.godot`).
   You get `addons/klattsch/`; if you already have an `addons` folder, it
   goes inside it.
3. Restart the Godot editor, or open the project if it was closed.

That's all: there is no plugin to enable, and exported games work without
extra export settings. You can now add a **KlattschPlayer** node from the
Add Child Node dialog.

## Use it

### Make something talk

Add a `KlattschPlayer` node to your scene, then in a script:

```gdscript
extends Node

@onready var voice: KlattschPlayer = $KlattschPlayer

func _ready() -> void:
	Klattsch.load_dictionary()        # load the English dictionary in the background
	await Klattsch.dictionary_loaded  # ready a fraction of a second later
	voice.say("Hello! Nice to meet you.")
```

Load the dictionary once, early (in your first scene or an autoload). Until
it is ready, English words are guessed from their spelling.

### Change the voice

Select the KlattschPlayer, open its **Stream** in the Inspector, and set the
**Voice** settings: pitch, speed, vocal tract size, vibrato, breathiness and
more. The **Audition** panel at the top of the Inspector plays a line in
that voice without running the game. Or set them in code:

```gdscript
voice.stream.base_f0 = 200.0   # higher pitch (Hz)
voice.stream.rate = 90.0       # faster (milliseconds per sound)
```

### Lip-sync and subtitles

```gdscript
func _ready() -> void:
	voice.phrase_started.connect(_on_sound)
	voice.utterance_finished.connect(_on_line_done)
	voice.say("Follow me, quickly!")

# Called as each sound is heard: pick a mouth shape from `phoneme`.
func _on_sound(id: int, index: int, phoneme: String, from: int, to: int) -> void:
	print(phoneme)

func _on_line_done(id: int) -> void:
	print("finished line ", id)
```

### More things it can do

```gdscript
voice.queue_mode = KlattschPlayer.QUEUE   # lines wait their turn instead of interrupting
voice.say("First line.")
voice.say("Second line.")
voice.stop()                              # stop speaking now

voice.say_japanese("こんにちは")             # kana or romaji
voice.say_phonemes("HH AH L OW'")          # klattsch phonemes, for full control
voice.set_param(&"F0", 300.0, 500.0)      # glide the pitch to 300 Hz over half a second

Klattsch.add_word("Zorblax", "Z AO1 R B L AE0 K S")   # teach it a name
```

To speak from a place in the world, set the KlattschPlayer's **Target** to
an `AudioStreamPlayer2D` or `AudioStreamPlayer3D`.

## Example project

[GodotSampleProjects/klattsch-demo](GodotSampleProjects/klattsch-demo) shows
every feature, one tab each, with scripts written to be copied from:
speaking, voices, live control, custom words and voices, pre-rendered lines
and WAV files, and 2D positional speech, plus a lip-synced face. To run it,
unzip the release into that folder and open it in Godot; its
[README](GodotSampleProjects/klattsch-demo/README.md) has the details.

The addon's own README (`addons/klattsch/README.md`, in the zip) lists
everything it offers.

## About

klattsch for Godot is a Rust port of [klattsch](https://github.com/tgies/klattsch)
by Tony Gies, and sounds exactly like it. Building from source and the Rust
library are covered in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md); changes
are in [CHANGELOG.md](CHANGELOG.md).

MIT license. A game that ships the addon must include the credits in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) (also in the zip).
