//! Keeping an unshared UDP port to its socket, as far as each OS allows.
//!
//! A B/IP or B/IPv6 socket on an ephemeral port binds the wildcard address
//! without SO_REUSEADDR (#892). How private that leaves the port depends on
//! the OS:
//!
//! - Linux refuses any other bind to the port, whatever options it sets.
//! - Windows lets another socket bind a more specific address on the same port
//!   (127.0.0.1:P beside a wildcard 0.0.0.0:P) and take the unicast sent
//!   there, unless the first socket set SO_EXCLUSIVEADDRUSE, which this module
//!   does (#950).
//! - macOS and the other BSDs refuse a plain bind there, but a socket that sets
//!   SO_REUSEADDR itself may still bind the more specific address. They have
//!   no option to stop it, so that case remains.
//!
//! The test-only helpers here cover tests that lose a port to another socket:
//! a probed port taken before the test binds it (`lost_port`), and a loopback
//! connect whose OS-chosen source port is still in use (`connect_loopback`).

use std::io;

#[cfg(test)]
use crate::port::{ReceivedNpdu, TransportPort};
#[cfg(test)]
use bacnet_types::error::Error;
#[cfg(test)]
use std::net::SocketAddr;
#[cfg(test)]
use std::time::Duration;
#[cfg(test)]
use tokio::sync::mpsc;

/// Keep an unshared socket's port to itself. Call it before `bind`.
///
/// Sets SO_EXCLUSIVEADDRUSE, which refuses every other bind to the port while
/// this socket is open.
#[cfg(windows)]
pub(crate) fn claim_exclusive(socket: &socket2::Socket) -> io::Result<()> {
    use std::os::windows::io::AsSocket;
    use windows_sys::Win32::Networking::WinSock::{SOL_SOCKET, SO_EXCLUSIVEADDRUSE};

    crate::sys::winsock::set_int_option(socket.as_socket(), SOL_SOCKET, SO_EXCLUSIVEADDRUSE, 1)
}

/// Keep an unshared socket's port to itself. Call it before `bind`.
///
/// Nothing to set: Linux already refuses other binds, and macOS and the BSDs
/// have no option for the SO_REUSEADDR case the module docs describe.
#[cfg(not(windows))]
pub(crate) fn claim_exclusive(_socket: &socket2::Socket) -> io::Result<()> {
    Ok(())
}

/// Whether a bind failed because another socket holds the port. Tests whose
/// configuration must name a port before the transport binds it probe for a
/// free one, and another process can take it in between (#1032); this is the
/// failure they retry on. An explicit B/IP or B/IPv6 port sets SO_REUSEADDR,
/// and Windows refuses that bind with `WSAEACCES` rather than
/// `WSAEADDRINUSE` when the holder did not share the port. Only an OS error
/// counts, so a transport's own `AddrInUse` (a VMAC collision) does not.
///
/// On macOS a holder bound to a specific address does not fail the wildcard
/// bind at all; as the module docs say, nothing there reports that case.
#[cfg(test)]
pub(crate) fn lost_to_another_socket(err: &io::Error) -> bool {
    err.raw_os_error().is_some()
        && (err.kind() == io::ErrorKind::AddrInUse
            || (cfg!(windows) && err.kind() == io::ErrorKind::PermissionDenied))
}

/// How many times a test that can lose its port to another socket runs, each
/// time on a fresh port.
#[cfg(test)]
pub(crate) const ATTEMPTS: usize = 8;

/// Whether `err`, from run `attempt` of such a test, is a bind that another
/// socket beat to the port, so the test may go again on a fresh one (#1032,
/// #1068, #1070, #1095). Never on the last run: a node that really kept its
/// port fails every run, and the last one reports it.
#[cfg(test)]
pub(crate) fn lost_port(attempt: usize, err: &io::Error) -> bool {
    attempt < ATTEMPTS && lost_to_another_socket(err)
}

