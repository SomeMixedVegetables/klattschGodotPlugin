# Changelog

Each release's section here becomes its notes on GitHub.

## 0.1.0

The first release of klattsch for Godot: a small formant speech synthesizer
for Godot 4.7 and later, on Windows (x86_64), Linux (x86_64) and macOS
(Apple Silicon and Intel). It sounds exactly like
[klattsch](https://github.com/tgies/klattsch) 0.8.0, which it is a Rust port
of.

- `KlattschPlayer` node: `say` (English), `say_japanese` (kana or romaji),
  `say_phonemes` and `say_utterance`, through a stock `AudioStreamPlayer`,
  2D or 3D. Interrupt or queue mode, `stop`, live pitch and parameter glides
  with `set_param`, and the signals `utterance_started`, `phrase_started`
  (for lip-sync and highlighting) and `utterance_finished`, timed to what
  the player hears.
- `KlattschStream`, the voice itself: bank, pitch, rate, vibrato, tremolo
  and voice quality in the Inspector.
- `Klattsch` singleton: English with the CMU Pronouncing Dictionary (loaded
  in the background), your own words and word lists, custom voices from
  bank JSON files, pre-rendered `KlattschSample` streams, and WAV files.
- In the editor: an Audition panel in the Inspector to hear a line in a
  node's voice without running the game, and syntax highlighting for
  phoneme strings.
- Exports include the dictionary automatically.

To install, unzip into your project folder (you get `addons/klattsch/`) and
restart the editor. On macOS, if the editor refuses to load the library,
run `xattr -dr com.apple.quarantine addons/klattsch` in the project folder.
