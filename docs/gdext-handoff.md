# Handoff: klattsch in Godot via gdext

**Audience:** a research agent. **Goal:** find the best way to build a Godot 4
GDExtension in Rust (with [gdext](https://github.com/godot-rust/gdext)) that
plays klattsch speech **through Godot's own audio system**, then report back
with a recommended architecture before any extension code is written.

The Rust engine is finished and tested; the open question is how to attach it
to Godot. This document says what exists, what the extension must do, what we
already believe about Godot audio (to be checked), and what we need answered.

---

## 1. Background

- **klattsch** is a small parallel-formant speech synthesizer (originally JS,
  running in a browser AudioWorklet). This repository is a bit-exact Rust port:
  `crates/klattsch-core` (the engine) and `crates/klattsch-cli` (a WAV
  renderer). See `README.md`.
- **Target:** Godot **4.7.2** (the editor exe sits in the parent folder),
  desktop only: **Windows, macOS, Linux**. Binding: **gdext** (decided; not
  godot-cpp).
- **Priorities from the project owner:**
  1. Heavily **real-time streaming** speech, with **live parameter control**
     ("full control of everything").
  2. **Pre-rendered** playback too (render a line once, replay it).
  3. **Exact output:** the samples klattsch produces must not be changed by
     the integration except where Godot's mixer necessarily processes them
     (volume, panning, bus effects). In particular, avoid any path that
     silently resamples or quantizes klattsch's output unless we choose it.
  4. Voices and pronunciations loadable at **runtime** (custom voices).
- Repository: https://github.com/SomeMixedVegetables/klattschGodotPlugin,
  branch `main`.

## 2. What the Rust engine already provides

All in `klattsch-core` (public API re-exported from `src/lib.rs`).

### Text to schedule (game thread, allocates)

| API | Purpose |
|---|---|
| `english_to_phonemes(text, &dyn Pronounce, &EnglishOptions) -> EnglishResult` | English text to a klattsch phoneme string, with pauses, numbers, money, times, abbreviations, intonation. `EnglishResult { phonemes, words }`. |
| `CmuDict::from_bytes(&[u8])` | Parses the CMU dictionary (~3.7 MB file, ~45 ms, thread-safe; meant to run on a background thread). |
| `Lexicon` | Custom word lists layered over a shared `Arc<CmuDict>`; `add_word("Zyx", "Z IH1 K S")` at runtime. |
| `japanese_to_phonemes(text)` | Kana/romaji to phonemes (use with a `ja-*` bank). |
| `compile_string_with_registry(src, &CompileOptions, &BankRegistry) -> CompileResult` | Phoneme string to per-voice schedules. `CompileResult { voices, total_ms, warnings, source }`; each voice has a `schedule: Vec<ScheduleEvent>` and `phrases: Vec<Phrase>`. |
| `Phrase { src_start, src_end, token_src_start, t_start_ms, t_end_ms, kind, phoneme }` | Timing of each spoken token, for lip-sync and highlighting. Offsets are Unicode code points into the **phoneme string** (Godot's string indexing). |
| `BankRegistry::register_json(&str)` | Registers a voice (phoneme bank JSON) at runtime; three banks are built in. |
| `build_highlight(src, &HighlightOptions) -> Vec<HighlightSpan>` | Syntax-highlight spans for phoneme strings (possible editor `SyntaxHighlighter`). |
| `CompileOptions` | `base_f0`, `rate`, `scale`, vibrato/tremolo depth and rate, `aspiration`, `tilt`, `effort`, `bank`, ... |

The phoneme string language itself supports inline directives (pitch `b+12`,
rate, `[voice=N]` sections for multiple simultaneous voices, `[bank=...]`).

### Real-time playback (the AudioWorklet equivalent), `src/player.rs`

```rust
let (handle, mut player) = player(sample_rate, ProcessorOptions::default(), 256)?;
// game thread (any thread): never blocks; a full queue hands the command back
handle.schedule(Some(voice), compiled.voices[v].schedule.as_slice(), None)?;
handle.frame(Some(voice), PartialParams::new().with(Param::F0, 180.0), Some(50.0))?; // live control
handle.reset(None, None)?;
let t = handle.current_time(); // audio clock, seconds (frames rendered / sample rate)
// audio thread, once per block:
player.process_mix(&mut mono_block, n_voices); // or process(frames, &mut per_voice_outputs)
```

- `Player` (audio side) is `Send`; `PlayerHandle` (game side) is `Send`.
  Commands go through a bounded `std::sync::mpsc::sync_channel` (sent with
  `try_send`, received with `try_recv`, so neither side waits), applied at
  the start of each block.
- **The audio side never allocates or frees** (proved by a counting-allocator
  test, `tests/realtime.rs`). Schedules are prepared on the game thread
  (`PreparedSchedule`) and old ones are handed back to be freed there.
- Output is **mono `f32`**, one buffer per voice or a mix. Any block size
  works. Sample rate is fixed at construction (any positive rate; output is
  exact for that rate).
- Live parameters (19, see `synth::PARAMS`): `F0, voicing, F1, BW1, A1, F2,
  BW2, A2, F3, BW3, A3, gain, vibratoDepth, vibratoRate, tremoloDepth,
  tremoloRate, aspiration, tilt, effort`. `frame` glides to new values over a
  transition time. A schedule can be started at an exact point on the audio
  clock (`start_time`).
- `phrase_at(phrases, t_ms) -> Option<usize>`: which phrase is sounding.
- Performance: about 300x real time per voice on the dev machine.

### Pre-rendered playback

`render_to_buffer(sample_rate, schedule, total_ms, initial) -> Vec<f32>` and
`render_mix(&CompileResult, sample_rate)` give the whole utterance as mono
`f32`. `encode_wav` writes 16-bit PCM WAV bytes, peak-normalized (0.95 by
default) like the JS; there is no float WAV writer.

### Known gap in the core

`EnglishResult.words` says how each English word was pronounced but not where
its phones landed in the phoneme string, so English-level `word_started`
signals and text highlighting need a small core addition (word to phoneme
offset mapping). Phoneme-level timing (`Phrase`) already exists.

### File loading decisions already made

- The CMU dictionary ships as a file in the Godot project, read with
  `FileAccess.get_file_as_bytes` (works inside an exported `.pck`, unlike
  `std::fs`), parsed on a background thread, shared via `Arc`. The file has no
  extension, so it must be added to the export preset's non-resource filters
  (or renamed, e.g. `.dict`).
- Custom banks and word lists are also read through `FileAccess` (`res://` or
  `user://`) and passed to `register_json` / `CmuDict::parse`.

## 3. What the extension should offer (draft; the research should refine it)

A `KlattschPlayer` (name agreed; whether it is a Node, a stream resource, or
both is an open question, see section 5).

- **Speak:** `say(english)`, `say_phonemes(string)`, `say_japanese(string)`;
  queueing vs interrupting; `stop()`; `is_speaking()`.
- **Live control:** set any of the 19 parameters with a glide time, per voice;
  voice/bank selection; base pitch and rate; reset.
- **Multiple voices:** `[voice=N]` sections (chords, choirs) and several
  characters talking at once.
- **Positional audio:** must work with `AudioStreamPlayer`,
  `AudioStreamPlayer2D` and `AudioStreamPlayer3D`, buses and bus effects,
  pause, volume, and pitch scale where it makes sense.
- **Timing signals:** phrase/phoneme started, word started, finished, synced
  to what is actually heard (lip-sync, subtitles, text highlighting).
- **Pre-render:** render a line to something Godot can replay or save (an
  `AudioStream`, a WAV file in `user://`).
- **Resources:** banks (voices), word lists, the shared dictionary (autoload
  or singleton, loaded in the background).
- **Editor (nice to have):** phoneme-string syntax highlighting via
  `build_highlight`, an inspector to audition a line.

## 4. What we believe about Godot audio (verify all of it for 4.7.2)

1. Godot mixes on its own **audio thread**. Each playing
   `AudioStreamPlayer*` node owns an `AudioStreamPlayback`, whose `_mix`
   virtual (`buffer: AudioFrame*` stereo float frames, `rate_scale`, `frames`)
   the server calls on that thread. Mix rate comes from
   `AudioServer.get_mix_rate()` (project setting `audio/driver/mix_rate`).
2. **`AudioStreamGenerator`** + `AudioStreamGeneratorPlayback.push_frames()`
   lets game code push samples into a ring buffer from the main thread. It is
   simple, but rendering happens on the game thread, so frame hitches mean
   audio underruns, and latency equals the buffer length.
3. A **custom `AudioStream` + `AudioStreamPlayback`** subclass from a
   GDExtension renders inside `_mix` on the audio thread, like the browser
   AudioWorklet klattsch was designed for. Our `Player` type is made for this.
4. **`AudioStreamPlaybackResampled`** (`_mix_resampled`,
   `_get_stream_sampling_rate`) lets a stream render at its own rate and have
   Godot resample. That changes the samples (relevant to the exactness goal).
5. **`AudioStreamWAV`** stores 8/16-bit PCM or compressed formats, not float.
   Pre-rendering into it would quantize to 16 bits.
6. Timing: `AudioServer.get_time_since_last_mix()`,
   `AudioServer.get_output_latency()` and
   `AudioStreamPlayer.get_playback_position()` exist for syncing visuals to
   heard audio.
7. gdext exposes `IAudioStream` / `IAudioStreamPlayback` virtuals; `mix` takes
   a raw `*mut AudioFrame`. gdext's object borrow rules (`bind()` /
   `bind_mut()`) may panic or block if the game thread touches the playback
   object while the audio thread is inside `mix`. Multi-threaded use may need
   the `experimental-threads` feature.

## 5. Questions to answer

### Architecture

1. Which approach should `KlattschPlayer` use: a custom `AudioStream` /
   `AudioStreamPlayback` with rendering in `_mix` (our expectation), or
   `AudioStreamGenerator`, or something else (bus `AudioEffect`,
   `AudioStreamPolyphonic`)? Compare latency, robustness to frame hitches,
   3D/bus support, and implementation risk.
2. Shape of the public API: a `KlattschStream` resource used with any of the
   three stock player nodes, a `KlattschPlayer` node that wraps one, or both?
   What do existing GDExtension audio plugins do?
3. How should game-thread calls (say, set parameter) reach the playback
   object on the audio thread under gdext's threading model? Our plan is to
   keep all shared state in plain Rust (`PlayerHandle` on the game side,
   `Player` inside the playback) and never call `bind_mut()` across threads.
   Confirm this is sound and find the recommended pattern.
4. One playback per character, or one shared playback with many voices? How
   does that interact with 3D positioning and with a player node restarting
   or creating new playback instances (`play()` called twice, polyphony
   setting `max_polyphony`)?
5. Is `_mix` guaranteed to run on a single audio thread, at a fixed block
   size, and never concurrently with itself for one playback instance?
   What happens on pause, `stream_paused`, scene-tree pause, or when the node
   leaves the tree?

### Exactness and sample rate

6. Can we render at exactly `AudioServer.get_mix_rate()` with no resampling
   in our path, and does Godot touch samples between `_mix` and the bus
   beyond volume, pan and effects? What does `rate_scale` (pitch scale,
   Doppler) mean for a generated stream, and should we honor or ignore it?
7. Can the mix rate change at runtime (output device change)? How is that
   signaled, so we can rebuild the synth at the new rate?
8. Best pre-render target: float samples in a custom stream, `AudioStreamWAV`
   (16-bit), or something else? What do we lose with each?

### Timing

9. How to raise `phrase_started` / `word_started` / `finished` signals in
   sync with what the listener hears: emitting from the audio thread
   (allowed? `call_deferred`?) vs polling from `_process` using the audio
   clock plus `get_time_since_last_mix()` and `get_output_latency()`.

### gdext specifics

10. Current gdext version and its compatibility with Godot **4.7.x** (API
    level, `compatibility_minimum`, prebuilt vs custom API JSON). Any 4.5–4.7
    audio API changes that matter.
11. The exact `IAudioStreamPlayback` method signatures in that gdext version
    (`mix`, `start`, `stop`, `is_playing`, `get_playback_position`, `seek`),
    and whether an `AudioStream` subclass must implement `instantiate_playback`,
    `get_length`, `is_monophonic`, etc.
12. Panic safety: a panic in `mix` crosses into C++. How does gdext handle it,
    and what should we do (catch_unwind, output silence)?
13. Real-time safety in `mix`: does gdext allocate or lock when calling a
    virtual (e.g. creating `Gd` handles, logging)? Is there anything in the
    call path that would break our zero-allocation audio thread?
14. Packaging for Windows, macOS (universal binary, signing/notarization
    concerns for `.dylib`/`.framework`) and Linux: `.gdextension` file
    layout, `reloadable`, building in CI.
15. Background work: loading the dictionary with `WorkerThreadPool` or a
    plain Rust thread, and reading `res://` files with `FileAccess` off the
    main thread (allowed?).

## 6. Constraints and non-goals

- Do not change `klattsch-core`'s DSP or sequencer behavior: they are
  bit-exact against the JS engine and covered by parity tests. Additions to
  the core (new APIs) are fine.
- The audio thread must not allocate, free, lock, or block.
- Desktop only for now; web and mobile are out of scope (note anything that
  would block them later).
- License: everything is MIT or similar; gdext is MPL-2.0 (usable unmodified,
  listed in `THIRD-PARTY-NOTICES.md` once added).

## 7. What to deliver

1. A recommended architecture (and the runner-up), with reasons tied to the
   priorities in section 1.
2. Answers to the questions in section 5, each with a source: Godot docs or
   source files (with the version/commit), gdext docs or source, or a small
   experiment. Mark anything uncertain.
3. A proposed GDScript-facing API for `KlattschPlayer` (classes, methods,
   properties, signals).
4. Risks and a suggested order of work, starting with a minimal "tone from
   `_mix` in Godot 4.7.2 via gdext" spike.
5. If you build experiments, keep them outside `crates/klattsch-core` (for
   example a new `crates/klattsch-godot` crate and a throwaway Godot project).

---

## Outcome (2026-10-27)

The research agent's build spec is `docs/klattsch-godot-build-spec.docx`
("Handoff: building klattsch-godot"): a custom `AudioStream` /
`AudioStreamPlayback` rendering in `mix`, game and exact render modes, and an
eight-step build order starting with a sine-tone spike. It is the plan of
record, with these amendments agreed after review:

1. **Queueing:** `Command::Schedule` replaces a voice's schedule on arrival,
   so `speak(interrupt := false)` / `queue_mode = QUEUE` need a new core
   command that appends a schedule after the current one (sample-accurate, no
   gap). New API only; existing commands and JS parity are unchanged.
2. **`save_wav`** matches `klattsch-cli`: 16-bit PCM, peak-normalized to
   0.95, the CLI's metadata. No float option. This is what makes the spec's
   step 6 byte-identical check hold.
3. **One stream per player:** `KlattschStream` is `resource_local_to_scene`
   by default and `KlattschPlayer` always creates its own, so NPCs never share
   a `PlayerHandle`.
4. **CI comes last:** build and test on Windows through the spec's steps 1-6
   and 8; the GitHub Actions matrix (Windows, Linux, macOS universal) is the
   final step.
5. The dictionary lives at `res://addons/klattsch/cmudict.dict` (the spec
   names two paths).
6. gdext: `godot` 0.5.5 on crates.io has an `api-4-7` feature, so no custom
   API JSON is needed (recheck when building).

## Build findings (Windows 11, Godot 4.7.2, gdext 0.5.5, WASAPI at 48 kHz)

- **Step 1:** `mix` runs on one audio thread with 512-frame blocks (128-frame
  chunks for `mix_resampled`); 10,000 GDScript `get_playback_position()` calls
  per frame do not delay it (the longest gap between mix calls, ~21 ms, is
  the same without the load). Playbacks are always freed on the main thread,
  and `instantiate_playback` ran only on the main thread. gdext 0.5.5's
  resampled playback has no stray `mix` to stub (#1133 does not apply); the
  raw pointer type is `godot::meta::conv::RawPtr`.
- **Step 2:** zero allocator calls inside `mix` (the `alloc-check` feature);
  a command is applied by the next mix call (12.7 ms worst seen, bounded by
  the gap between mix calls, plus the device's output latency).
- **Step 3:** exact mode through a plain `AudioStreamPlayer` reaches the
  master bus **bit-identical** to `KlattschUtterance.render()`
  (`render_mix`, itself bit-exact against the JS engine), on both channels,
  for a single voice, a three-voice chord, and non-default voice options,
  with exact silence afterwards. Positional players only scale it:
  `AudioStreamPlayer2D` at the listener by exactly 0.5 (a linear pan law,
  -6.02 dB, every sample exact), `AudioStreamPlayer3D` 1 m straight ahead by
  ~0.99881 (-0.010 dB, within 1 ulp; the 3D low-pass filter does nothing at
  that distance). Game mode at pitch 1 with the internal rate equal to the
  mix rate is also bit-identical (Godot's cubic resampler is exact at 1:1),
  130 samples later.
- **Step 4:** `KlattschPlayer` learns each utterance's exact start frame
  from the audio thread (schedules carry a tag, `PreparedSchedule::with_tag`;
  the playback publishes voice 0's tag and start frame after every block)
  and estimates the heard frame as rendered frames + time since the last mix
  - output latency (10 ms reported here). `utterance_started` fired about 10
  ms before the first sound's mix time plus that latency, which itself is
  measured up to a frame late, so within the spec's 20 ms; phrase signals sit
  within about 13 ms of their `t_start_ms`. Queued lines start one length
  apart (480.2 ms for a 480 ms line), an interrupted line finishes the moment
  the new one starts with no later phrase signals, a tree pause freezes both
  sound and signals, and every id gets exactly one `utterance_finished`
  (including stop and dropped lines). Calling `say` from a signal handler
  works. Godot reports "ObjectDB instances leaked at exit" whenever a
  player is still playing (or was stopped one frame before) at quit; its own
  `AudioStreamGenerator` does the same, so this is the engine's shutdown
  order, not a klattsch leak.
- **Step 5:** the `Klattsch` engine singleton (`#[class(singleton)]`) reads
  `res://addons/klattsch/cmudict.dict` with `FileAccess` and parses it on a
  Rust thread: `dictionary_loaded` after 55 ms in a release build (449 ms
  debug). Before it is ready, `say()` speaks from the lexicon and
  letter-to-sound rules and reports the sounded-out words through `error`.
  English from Godot matches `klattsch-cli --english` exactly; word lists
  and banks load from `user://`; Japanese uses a ja-* bank. The stream's
  Voice properties default to klattsch's own CompileOptions defaults, so an
  untouched stream compiles exactly like `compile_string` with no options.
  All of this was checked in **exported** Windows builds (debug and
  release): the export preset needs `*.dict` in the non-resource filter,
  and exported binaries refuse a scene on the command line (the demo scene
  switches to `--scene=...` given after `--`).
- **Step 6:** `Klattsch.save_wav` writes files byte-identical to
  `klattsch-cli` (and so to the JS CLI) for plain phonemes, a three-voice
  chord, 22050 Hz, English (comment = the English text) and Japanese. A
  `KlattschSample` in exact mode reaches the bus bit-identical to its
  samples, also after `seek`; game mode honors pitch scale. Godot ends a
  custom stream, and the player emits `finished`, when `mix` returns fewer
  frames than requested; `is_playing()` going false is not enough.
  `to_audio_stream_wav()` differs from the float samples by at most half a
  16-bit step (1/65534).
- **Step 7:** the addon (`addon/`, `tools/package_addon.py`) unzipped into
  a fresh 4.7.2 project passes the step 4 and 5 checks in the editor and in
  exported debug and release builds. The CI-style Windows DLL (static C
  runtime) imports only system DLLs. Testing the exports exposed a crash on
  **exit**: a GDScript lambda connected to an engine singleton's signal
  (`Klattsch.dictionary_loaded.connect(func(): ...)`) is still connected
  when the library unloads, after GDScript has shut down, and freeing it
  crashes. The extension now disconnects everything from the singleton's
  signals at the `MainLoop` deinit stage (Godot 4.5+), while scripts are
  alive. (The step 5 exported runs had printed their results and then hit
  this crash; the exit code was not checked then.) CI itself has not run
  yet: it runs on the first push.
- **CI, first run:** parity passed on Windows and Ubuntu, and all three
  libraries built, but macOS (Apple Silicon) parity failed in the last bit
  of `Math.sin`/`Math.cos`: Node's V8 there is compiled with FMA contraction.
  A copy of the port's `sin` with V8's `a*b + c` steps fused reproduced all
  15 reported Node values, so the port now fuses the same 14 expressions on
  Apple arm64 (`FUSED` / `mad` in `src/fdlibm.rs`), matching Node per
  platform like `pow`.
- **Step 8 (editor extras):** `Klattsch.highlight` and
  `KlattschSyntaxHighlighter` (any TextEdit) wrap the core's
  `build_highlight`; the script editor gets `KlattschEditorSyntaxHighlighter`
  ("klattsch phonemes" under Edit > Syntax Highlighter; Godot lists every
  registered highlighter there but fills the menu only as it opens). The
  core highlights a whole text at once (block comments and `[bank=...]`
  reach later lines) while Godot asks per line and caches lines, so the
  highlighter re-highlights when the text changes and then clears Godot's
  line cache (deferred) so lines an edit elsewhere recoloured repaint.
  The audition panel comes from a gdext `tool` EditorPlugin, registered
  automatically (nothing to enable). gdext makes non-`tool` classes
  *runtime classes*, placeholders in the editor whose methods cannot run,
  so the `Klattsch` singleton, `KlattschUtterance` and `KlattschSample`
  (with its playbacks) are now `tool`; `KlattschStream` and
  `KlattschPlayer` stay runtime classes and the panel reads their
  properties with `get()`. The panel pre-renders with the node's Voice
  settings (`say_japanese`'s bank fallback included) and plays on an
  AudioStreamPlayer the plugin owns. Checked in the editor (headless, via
  `addons/klattsch_editor_test`): each mode plays exactly
  `Klattsch.render(Klattsch.compile_with(...))` of the same settings, for a
  player's own stream, a target's stream (intonation off) and a bare
  KlattschStream. Found on the way: closing the editor while an audition
  still played crashed it (the audio server freed the KlattschSample
  playback after the library unloaded); the plugin now stops and drops it
  in `exit_tree`. A game that quits mid-sample is unaffected (Godot only
  reports the leaked playback).
