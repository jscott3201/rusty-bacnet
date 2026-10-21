//! Running a test again when its closing release probe loses the port (#1070).
//!
//! A test that stops a node and then binds the node's old address, to prove the
//! node let it go, leaves a gap between the two in which another process can
//! take the port. The probe then fails although the node released the port.
//! [`rerun_on_lost_port`] runs such a test again from the start; the node binds
//! fresh ports each run, so one that really keeps its port fails every run.
//!
//! The benchmarks' standalone-binary tests include this file by path.

use std::io;

/// How many times [`rerun_on_lost_port`] runs a test.
const ATTEMPTS: usize = 8;

/// Whether a bind failed because another socket holds the port. This is
/// `bacnet-transport`'s `port_ownership::lost_to_another_socket`, which is
/// private to that crate's own tests. Only an OS error counts, and Windows
/// refuses a SO_REUSEADDR bind to a port held without sharing with
/// `WSAEACCES` instead of `WSAEADDRINUSE`.
fn lost_to_another_socket(err: &io::Error) -> bool {
    err.raw_os_error().is_some()
        && (err.kind() == io::ErrorKind::AddrInUse
            || (cfg!(windows) && err.kind() == io::ErrorKind::PermissionDenied))
}

/// Runs `run` until it passes. `run` is a whole test whose result is its
/// release probe's bind. A probe that another socket beat to the port runs the
/// test again, a bounded number of times; any other failure, or a lost port on
/// the last run, panics.
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
            Err(err) => panic!("release probe failed on run {attempt} of {ATTEMPTS}: {err}"),
        }
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
