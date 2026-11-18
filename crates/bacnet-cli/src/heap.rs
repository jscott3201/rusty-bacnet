//! Heap placement for the futures a one-shot command nests (#2656).
//!
//! An unoptimized build gives every future an async fn awaits one or two
//! stack slots the size of that whole future, in the poll frame of the fn,
//! and does not share those slots between the arms of a `match`. So the
//! one-shot dispatch, which awaits each command's future in an arm of its
//! own, had a poll frame that summed them all: 353,600 bytes on macOS for
//! a BACnet/SC client, on the main thread, under every command. A `read`
//! over BACnet/SC needed 640 KiB of the main thread's stack in a debug
//! build, against the 1 MiB Windows gives it.
//!
//! [`boxed`] creates such a future in a frame of its own, which returns
//! before polling starts, and keeps it on the heap, so the fn awaiting it
//! holds a pointer. That is one box per command, its target's resolution
//! and the session around it, never per request: the client's request
//! futures, further in, are left as they are.

use std::future::Future;
use std::pin::Pin;

/// Create the future `make` returns and move it to the heap.
///
/// Awaiting the box costs the caller a pointer where awaiting the future
/// itself would cost the whole future, in the caller's state and poll
/// frame. Not inlined, so the full-size temporary stays in this frame in
/// an optimized build too.
#[inline(never)]
pub(crate) fn boxed<F: Future>(make: impl FnOnce() -> F) -> Pin<Box<F>> {
    Box::pin(make())
}
