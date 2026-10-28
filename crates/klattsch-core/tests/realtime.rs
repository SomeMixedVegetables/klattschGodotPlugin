// Checks that the audio-thread side of the player never allocates or frees
// memory, using a global allocator that counts calls made while a
// thread-local flag is set.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use klattsch_core::*;

struct Counting;

// Both per thread: the test harness runs tests (which allocate) in
// parallel, and a shared counter would count their calls too.
thread_local! {
    static WATCHING: Cell<bool> = const { Cell::new(false) };
    static CALLS: Cell<usize> = const { Cell::new(0) };
}

fn note() {
    if WATCHING.with(Cell::get) {
        CALLS.with(|c| c.set(c.get() + 1));
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// Counts allocator calls made by `f` on this thread.
fn allocations(f: impl FnOnce()) -> usize {
    let before = CALLS.with(Cell::get);
    WATCHING.with(|w| w.set(true));
    f();
    WATCHING.with(|w| w.set(false));
    CALLS.with(Cell::get) - before
}

#[test]
fn audio_thread_does_not_allocate() {
    let sr = 48000.0;
    let compiled = compile_string("b140 HH AH L OW , W ER L D [voice=1] b200 AA IY UW", &CompileOptions::default()).unwrap();
    let (handle, mut player) = player(sr, ProcessorOptions::default(), 64).unwrap();
    let mut block = vec![0.0f32; 512];
    let mut a = vec![0.0f32; 256];
    let mut b = vec![0.0f32; 256];
    let mut heard = false;

    for round in 0..20 {
        // Game thread: prepare and send every kind of command, including to
        // voices that do not exist yet (within the reserved slots).
        let voice = round % player::RESERVED_VOICES;
        handle.schedule(Some(voice), compiled.schedule(), Some(handle.current_time() - 0.01)).unwrap();
        handle.schedule(Some(1), &compiled.voices[1].schedule, None).unwrap();
        handle.frame(Some(voice), PartialParams::new().with(Param::F0, 150.0 + round as f64), Some(20.0)).unwrap();
        // Appends: one that waits, one that replaces it, and one on a silent voice.
        handle.append(Some(voice), compiled.schedule(), compiled.total_ms).unwrap();
        handle.append(Some(voice), &compiled.voices[1].schedule, compiled.total_ms).unwrap();
        handle.append(Some(3), compiled.schedule(), 5.0).unwrap();
        if round % 5 == 4 {
            handle.reset(None, Some(PartialParams::new().with(Param::Voicing, 1.0))).unwrap();
            handle.reset(Some(2), None).unwrap();
        }

        // Audio thread: applying the commands and rendering must not touch
        // the allocator (old schedules go back to the handle).
        let n = allocations(|| {
            player.process_mix(&mut block, player::RESERVED_VOICES);
            player.process(256, &mut [&mut a[..], &mut b[..]]);
            player.process_mix(&mut block[..100], 3);
        });
        assert_eq!(n, 0, "round {round}: {n} allocator calls on the audio thread");
        heard |= block.iter().chain(&a).any(|x| *x != 0.0);
    }
    assert!(heard, "the voices were audible");
    handle.collect_garbage();
}

#[test]
fn counting_allocator_works() {
    assert!(allocations(|| drop(vec![1u8; 100])) >= 2);
}
