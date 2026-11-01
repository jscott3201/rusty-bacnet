//! Running a test again when another process takes a port it reserved
//! (#1070, #2225).
//!
//! Some tests must name a port before the code under test binds it: a
//! closing release probe binds a stopped node's old address to prove the node
//! let it go, and a B/IP-M group's port or a BBMD's own BDT row is fixed
//! before the transport starts. Between the reservation and the bind, another
//! process can take the port, and the bind then fails through no fault of the
//! code under test. [`rerun_on_lost_port`] runs such a test again from the
//! start on fresh ports; code that really keeps or refuses a port fails every
//! run, and the last run reports it.
//!
//! This one file is the shared helper: the crates' tests and the benchmarks'
//! standalone-binary tests include it by path, and `bacnet-transport`'s
//! `port_ownership` takes its classification from here.

use std::io;

/// How many times [`rerun_on_lost_port`] runs a test.
pub const ATTEMPTS: usize = 8;

/// Whether a bind failed because another socket holds the port.
///
/// Only an OS error counts, so a transport's own `AddrInUse` (a VMAC
/// collision) does not. Windows refuses a SO_REUSEADDR bind to a port held
/// without sharing with `WSAEACCES` instead of `WSAEADDRINUSE`. B/IP wraps a
/// listener's bind error with the address it tried (a message starting
/// `could not bind the B/IP `), which keeps the kind but drops the OS code;
/// that wrapper counts too, and no other message does.
///
/// On macOS a SO_REUSEADDR bind of a specific address succeeds beside another
/// socket's plain wildcard bind of the port (see `port_ownership` in
/// `bacnet-transport`), so nothing there reports that loss.
pub fn lost_to_another_socket(err: &io::Error) -> bool {
    let taken = err.kind() == io::ErrorKind::AddrInUse
        || (cfg!(windows) && err.kind() == io::ErrorKind::PermissionDenied);
    taken && (err.raw_os_error().is_some() || wrapped_bind(err))
}

/// Whether `err` is a listener bind error B/IP wrapped with its address
/// (`group_listener` and the per-address broadcast listeners).
fn wrapped_bind(err: &io::Error) -> bool {
    err.get_ref()
        .is_some_and(|inner| inner.to_string().starts_with("could not bind the B/IP "))
}

/// Runs `run` until it passes. `run` is a whole test; an `Err` it returns
/// is a bind that another socket may have beaten to the port, such as a
/// release probe or a transport's start (see [`bind_error`]). A lost port
/// runs the test again, a bounded number of times; any other failure, or a
/// lost port on the last run, panics with the error. A panic inside `run`
/// fails the test at once.
///
/// On macOS a SO_REUSEADDR bind of 127.0.0.1 succeeds beside another socket's
/// wildcard bind (see `start_on_free_port` in `bacnet-transport`). A probe that
/// sets it, as Tokio's `TcpListener::bind` does on Unix, may then pass on a
/// lost port, which is harmless, but it proves release only where the node
/// bound that same specific address.
pub async fn rerun_on_lost_port(mut run: impl AsyncFnMut() -> io::Result<()>) {
    for attempt in 1..=ATTEMPTS {
        match Box::pin(run()).await {
            Ok(()) => return,
            Err(err) if attempt < ATTEMPTS && lost_to_another_socket(&err) => {}
            Err(err) => panic!("run {attempt} of {ATTEMPTS} failed: {err}"),
        }
    }
}

/// A transport or server start error as the I/O error
/// [`rerun_on_lost_port`] judges: a transport error is its own I/O error,
/// which may be a lost port; anything else becomes one that is never a lost
/// port, so it fails the test.
#[allow(dead_code)] // Not every test binary that includes this file starts a transport.
pub fn bind_error(err: bacnet_types::error::Error) -> io::Error {
    match err {
        bacnet_types::error::Error::Transport(err) => err,
        other => io::Error::other(other.to_string()),
    }
}

/// Binds with `bind` until it succeeds or `within` passes, for an owner whose
/// release finishes in the background, such as a dropped node's aborted
/// tasks (#1419). On timeout it returns the last bind error, so a port lost
/// to another socket reruns the test under [`rerun_on_lost_port`] while one
/// the owner kept fails on the last run.
#[allow(dead_code)] // Not every test binary that includes this file polls.
pub async fn bind_eventually<T>(
    within: std::time::Duration,
    mut bind: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        match bind() {
            Ok(bound) => return Ok(bound),
            Err(err) if tokio::time::Instant::now() >= deadline => return Err(err),
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
        }
    }
}
