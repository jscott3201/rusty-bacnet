//! Bounds on the sizes of the futures an MCP request nests (#2656).
//!
//! The SDK runs each request on a task of its own, inside two task-locals
//! and its dispatch over every kind of request, and a debug build keeps a
//! copy of the future each of those polls, at every layer, as it does at
//! each layer of this crate under them: the panic guard, the tool call's
//! budget, account and cancel, and the tool. On dev at 079a60156 every
//! layer held the one below inline: the tool-call handler's future was
//! 25,784 bytes, the dispatch's 27,768 and `tools::call`'s 12,344, and the
//! deepest MCP test needed 980 KiB of stack, against the 1 MiB CI's stack
//! step gives Windows, whose frames need 44 to 60 KiB more than macOS's.
//! Now the three handlers that reach the network (`call_tool`,
//! `read_resource` and `listen`) hand the SDK a box, and each tool's and
//! each resource read's own future is a box too, and no MCP test needs
//! more than 388 KiB on macOS. The small handlers (`list_tools`,
//! `get_prompt` and the like) stay async fns: the dispatch's bound covers
//! them.
//!
//! The bounds are the sizes with that change (debug, 64-bit, Rust 1.99.0,
//! rmcp 3.9.0) on macOS, plus 32 bytes. Linux and Windows x86_64 are the
//! same or a few bytes smaller (`tools::call` 896, `call_write` 3,208, a
//! write attempt 760; the dispatch 3,328 on all three), so the bounds hold there without a cfg. A change
//! that has to grow a future past its bound first shows the stack step
//! still passes on Windows, then moves the bound. A toolchain or SDK bump
//! can move these layouts too, either way: one that trips a bound means
//! measuring the sizes on dev again with the new compiler and setting the
//! bounds from those, not a regression to chase here.

use std::future::Future;

use bacnet_transport::test_transport::TestTransport;
use rmcp::service::Service;
use rmcp::{RoleServer, ServerHandler};

use crate::McpServer;

type T = TestTransport;
type Server = McpServer<T>;

/// A box: the three network handlers hand the SDK nothing else.
const HANDLER_BOUND: usize = size_of::<usize>();

/// The SDK's dispatch over this server's handlers was 3,328 bytes.
const DISPATCH_BOUND: usize = 3_328 + 32;

/// `serve_call`'s future, behind `call_tool`'s box, was 4,176 bytes.
const SERVE_CALL_BOUND: usize = 4_176 + 32;

/// `serve_read`'s future, behind `read_resource`'s box, was 528 bytes.
const SERVE_READ_BOUND: usize = 528 + 32;

/// `resources::read`'s future, behind `serve_read`'s box, was 656 bytes.
const READ_BOUND: usize = 656 + 32;

/// `tools::call`'s future was 904 bytes.
const CALL_BOUND: usize = 904 + 32;

/// `tools::call_write`'s future was 3,224 bytes.
const CALL_WRITE_BOUND: usize = 3_224 + 32;

/// A write attempt's future was 768 bytes.
const ATTEMPT_BOUND: usize = 768 + 32;

/// The size of the future `f` returns, without calling it.
fn returns<A, B, C, F: Future>(_: impl FnOnce(A, B, C) -> F) -> usize {
    size_of::<F>()
}

#[test]
fn the_request_futures_stay_within_their_bounds() {
    let handlers = [
        ("call_tool", returns(<Server as ServerHandler>::call_tool)),
        (
            "read_resource",
            returns(<Server as ServerHandler>::read_resource),
        ),
        ("listen", size_of_listen()),
    ];
    for (name, size) in handlers {
        assert!(
            size <= HANDLER_BOUND,
            "the {name} handler's future is {size} bytes, not a box"
        );
    }
    let (serve_call, serve_read) = Server::serve_sizes();
    let bounded = [
        (
            "the dispatch",
            returns(<Server as Service<RoleServer>>::handle_request),
            DISPATCH_BOUND,
        ),
        ("serve_call", serve_call, SERVE_CALL_BOUND),
        ("serve_read", serve_read, SERVE_READ_BOUND),
        (
            "resources::read",
            returns(crate::resources::read::<T>),
            READ_BOUND,
        ),
        ("tools::call", call_size(), CALL_BOUND),
        ("tools::call_write", call_write_size(), CALL_WRITE_BOUND),
        ("a write attempt", attempt_size(), ATTEMPT_BOUND),
    ];
    for (name, size, bound) in bounded {
        assert!(
            size <= bound,
            "{name}'s future is {size} bytes, over {bound}"
        );
    }
}

fn size_of_listen() -> usize {
    fn returns<A, B, F: Future>(_: impl FnOnce(A, B) -> F) -> usize {
        size_of::<F>()
    }
    returns(<Server as ServerHandler>::listen)
}

fn call_size() -> usize {
    fn returns<A, B, C, D, F: Future>(_: impl FnOnce(A, B, C, D) -> F) -> usize {
        size_of::<F>()
    }
    returns(super::call::<T>)
}

fn call_write_size() -> usize {
    fn returns<A, B, C, D, E, G, F: Future>(_: impl FnOnce(A, B, C, D, E, G) -> F) -> usize {
        size_of::<F>()
    }
    returns(super::call_write::<T>)
}

fn attempt_size() -> usize {
    fn returns<A, B, C, D, E, G, H, F: Future>(_: impl FnOnce(A, B, C, D, E, G, H) -> F) -> usize {
        size_of::<F>()
    }
    returns(super::attempt::<T>)
}
