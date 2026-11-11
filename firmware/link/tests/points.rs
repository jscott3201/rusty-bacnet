//! Runs the points image's scenario natively and checks what it reports,
//! then runs it again in the image's allocator over an arena of the image's
//! size and measures what the list, the build and the requests take. This
//! is where CI verifies the scenario's behaviour: CI only links the
//! bare-metal image.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use bacnet_objects::points::build_database;
use firmware_link::objects::APDUS;
use firmware_link::points::{ARENA, READ_APDUS, expected, identity, list, run};
use linked_list_allocator::LockedHeap;

/// The whole summary, against the one written out by hand
/// (`firmware_link::points::expected`, which explains each answer). Its
/// objects part is the objects image's own expected summary: a database the
/// builder makes answers the objects image's requests as the one that image
/// assembles by hand does.
#[test]
fn scenario_summary() {
    assert_eq!(run(list().unwrap(), &APDUS, &READ_APDUS), expected());
}

/// The arena for [`heap_fits_the_arena`]. The image's allocator,
/// `embedded-alloc`'s linked-list first fit, is this heap behind a critical
/// section.
static ARENA_HEAP: LockedHeap = LockedHeap::empty();
/// The most the arena had in use at once since it was last reset, in
/// octets, each block rounded up as the allocator rounds it.
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

/// The octets the arena has in use now.
fn used() -> usize {
    ARENA_HEAP.lock().used()
}

/// Start the peak again from what is in use now.
fn reset_peak() {
    PEAK.store(used(), Ordering::Relaxed);
}

/// Runs the scenario with its allocations in an arena of the image's size,
/// in the image's allocator: it must fit, succeed and give back everything
/// it took. First it measures the build alone, twice: as the image builds,
/// taking the list by value, and as a host that keeps its list would, from
/// a clone, so the list and the database are both live. Prints, which
/// nextest shows (.config/nextest.toml), what the identity and the list
/// hold, each build's peak and what it leaves, and the whole scenario's
/// peak, requests included.
///
/// A 64-bit host's blocks are at least as large as a 32-bit target's: the
/// allocator rounds each up to two words and to word alignment, and blocks
/// that hold pointers are twice the size. So each figure here bounds the
/// image's.
#[test]
fn heap_fits_the_arena() {
    let arena: &'static mut [MaybeUninit<u8>] = Box::leak(Box::new([MaybeUninit::uninit(); ARENA]));
    ARENA_HEAP.lock().init_from_slice(arena);
    IN_ARENA.set(true);

    // The image's build: the builder takes the list by value and moves its
    // strings into the objects, and the list's own buffer goes once every
    // point is built, before the database is assembled.
    let identity = identity().unwrap();
    let with_identity = used();
    let points = list().unwrap();
    let listed = used() - with_identity;
    reset_peak();
    let db = build_database(points, &identity).unwrap();
    let build_peak = PEAK.load(Ordering::Relaxed);
    let database = used();
    drop(db);

    // A host that keeps its list builds from a clone.
    let points = list().unwrap();
    reset_peak();
    let db = build_database(points.clone(), &identity).unwrap();
    let kept_peak = PEAK.load(Ordering::Relaxed);
    let kept = used();
    drop(db);
    drop(points);
    drop(identity);
    let after_builds = used();

    // The whole scenario, as the image runs it.
    reset_peak();
    let summary = run(list().unwrap(), &APDUS, &READ_APDUS);
    IN_ARENA.set(false);
    let errors = summary.errors + summary.objects.errors;
    drop(summary);
    let peak = PEAK.load(Ordering::Relaxed);
    let left = used();

    println!(
        "points scenario heap on this host, in octets, the identity included in each but \
         the list's: identity {with_identity}, list {listed}; built from the list, peak \
         {build_peak}, database {database}; built from a clone of a kept list, peak \
         {kept_peak}, list and database {kept}; scenario peak {peak} of {ARENA}"
    );
    assert!(
        !EXHAUSTED.load(Ordering::Relaxed),
        "the scenario needs more than {ARENA} octets of heap"
    );
    assert_eq!(
        after_builds, 0,
        "the builds kept {after_builds} octets of heap"
    );
    assert_eq!(errors, 0, "the scenario failed in the arena");
    assert_eq!(left, 0, "the scenario kept {left} octets of heap");
}
