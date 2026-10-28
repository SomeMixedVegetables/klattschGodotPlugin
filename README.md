# klattsch for Godot

A small parallel-formant speech synthesizer for Godot 4.7 and later. It
speaks English, Japanese and klattsch phoneme strings in real time through
Godot's own audio players (2D and 3D included), with live control of every
synth parameter and signals timed to what the player hears, for lip-sync and
subtitles.

It is a Rust port of [klattsch](https://github.com/tgies/klattsch) 0.8.0 by
Tony Gies (MIT), and sounds exactly like it: the port is bit-exact against
the original JS engine.

Windows (x86_64) and Linux (x86_64). macOS builds are paused for now.

## Install

1. Download `klattsch-godot-vX.Y.Z.zip` from
   [Releases](https://github.com/SomeMixedVegetables/klattschGodotPlugin/releases).
2. Unzip it into your project folder, so the project contains
   `addons/klattsch/`.
3. Restart the Godot editor.

Exported games include the pronunciation dictionary automatically.

## Quick start

Add a `KlattschPlayer` node (with no target it makes its own audio player,
or point `target` at an `AudioStreamPlayer2D` / `AudioStreamPlayer3D`), then:

```gdscript
func _ready() -> void:
	Klattsch.load_dictionary()   # loads in the background
	$KlattschPlayer.phrase_started.connect(func(id, i, phoneme, a, b): print(phoneme))
	$KlattschPlayer.say("Halt! Who goes there?")
```

Select the `KlattschPlayer` to get an Audition panel in the Inspector:
type a line, press Play, and hear it in that voice without running the game.
The addon's [README](addon/README.md) covers the rest: voices, live pitch
control, queueing, custom words and voices, pre-rendering and WAV files.

## Releasing

1. Set the new version in `Cargo.toml` (`[workspace.package] version`).
2. Add a `## X.Y.Z` section to [CHANGELOG.md](CHANGELOG.md); it becomes the
   release notes.
3. Commit, then tag and push:

```bash
git tag vX.Y.Z
```

```bash
git push origin vX.Y.Z
```

CI builds the three libraries, packages the addon and publishes the GitHub
release with `klattsch-godot-vX.Y.Z.zip` attached. It stops if the tag does
not match the version in `Cargo.toml` or the changelog has no section for it.

## Development

### Layout

| Path | What |
|---|---|
| `crates/klattsch-core` | The engine: `dsp`, `synth` (FormantSynth), `sequencer` (tokenize/compile), `banks` (registry, runtime-loadable banks), `wav`, `kana`, `pronounce` (CMU dictionary), `english` and `lts` (English text), `player` (real-time playback), `highlight` (editor lexer) |
| `crates/klattsch-core/banks` | The bundled phoneme banks |
| `crates/klattsch-cli` | The `klattsch` command-line WAV renderer |
| `crates/klattsch-godot` | The Godot 4.7 GDExtension (gdext 0.5): `KlattschPlayer`, `KlattschStream`, `KlattschSample`, `KlattschUtterance`, the `Klattsch` singleton, and the editor extras (Audition panel, phoneme syntax highlighting, the export plugin that adds the dictionary) |
| `data/cmudict` | CMU Pronouncing Dictionary 0.7b (BSD-style license in the file header) |
| `addon` | Sources for the shipped addon: `klattsch.gdextension`, class icons, the addon README |
| `godot` | Development project: `klattsch.gdextension` loads the `target/debug` build, and `demo.tscn` highlights each phoneme as it is heard (Space speaks, I interrupts, P pauses, Up/Down glide the pitch). Copy the dictionary in first: `cp data/cmudict/cmudict-0.7b godot/addons/klattsch/cmudict.dict` (gitignored) |
| `tools/package_addon.py` | Assembles `addons/klattsch/` (libraries, dictionary, icons, licenses) and optionally a zip |
| `.github/workflows/build.yml` | CI: the three libraries and the `klattsch-godot-addon` artifact on every push to `main`; releases for version tags |

### Rust usage

```rust
use klattsch_core::*;

let compiled = compile_string("HH AH L OW", &CompileOptions::default())?;
let buf = render_to_buffer(48000.0, compiled.schedule(), Some(compiled.total_ms), None)?;
let wav = encode_wav(&buf, 48000, &WavOptions::default());
```

For real-time playback, `player` (`src/player.rs`) is the equivalent of the
JS AudioWorklet (`src/formant-worklet.js`), with no audio API attached:

```rust
let (handle, mut player) = player(48000.0, ProcessorOptions::default(), 256)?;

// Game thread: speak, steer live, reset. Sends never block; a full queue
// hands the command back.
handle.schedule(Some(0), compiled.schedule(), None)?;
handle.append(Some(0), next.schedule(), next.total_ms)?; // after the current line
handle.frame(Some(0), PartialParams::new().with(Param::F0, 180.0), Some(50.0))?;

// Audio thread, once per block: apply pending commands, then render.
player.process_mix(&mut block, 1);     // voices mixed into one buffer
player.process(frames, &mut outputs);  // or one buffer per voice

// Lip-sync: which phrase is sounding, from the audio clock.
let t_ms = (handle.current_time() - started_at) * 1000.0;
let phrase = phrase_at(compiled.phrases(), t_ms);
```

- `FormantProcessor` is the worklet's logic: per-voice synths created on
  first use, `frame` / `schedule` / `reset` commands, and `startTime`
  (`Command::Schedule::start_time`) to start a schedule at a point on the
  audio clock.
- Schedules can carry a tag (`PreparedSchedule::with_tag`); a synth reports
  the tag it is playing (`FormantSynth::utterance_tag`) and its start
  (`sample_counter` samples ago), which is how KlattschPlayer times its
  signals.
- `append` / `Command::Append` (not in the worklet) starts a schedule at
  the exact sample the voice's current utterance ends (its `total_ms`, set
  with `PreparedSchedule::with_duration_ms`), or at once if the voice is
  silent; it is how queued speech plays without gaps.
- The audio-thread side never allocates or frees memory: schedules are
  prepared on the game thread (`PreparedSchedule`), and replaced ones are
  handed back to it to be freed.
- `FormantSynth` can still be driven directly: `queue_schedule` /
  `queue_events` for compiled utterances, `set_target` for live parameter
  changes, and `process` to fill audio blocks of any size.

Custom voices are bank JSON files registered at runtime:
`BankRegistry::load_file("my-voice.json")`, then compile with
`compile_string_with_registry` (or `[bank=my-voice]` in the string).

CLI (same arguments and output as the JS `bin/klattsch.mjs`, plus options):

```bash
cargo run --release -p klattsch-cli -- "HH AH L OW" hello.wav
```

```bash
cargo run --release -p klattsch-cli -- --english "Hello, world" hello.wav
```

Run with no arguments to see all options.

### English text

`english_to_phonemes` (and the CLI's `--english`) turns English text into a
klattsch phoneme string. The JS engine's `pronounceText` is only a dictionary
lookup, so this front end is new; it never changes how phoneme strings
compile or render. It:

- turns punctuation into klattsch pauses: `,` `(` `)` 100 ms, `;` `:` dashes
  200 ms, `.` `!` `?` `…` 300 ms
- reads numbers as words (`20`, `1,250`, `3.5`, `21st`, `50%`, years like
  `1999`), money (`$4.50`, `£3`, `€2.5 million`) and clock times (`3:30`,
  `10:00`, `9pm`, `3:30 p.m.`)
- reads common abbreviations (`Dr.`, `Mr`, `St.`, `Jan.`, `etc.`, `vs`,
  `No. 5`), dotted acronyms (`U.S.`, `e.g.`) and initials (`John F. Kennedy`)
  without ending the sentence at their period; `St.`/`Dr.` read as saint/doctor
  before a name and street/drive after one
- looks words up in the dictionary, then tries acronym spelling (also plurals
  like `NPCs`), possessives, hyphen and camel-case splits (`FitnessGram` =
  fitness + gram, `GDExtension` = G D + extension), two-word compounds
  (`multistage` = multi + stage), and finally the NRL letter-to-sound rules
  (`src/lts.rs`, Elovitz et al. 1976, rule table from
  [mewo2/elovitz](https://github.com/mewo2/elovitz), MIT)
- takes phones written in braces as they are, for words the rules get wrong
  (homographs, names): `I will {R IY1 D} it`
- removes dictionary stress from function words (the, is, to, you, ...)
- adds sentence melody with pitch directives: a slow downward drift, a fall
  on a statement's last accent, a rise on a yes/no question (wh-questions
  fall), and a small rise before a comma (`EnglishOptions` sets the amounts;
  `--no-intonation` turns it off)

#### Custom word lists

Word lists for names and invented words use the CMU file format (`;;;`
comments, `WORD  PH1 PH2 ...` with optional stress digits) and are layered
over the CMU dictionary with a `Lexicon`. The first list that has a word
decides it, so a list can also correct a CMU pronunciation:

```rust
let cmu = Arc::new(CmuDict::load("data/cmudict/cmudict-0.7b")?);
let mut lex = Lexicon::new();
lex.push_layer(Arc::new(CmuDict::load("game-words.txt")?)); // checked first
lex.push_layer(cmu);                                        // then CMU
lex.add_word("Zyx", "Z IH1 K S")?;                          // checked before both
let spoken = english_to_phonemes("Zyx says hi", &lex, &EnglishOptions::default());
```

Layers are shared (`Arc`), so cloning a lexicon to give one character extra
words does not copy the dictionary. On the CLI: `--words game-words.txt`
(repeatable; earlier files win).

### Building the addon

CI (`.github/workflows/build.yml`) builds the Windows DLL (static C runtime,
so players need no Visual C++ redistributable), the Linux library on Ubuntu
22.04 (an older glibc, so it runs on more distributions), and uploads
`addons/klattsch` as the `klattsch-godot-addon` artifact on every push to
`main`. The universal macOS library (`lipo` of arm64 and x86_64, signed ad
hoc) is turned off for now: GitHub's Apple Silicon runners were unavailable.
To turn it back on, add `macos-14` to the test matrix and remove
`if: false` from `build-macos` in the workflow.

A local Windows-only build:

```bash
RUSTFLAGS="-C target-feature=+crt-static" CARGO_TARGET_DIR=target/package cargo build --release -p klattsch-godot --target x86_64-pc-windows-msvc
```

```bash
python tools/package_addon.py --windows target/package/x86_64-pc-windows-msvc/release/klattsch_godot.dll --allow-missing --out target/dist/klattsch-godot --zip
```

## Licenses

MIT, like klattsch. [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)
collects every notice that a program or game shipping klattsch must
reproduce: klattsch itself, V8's fdlibm, the CMU dictionary, the NRL rule
table, gdext and the Rust crates compiled in. After changing dependencies,
regenerate it with [cargo-about](https://github.com/EmbarkStudios/cargo-about)
(`cargo install cargo-about --features cli`):

```bash
cargo about generate --offline about.hbs -o THIRD-PARTY-NOTICES.md
```
