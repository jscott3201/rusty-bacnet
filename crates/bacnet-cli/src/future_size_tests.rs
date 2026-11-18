//! Bounds on the sizes of the futures a one-shot command nests (#2656).
//!
//! The one-shot dispatch awaits each command in an arm of its own, and a
//! debug build gives every future an async fn awaits one or two stack slots
//! the size of that whole future, in the fn's poll frame, without sharing
//! them between arms. On dev at 171799d3c `execute_command`'s future was
//! 6,984 bytes for a BACnet/SC client and its poll frame 353,600 bytes, on
//! the main thread under every command: a `read` over BACnet/SC needed
//! 640 KiB of a 1 MiB stack on macOS. Now each command's future, its
//! target's resolution and each session in `cli_main` are created in a frame
//! of its own and boxed (`heap::boxed`), and that `read` needs 288 KiB.
//!
//! A command arm that awaits its future inline puts that future back in
//! the dispatch's state, which these bounds catch. They do not catch a box
//! made from an already built future (`Box::pin(cmd(..))`), which leaves the
//! state small and puts a full-size temporary back in the poll frame: only
//! CI's 1 MiB stack step sees that. The bounds are the sizes with
//! this change (debug, 64-bit, Rust 1.99.0) on macOS arm64, plus 32 bytes.
//! A change that has to grow a future past its bound first shows CI's
//! 1 MiB stack step still passes on Windows, then moves the bound. A
//! toolchain bump can move these layouts too, either way: one that trips a
//! bound means measuring the sizes on dev again with the new compiler and
//! setting the bounds from those, not a regression to chase here.
//!
//! `cli_main` holds the command line, whose paths are 8 bytes wider on
//! Windows (its `OsString` keeps a UTF-8 flag beside the bytes), so its
//! bound is checked on Linux and macOS only; the dispatch holds no path.

use std::future::Future;

use bacnet_transport::bip::BipTransport;

use super::*;

/// `execute_command`'s future was 472 bytes for B/IP and BACnet/SC clients.
const DISPATCH_BOUND: usize = 472 + 32;

/// `execute_bip_command`'s future was 232 bytes.
const BIP_DISPATCH_BOUND: usize = 232 + 32;

/// `cli_main`'s future was 2,448 bytes.
#[cfg(not(windows))]
const MAIN_BOUND: usize = 2_448 + 32;

/// The size of the future `f` returns, without calling it.
#[cfg(not(windows))]
fn returns1<A, F: Future>(_: impl FnOnce(A) -> F) -> usize {
    size_of::<F>()
}

/// The size of the future a function of three arguments returns, without
/// calling it.
fn returns3<A, B, C, F: Future>(_: impl FnOnce(A, B, C) -> F) -> usize {
    size_of::<F>()
}

/// [`returns3`] for a function of four arguments.
fn returns4<A, B, C, D, F: Future>(_: impl FnOnce(A, B, C, D) -> F) -> usize {
    size_of::<F>()
}

fn within(name: &str, size: usize, bound: usize) {
    assert!(
        size <= bound,
        "{name}'s future is {size} bytes, over {bound}"
    );
}

#[test]
fn the_one_shot_futures_stay_within_their_bounds() {
    within(
        "execute_command::<BipTransport>",
        returns3(execute_command::<BipTransport>),
        DISPATCH_BOUND,
    );
    #[cfg(feature = "sc-tls")]
    within(
        "execute_command::<ScTransport>",
        returns3(
            execute_command::<
                bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>,
            >,
        ),
        DISPATCH_BOUND,
    );
    within(
        "execute_bip_command",
        returns4(execute_bip_command),
        BIP_DISPATCH_BOUND,
    );
    #[cfg(not(windows))]
    within("cli_main", returns1(cli_main), MAIN_BOUND);
}
