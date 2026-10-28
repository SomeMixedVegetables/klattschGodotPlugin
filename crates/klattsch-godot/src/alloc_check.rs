// With the `alloc-check` feature, a global allocator counts allocator calls
// made inside PlaybackCore::mix (the spec's step 2 check that the audio
// thread never allocates). Without it, these are no-ops.

#[cfg(feature = "alloc-check")]
mod imp {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

    static CALLS: AtomicU64 = AtomicU64::new(0);

    thread_local! {
        static IN_MIX: Cell<bool> = const { Cell::new(false) };
    }

    fn note() {
        if IN_MIX.with(Cell::get) {
            CALLS.fetch_add(1, Relaxed);
        }
    }

    struct Counting;

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

    pub fn enter_mix() {
        IN_MIX.with(|f| f.set(true));
    }

    pub fn leave_mix() {
        IN_MIX.with(|f| f.set(false));
    }

    // Allocator calls seen inside mix so far.
    pub fn mix_allocations() -> i64 {
        CALLS.load(Relaxed) as i64
    }
}

#[cfg(not(feature = "alloc-check"))]
mod imp {
    pub fn enter_mix() {}
    pub fn leave_mix() {}
    // -1: not counted in this build.
    pub fn mix_allocations() -> i64 {
        -1
    }
}

pub use imp::*;
