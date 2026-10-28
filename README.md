# klattsch (Rust port)

A Rust port of [klattsch](https://github.com/tgies/klattsch) 0.8.0, the
primitive parallel-formant speech synthesizer by Tony Gies (MIT). The port is
**bit-exact** against the JS engine: tokens, schedules, phrases, rendered
samples, WAV bytes, and highlight spans all match, checked by parity tests
against fixtures recorded from the original code.

The end goal is a Godot GDExtension (`KlattschPlayer` node), being built in
`crates/klattsch-godot` with [gdext](https://github.com/godot-rust/gdext) to
the plan in `docs/klattsch-godot-build-spec.docx` (amendments at the end of
`docs/gdext-handoff.md`).

## Layout

| Path | What |
|---|---|
| `crates/klattsch-core` | The engine: `dsp`, `synth` (FormantSynth), `sequencer` (tokenize/compile), `banks` (registry, runtime-loadable banks), `wav`, `kana`, `pronounce` (CMU dictionary), `highlight` (editor lexer) |
| `crates/klattsch-core/banks` | The bundled phoneme banks (copied from `klattsch-main/src/engine/banks`) |
| `crates/klattsch-cli` | The `klattsch` command-line WAV renderer |
| `data/cmudict` | CMU Pronouncing Dictionary 0.7b (BSD-style license in the file header) |
| `crates/klattsch-godot` | The Godot 4.7 GDExtension (gdext 0.5, `api-4-7`, `experimental-threads`). So far: `KlattschStream`, an `AudioStream` that speaks klattsch on Godot's audio thread in game (resampled) or exact mode, with `speak` / `set_param` / `set_params` / `reset` / `silence`; `Klattsch.compile` and `KlattschUtterance`; `KlattschPlayer`, a node that speaks through an `AudioStreamPlayer` / 2D / 3D (`say_phonemes`, `say_utterance`, interrupt or queue, `stop`) and emits `utterance_started`, `phrase_started`, `utterance_finished` and `error` in time with what is heard; `say(english)` and `say_japanese(kana)` use the stream's Voice properties (bank, base pitch, rate, ...). The `Klattsch` singleton loads the CMU dictionary in the background (`load_dictionary`, `dictionary_loaded`), holds the runtime lexicon (`add_word`, `load_word_list`), registers banks (`load_bank`, `register_bank_json`), converts text (`english_to_phonemes`, `english_words`, `japanese_to_phonemes`, `compile`), and pre-renders: `render(utterance)` returns a `KlattschSample` (an `AudioStream` over the float samples, exact or game mode, seekable, `to_audio_stream_wav()` for a 16-bit stock copy) and `save_wav(utterance, path, comment, sample_rate)` writes the same bytes as `klattsch-cli`. Editor extras: `Klattsch.highlight(phonemes, bank)` and `KlattschSyntaxHighlighter` colour phoneme strings in any `TextEdit` (unknown phonemes in red), the script editor gains a "klattsch phonemes" highlighter, and the Inspector shows an audition panel on a `KlattschPlayer` or `KlattschStream` (English, Japanese or phonemes, Play renders the line in that node's voice; "To Phonemes" converts text to an editable phoneme string). The `alloc-check` feature counts allocations inside `mix` |
| `godot` | Test project for the extension: `klattsch.gdextension` loads the `target/debug` build (the editor always uses the debug entry); `demo.tscn` (the main scene) highlights each phoneme as it is heard (Space speaks, I interrupts, P pauses, Up/Down glide the pitch); `speech.tscn`, `exact.tscn`, `player_test.tscn`, `lang_test.tscn`, `sample_test.tscn` and `highlight_test.tscn` run the step 2-6 and 8 checks and print a `STEPn_RESULT` JSON line ending in a verdict (`"ok"`, and the `"failed"` checks; see `verdict.gd`). The step 8 editor checks run in the editor itself through the `addons/klattsch_editor_test` plugin: `Godot_v4.7.2-stable_win64_console.exe --headless --editor --path godot -- --klattsch-editor-test` prints `STEP8_EDITOR_RESULT` and quits (add `--screenshot=<png>` without `--headless` to save the editor window). Copy the dictionary in first: `cp data/cmudict/cmudict-0.7b godot/addons/klattsch/cmudict.dict` (gitignored). `export_presets.cfg` exports a Windows build with `*.dict` in the non-resource filter; exported builds take no scene argument, so run a test as `klattsch-test.console.exe -- --scene=res://lang_test.tscn`. Findings are in `docs/gdext-handoff.md` |
| `addon` | Sources for the shipped addon: `klattsch.gdextension` (library paths for Windows, Linux, macOS universal), class icons, the addon README |
| `tools/package_addon.py` | Assembles `addons/klattsch/` (libraries, dictionary, icons, licenses) and optionally a zip |
| `.github/workflows/build.yml` | CI: parity tests on Windows, Linux and macOS against tgies/klattsch 0.8.0, the three libraries, and the `klattsch-godot-addon` artifact |
| `tools/gen-fixtures.mjs` | Runs the JS engine and records the parity fixtures |

## Usage

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
  audio clock. Parity tests run the real `formant-worklet.js` in Node
  (`tools/worklet-harness.mjs`) and compare every output block.
- Schedules can carry a tag (`PreparedSchedule::with_tag`); a synth reports
  the tag it is playing (`FormantSynth::utterance_tag`) and its start
  (`sample_counter` samples ago), which is how KlattschPlayer times its
  signals.
- `append` / `Command::Append` (not in the worklet) starts a schedule at
  the exact sample the voice's current utterance ends (its `total_ms`, set
  with `PreparedSchedule::with_duration_ms`), or at once if the voice is
  silent; it is how queued speech plays without gaps. Existing commands are
  unchanged and still bit-exact against the worklet.
- The audio-thread side never allocates or frees memory
  (`tests/realtime.rs` checks this with a counting allocator): schedules are
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

### English text (Rust-only)

`english_to_phonemes` (and the CLI's `--english`) turns English text into a
klattsch phoneme string. The JS engine's `pronounceText` is only a dictionary
lookup, so this front end is new and has no JS parity fixtures; it never
changes how phoneme strings compile or render. It:

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

## Parity tests

The fixtures are generated, not committed (about 30 MB). They need Node and the
`klattsch-main` checkout next to this folder:

```bash
node tools/gen-fixtures.mjs
```

```bash
cargo test --workspace --release
```

### Differential fuzzing

`tools/fuzz.mjs` generates seeded random inputs for every feature (phoneme
strings using every directive and malformed variant, banks with inheritance
chains and cycles, live-synth operation sequences with extreme and non-finite
values, WAV buffers, kana/romaji, highlight input, every dictionary word), runs
them through the JS engine, and `tests/fuzz_parity.rs` checks the port against
all of them, reporting every mismatch:

```bash
node tools/fuzz.mjs --seed 2 --scale 3
```

```bash
cargo test --release -p klattsch-core --test fuzz_parity -- --nocapture
```

Rendered audio is compared by a hash of the exact `f32` bit patterns (NaNs
canonicalized, since neither language specifies NaN payload bits).

### JS coverage

`tools/coverage.mjs` runs the fixture generator, the fuzzer and the JS CLI
under V8 block coverage and lists any engine code that never ran, i.e.
behavior the tests have not compared. Everything is covered except
unreachable defensive branches (for example `if (!part) continue` after the
loop that guarantees a non-empty part) and fallbacks for hand-built JS objects
the Rust types cannot express.

```bash
node tools/coverage.mjs
```

### Rust coverage

With [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov), after
generating both fixture sets (a larger fuzz set reaches more rare paths):

```bash
cargo llvm-cov --workspace --release --show-missing-lines
```

About 99% of lines run. The rest are unreachable: the same defensive
branches the JS has (an empty token, leftover highlight text, an empty voice,
a note name that fails the regex it already matched), and fdlibm branches that
`sin`/`cos` screen out before calling the reduction (`|x| <= pi/4`, inf/NaN)
or that the algorithm's invariants exclude.

### Platform note

Fixtures are recorded with Node on the machine that runs the tests, and the
port matches Node on that platform, so regenerate fixtures on each platform
you test on. Two things differ by platform, in V8 and so in the port:

- `Math.pow` calls the platform C library (the port uses it only for note
  names like `bC#4`).
- On Apple Silicon, Node's V8 is compiled with Apple clang's default
  floating-point contraction, so the `a*b + c` steps inside `Math.sin` and
  `Math.cos` are fused multiply-adds (one rounding instead of two). The port
  does the same there (`FUSED` in `src/fdlibm.rs`, Apple arm64 only; the
  Intel half of a universal macOS build is unfused). Results on M-series
  Macs can therefore differ from other platforms in the last bit, exactly as
  Node's do.

How exactness is achieved:

- All DSP math is `f64` with the same operation order as the JS; samples are
  rounded to `f32` exactly where the JS stores into a `Float32Array`.
- `Math.sin`/`Math.cos` are V8's own fdlibm port (`src/fdlibm.rs`). The `libm`
  crate and the platform libraries differ from V8 in the last bit for about
  1% of arguments, which showed up in rendered audio.
- JS semantics that differ from Rust are reproduced in `src/js.rs`: `Math.max`
  / `Math.min` NaN and signed-zero rules, `Math.round` ties, the `\s`
  whitespace set, number-to-string, and string-to-number.
- Bank and schedule data keep JS object semantics (`Value`, ordered maps,
  extra fields riding along into schedule targets).

## Packaging the Godot addon

CI (`.github/workflows/build.yml`) does this on every push to `main`:
it checks out [tgies/klattsch](https://github.com/tgies/klattsch) at the
0.8.0 release commit next to the repository, records fixtures with Node and
runs the parity tests on Windows, Ubuntu and macOS; builds the Windows DLL
(static C runtime, so players need no Visual C++ redistributable), the Linux
library on Ubuntu 22.04 (an older glibc) and a universal macOS library
(`lipo` of arm64 and x86_64, signed ad hoc); and uploads `addons/klattsch`
as the `klattsch-godot-addon` artifact, ready to unzip into a project.
It then downloads Godot 4.7.2 (checksum verified) on Ubuntu, macOS and
Windows and runs `tools/godot_tests.py`: every test scene in `godot/` and
the editor checks, headless, against that addon, with `save_wav` output
compared to the CLI's byte for byte. Locally:

```bash
python tools/godot_tests.py --godot ../Godot_v4.7.2-stable_win64_console.exe --addon target/dist/klattsch-godot --cli target/release/klattsch
```

A local Windows-only build:

```bash
RUSTFLAGS="-C target-feature=+crt-static" CARGO_TARGET_DIR=target/package cargo build --release -p klattsch-godot --target x86_64-pc-windows-msvc
```

```bash
python tools/package_addon.py --windows target/package/x86_64-pc-windows-msvc/release/klattsch_godot.dll --allow-missing --out target/dist/klattsch-godot --zip
```

## Licenses

The port is MIT, like klattsch. [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md)
collects every notice that a program or game shipping klattsch must
reproduce: klattsch itself, V8's fdlibm, the CMU dictionary, the NRL rule
table, and the Rust crates compiled in. After changing dependencies,
regenerate it with [cargo-about](https://github.com/EmbarkStudios/cargo-about)
(`cargo install cargo-about --features cli`):

```bash
cargo about generate --offline about.hbs -o THIRD-PARTY-NOTICES.md
```

## Intentional differences from the JS

- **Source offsets are Unicode code points** (Godot's string indexing). The JS
  counts UTF-16 code units; the two agree unless the text contains characters
  outside the BMP, such as emoji. The fixture generator converts the JS
  offsets for comparison.
- **The highlight lexer returns spans** (`class`, `start`, `end`, `text`)
  instead of DOM nodes, without the trailing `"\n"` node the JS adds for
  textarea layout.
- **Bank registries are explicit objects.** `banks()` is a process-wide default
  like the JS module-level registry, but you can create independent
  `BankRegistry` instances. A raw bank object passed as `CompileOptions::bank`
  is resolved against the registry you compile with (the JS always used the
  global one).
- **String-valued synth parameters are coerced with JS `ToNumber`** when
  queued. The JS stores the string and later string-concatenates onto it
  (`"0.5" + 0.01` is `"0.50.01"`, then NaN). This only arises from malformed
  banks that give `voicing`, `A1`-`A3` or `gain` a string value; numbers,
  booleans and `null` behave identically. (Formant fields such as `F1` are
  scaled to numbers by the compiler in both versions, so strings there match.)
- **Words named after `Object.prototype` members are unknown tokens.** In the
  JS, `toString`, `constructor`, `valueOf`, `hasOwnProperty` and the like hit
  the pause table's prototype and become pauses whose length is a function, so
  every later time turns into a string (`totalMs` becomes NaN). The port warns
  `unknown token` instead.
- **The CMU dictionary is loaded from a file at runtime** instead of the npm
  package. Alternate pronunciations are separate words (`hello(1)`), and
  stress digits are honored when the file has them (0.7b does).