/// Starts a stopped `transport` again, for the restart tests. The restart
/// rebinds the port the first start was given, and another process can take
/// it while the transport is stopped (#1070). `None` means it did, and the
/// test goes again from its first start, which binds a fresh port; the last
/// run keeps the bind's error, so a transport that really kept its port fails
/// every run.
///
/// For a transport whose first start let the OS choose the port, the restart
/// binds it without SO_REUSEADDR, and every OS refuses such a bind while
/// another socket holds the port. The macOS gap in
/// [`lost_to_another_socket`] needs a SO_REUSEADDR bind, so it does not apply.
#[cfg(test)]
pub(crate) async fn restart(
    transport: &mut impl TransportPort,
    attempt: usize,
) -> Option<Result<mpsc::Receiver<ReceivedNpdu>, Error>> {
    match transport.start().await {
        Err(Error::Transport(ref err)) if lost_port(attempt, err) => None,
        started => Some(started),
    }
}

/// How many times [`connect_loopback`] connects before a local port collision
/// fails it.
#[cfg(test)]
const CONNECT_ATTEMPTS: usize = 5;

/// The real time [`connect_loopback`] waits after a local port collision.
#[cfg(test)]
const CONNECT_PAUSE: Duration = Duration::from_millis(50);

/// Opens a test's outbound loopback TCP connection; the SC hub and SC TLS
/// tests dial through here (#1629).
///
/// The OS picks the connection's source port. After many short-lived loopback
/// connections the port it picks can still be in use, and Windows then fails
/// the connect with `WSAEADDRINUSE`; on Linux and macOS a host out of
/// ephemeral ports fails it with `AddrNotAvailable`. Neither says anything
/// about the listener, so the connect runs again on a new socket, up to
/// [`CONNECT_ATTEMPTS`] times. Any other error fails at once.
#[cfg(test)]
pub(crate) async fn connect_loopback(address: SocketAddr) -> tokio::net::TcpStream {
    match retry_local_port(|| tokio::net::TcpStream::connect(address)).await {
        Ok(tcp) => tcp,
        Err(err) if local_port_collision(&err) => panic!(
            "loopback connect to {address} still collided on a local port after \
             {CONNECT_ATTEMPTS} attempts: {err} (#1629)"
        ),
        Err(err) => panic!("loopback connect to {address} failed: {err}"),
    }
}

/// Whether a connect failed at its own end: the source port the OS gave it
/// was taken, or there was none to give. A connect's errors all come from the
/// OS, so the kind is enough.
#[cfg(test)]
fn local_port_collision(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::AddrInUse | io::ErrorKind::AddrNotAvailable
    )
}

/// Runs `dial` again while it fails with a local port collision, up to
/// [`CONNECT_ATTEMPTS`] runs, and returns the last run's result.
///
/// The pause between runs is a thread sleep on Tokio's blocking pool. Many
/// callers run on a paused clock, some inside the SC hub tests' `poll_io`,
/// whose yields keep that clock from auto-advancing. A Tokio `sleep` would
/// never end there, and elsewhere its auto-advance would move the clock past
/// hub deadlines a test pins to the millisecond. Tokio holds a paused clock
/// still while a `spawn_blocking` task runs (see `tokio::time::pause`). Only a
/// current-thread runtime can pause its clock, so that is the only kind this
/// matters on; on a multi-thread runtime the pause is a plain 50 ms of real
/// time. On either, the runtime's threads stay free.
#[cfg(test)]
async fn retry_local_port<T, F>(mut dial: impl FnMut() -> F) -> io::Result<T>
where
    F: std::future::Future<Output = io::Result<T>>,
{
    let mut attempt = 1;
    loop {
        match dial().await {
            Err(err) if attempt < CONNECT_ATTEMPTS && local_port_collision(&err) => {
                attempt += 1;
                tokio::task::spawn_blocking(|| std::thread::sleep(CONNECT_PAUSE))
                    .await
                    .unwrap();
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

    /// A plain bind of 127.0.0.1:P beside the claimed 0.0.0.0:P fails on
    /// every OS; on Windows only because of the claim.
    #[test]
    fn a_claimed_wildcard_port_refuses_a_specific_address_bind() {
        let socket =
            socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None).unwrap();
        super::claim_exclusive(&socket).unwrap();
        socket
            .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)).into())
            .unwrap();
        let port = socket.local_addr().unwrap().as_socket().unwrap().port();
        assert!(UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).is_err());
        drop(socket);
        // UDP has no TIME_WAIT: the port is free as soon as its owner closes.
        UdpSocket::bind((Ipv4Addr::LOCALHOST, port)).unwrap();
    }

    /// A port held by another socket reruns every run but the last, so a
    /// leak still fails the test.
    #[test]
    fn a_lost_port_reruns_every_run_but_the_last() {
        let holder = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let err = UdpSocket::bind(holder.local_addr().unwrap()).unwrap_err();
        assert!((1..super::ATTEMPTS).all(|attempt| super::lost_port(attempt, &err)));
        assert!(!super::lost_port(super::ATTEMPTS, &err));
        // An AddrInUse the transport made up, not the OS, is never a lost port.
        let made_up = std::io::Error::from(std::io::ErrorKind::AddrInUse);
        assert!(!super::lost_port(1, &made_up));
    }
}

