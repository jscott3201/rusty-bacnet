//! Heap placement for the futures an MCP request nests (#2656).
//!
//! A request runs on its own task, inside the SDK's dispatch over every
//! kind of request, then this crate's handler, the panic guard, the tool
//! call with its budget, account and cancel, and the tool itself. An
//! unoptimized build gives each future an async fn awaits at least one
//! stack slot sized to the whole future, in that fn's poll frame, often
//! two or three, and an async fn's arguments are held twice in its own
//! future. So every layer paid again for the futures below it: on dev at
//! 079a60156 a tool call needed up to 980 KiB of a test thread's stack in
//! a debug build, against the 1 MiB CI's stack step gives it.
//!
//! [`boxed`] creates such a future in a frame of its own, which returns
//! before polling starts, and keeps it on the heap, so the layer awaiting
//! it holds a pointer. The boxes are one per request (the handler the SDK
//! awaits), one per tool call (the tool's own future), one per write
//! attempt and one per resource read (the read itself). The client's
//! per-request futures, further in, are left as they are.

use std::future::Future;
use std::pin::Pin;

/// Create the future `make` returns and move it to the heap.
///
/// Awaiting the box costs the caller a pointer where awaiting the future
/// itself would cost the whole future, in the caller's state and poll
/// frame. Not inlined, so the full-size temporary stays in this frame
/// in an optimized build too.
#[inline(never)]
pub(crate) fn boxed<F: Future>(make: impl FnOnce() -> F) -> Pin<Box<F>> {
    Box::pin(make())
}
