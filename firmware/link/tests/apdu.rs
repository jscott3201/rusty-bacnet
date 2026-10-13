//! Runs the apdu image's scenario natively and checks what it reports,
//! then runs it again in the image's allocator over an arena of the image's
//! size. This is where CI verifies the scenario's behaviour: CI only links
//! the bare-metal image, and its emulation build, which checks the same
//! summary on an emulated CPU, runs by hand (docs/ci.md, "Emulation
//! (manual)").

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use firmware_link::apdu::{expected, run, ARENA, DATAGRAMS};
use linked_list_allocator::LockedHeap;

/// The whole summary, against the one the emulation build checks too
/// (`firmware_link::apdu::expected`, which explains each value).
#[test]
fn scenario_summary() {
    assert_eq!(run(&DATAGRAMS), expected());
}

/// The arena for [`heap_fits_the_arena`]. The image's allocator,
/// `embedded-alloc`'s linked-list first fit, is this heap behind a critical
/// section.
static ARENA_HEAP: LockedHeap = LockedHeap::empty();
/// The most the arena had in use at once, in octets, each block rounded up
/// as the allocator rounds it.
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Set when the arena couldn't serve an allocation, which then came from
/// the system instead.
static EXHAUSTED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Whether this thread's allocations come from the arena.
    static IN_ARENA: Cell<bool> = const { Cell::new(false) };
}

/// The test binary's allocator: the system's, except on a thread that has
/// set [`IN_ARENA`], whose allocations come from [`ARENA_HEAP`]. A block goes
/// back to whichever heap its address lies in.
struct Switch;

#[global_allocator]
static SWITCH: Switch = Switch;

// SAFETY: each block comes from, and goes back to, one of two allocators
// that uphold `GlobalAlloc`'s contract: the system's, or the arena's heap,
// which hands out only octets of its arena and is locked for each call. A
// block's address says which heap it came from: the arena is a system block
// that is never freed, so the system never hands its octets out again.
#[allow(unsafe_code)]
unsafe impl GlobalAlloc for Switch {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if IN_ARENA.try_with(Cell::get).unwrap_or(false) {
            let mut heap = ARENA_HEAP.lock();
            if let Ok(block) = heap.allocate_first_fit(layout) {
                PEAK.fetch_max(heap.used(), Ordering::Relaxed);
                return block.as_ptr();
            }
            EXHAUSTED.store(true, Ordering::Relaxed);
        }
        // SAFETY: the layout is the caller's, nonzero in size as `alloc`
        // requires.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        let mut heap = ARENA_HEAP.lock();
        if (heap.bottom()..heap.top()).contains(&block) {
            // SAFETY: the block lies in the arena, so the arena's heap
            // allocated it, with this layout, as the caller guarantees.
            unsafe { heap.deallocate(NonNull::new_unchecked(block), layout) };
            return;
        }
        drop(heap);
        // SAFETY: the block lies outside the arena, so the system allocated
        // it, with this layout, as the caller guarantees.
        unsafe { System.dealloc(block, layout) }
    }
}

/// Runs the scenario with its allocations in an arena of the image's size,
/// in the image's allocator: it must fit, succeed, give back everything it
/// took, and peak at no more than half the arena, the margin
/// [`ARENA`]'s docs promise. Prints the peak, which nextest shows
/// (.config/nextest.toml).
///
/// A 64-bit host's blocks are at least as large as a 32-bit target's: the
/// allocator rounds each up to two words and to word alignment, and the
/// blocks that hold pointers (`bytes`' shared-buffer headers) are twice the
/// size. So the peak here bounds the image's.
#[test]
fn heap_fits_the_arena() {
    let arena: &'static mut [MaybeUninit<u8>] = Box::leak(Box::new([MaybeUninit::uninit(); ARENA]));
    ARENA_HEAP.lock().init_from_slice(arena);
    IN_ARENA.set(true);
    let summary = run(&DATAGRAMS);
    IN_ARENA.set(false);
    let errors = summary.errors;
    drop(summary);
    let peak = PEAK.load(Ordering::Relaxed);
    let left = ARENA_HEAP.lock().used();
    println!("apdu scenario heap: peak {peak} of {ARENA} octets on this host");
    assert!(
        !EXHAUSTED.load(Ordering::Relaxed),
        "the scenario needs more than {ARENA} octets of heap"
    );
    assert_eq!(errors, 0, "the scenario failed in the arena");
    assert_eq!(left, 0, "the scenario kept {left} octets of heap");
    assert!(
        peak * 2 <= ARENA,
        "the scenario's heap peak of {peak} octets is over half the {ARENA}-octet arena; \
         grow firmware_link::apdu::ARENA, and the image's RAM budget with it"
    );
}