#[cfg(test)]
mod connect_loopback_tests {
    use super::{connect_loopback, local_port_collision, retry_local_port, CONNECT_ATTEMPTS};
    use std::io::{Error, ErrorKind};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// OS error codes, each with whether it is a local port collision:
    /// WSAEADDRINUSE (the code #1629 reported), WSAEADDRNOTAVAIL,
    /// WSAECONNREFUSED and WSAECONNRESET.
    #[cfg(windows)]
    const OS_CODES: [(i32, bool); 4] =
        [(10048, true), (10049, true), (10061, false), (10054, false)];

    /// OS error codes, each with whether it is a local port collision.
    #[cfg(unix)]
    const OS_CODES: [(i32, bool); 4] = [
        (libc::EADDRINUSE, true),
        (libc::EADDRNOTAVAIL, true),
        (libc::ECONNREFUSED, false),
        (libc::ECONNRESET, false),
    ];

    /// Runs a dial through the retry, its first runs failing with `errors` in
    /// turn and the rest connecting. Returns how many runs there were and the
    /// result.
    async fn dial(errors: &[ErrorKind]) -> (usize, std::io::Result<()>) {
        let runs = AtomicUsize::new(0);
        let result = retry_local_port(|| {
            let outcome = match errors.get(runs.fetch_add(1, Ordering::Relaxed)) {
                Some(&kind) => Err(Error::from(kind)),
                None => Ok(()),
            };
            async move { outcome }
        })
        .await;
        (runs.into_inner(), result)
    }

    #[test]
    fn only_the_address_in_use_family_is_a_local_port_collision() {
        for (code, collision) in OS_CODES {
            let err = Error::from_raw_os_error(code);
            assert_eq!(local_port_collision(&err), collision, "{err:?}");
        }
        for kind in [
            ErrorKind::ConnectionRefused,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::TimedOut,
            ErrorKind::PermissionDenied,
            ErrorKind::Other,
        ] {
            assert!(!local_port_collision(&kind.into()), "{kind:?}");
        }
    }

    /// A collision connects again and a real failure fails at once, on a
    /// paused clock that holds still across the pauses.
    #[tokio::test(start_paused = true)]
    async fn a_collision_connects_again_and_a_refusal_fails_at_once() {
        // A pending timer, standing in for a hub deadline: a pause that let
        // the clock auto-advance would fire it.
        let deadline = tokio::spawn(tokio::time::sleep(std::time::Duration::from_millis(1)));
        tokio::task::yield_now().await;
        let start = tokio::time::Instant::now();

        let (runs, result) = dial(&[ErrorKind::AddrInUse, ErrorKind::AddrNotAvailable]).await;
        assert_eq!(runs, 3);
        result.unwrap();

        let (runs, result) = dial(&[ErrorKind::ConnectionRefused]).await;
        assert_eq!(runs, 1);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::ConnectionRefused);

        let (runs, result) = dial(&[ErrorKind::AddrInUse, ErrorKind::ConnectionRefused]).await;
        assert_eq!(runs, 2);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::ConnectionRefused);

        let (runs, result) = dial(&[ErrorKind::AddrInUse; CONNECT_ATTEMPTS + 1]).await;
        assert_eq!(runs, CONNECT_ATTEMPTS);
        assert_eq!(result.unwrap_err().kind(), ErrorKind::AddrInUse);

        assert_eq!(
            tokio::time::Instant::now(),
            start,
            "a pause moved the clock"
        );
        assert!(!deadline.is_finished(), "a pause fired a pending timer");
    }

    #[tokio::test]
    async fn a_free_loopback_listener_is_reached() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (dialed, accepted) = tokio::join!(connect_loopback(address), listener.accept());
        assert_eq!(dialed.local_addr().unwrap(), accepted.unwrap().1);
    }
}
