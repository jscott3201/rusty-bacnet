//! A counting global allocator for allocation tests (#1881): the one copy
//! that bacnet-services' and bacnet-mstp-core's allocation tests include by
//! `#[path]`. Including it installs the allocator, so each test binary that
//! does is one of its own.
//!
//! It counts every allocation on the thread that makes it. `realloc` and
//! `alloc_zeroed` keep their default bodies, which call `alloc`, so a buffer
//! that grows counts once per growth.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

std::thread_local! {
    static THREAD_ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

// SAFETY: every allocation and free goes to System unchanged, so the
// GlobalAlloc contract holds as System's does. The count is a const
// thread-local Cell with no destructor, so it never allocates, and `with`
// can't fail or unwind out of the allocator.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        THREAD_ALLOCATIONS.with(|count| count.set(count.get().saturating_add(1)));
        // SAFETY: the caller upholds alloc's contract (a non-zero-size
        // layout), which is System.alloc's; it is passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: every block came from System.alloc above with this layout
        // (the caller's contract), so System.dealloc may free it.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Run `f`; return what it returned and the allocations it made on this
/// thread.
pub fn counted<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let before = THREAD_ALLOCATIONS.with(Cell::get);
    let value = f();
    (value, THREAD_ALLOCATIONS.with(Cell::get) - before)
}
