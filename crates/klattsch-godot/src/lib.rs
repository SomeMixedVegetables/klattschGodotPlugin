// klattsch for Godot 4 (gdext).
//
// Built to docs/klattsch-godot-build-spec.docx (steps 1-8):
// KlattschStream, an AudioStream whose playback renders klattsch_core's
// Player on Godot's audio thread, in game (resampled) or exact mode, with
// speak / set_param / reset control from GDScript; Klattsch.compile and
// KlattschUtterance to make something to speak; and KlattschPlayer, the node
// that speaks through a stock audio player with queueing and timing
// signals; and the Klattsch singleton (English and Japanese, the CMU
// dictionary loaded in the background, the runtime lexicon, banks through
// FileAccess, pre-rendering to KlattschSample and WAV files); and editor
// extras (KlattschSyntaxHighlighter for phoneme strings, and an Inspector
// panel to audition a voice). The playbacks record timing statistics so
// the test project (godot/) can check for stalls.
//
// Threading rules (from the spec):
//   - Playback classes have no #[func] methods. Godot alone calls them.
//   - mix touches only PlaybackCore: no Godot calls, no allocation, no
//     locks of our own, panics caught and turned into silence.
//   - The main thread and the audio thread share only SharedState atomics.

// gdext's #[derive(GodotClass)] expands `#[init]` fields to `base: base`.
#![allow(clippy::redundant_field_names)]

use std::sync::OnceLock;
use std::thread::ThreadId;

use godot::prelude::*;

mod alloc_check;
mod editor;
mod highlighter;
mod klattsch;
mod playback;
mod player_node;
mod sample;
mod stream;
mod utterance;

struct KlattschExtension;

// The thread Godot initializes extensions on: the main thread.
static MAIN_THREAD: OnceLock<ThreadId> = OnceLock::new();

fn on_main_thread() -> bool {
    MAIN_THREAD.get() == Some(&std::thread::current().id())
}

#[gdextension]
unsafe impl ExtensionLibrary for KlattschExtension {
    fn on_stage_init(_stage: InitStage) {
        let _ = MAIN_THREAD.set(std::thread::current().id());
    }

    fn on_stage_deinit(stage: InitStage) {
        if stage == InitStage::MainLoop {
            klattsch::Klattsch::disconnect_all();
        }
    }
}
