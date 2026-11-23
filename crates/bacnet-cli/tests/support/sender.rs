//! Judging a loopback datagram by its sender (#2680, #2718).
//!
//! nextest runs every test in its own process, in parallel, and the OS hands
//! a freed ephemeral port to the next socket that asks, so a late datagram
//! meant for another test can land on a socket a test has just bound. A
//! check on such a socket names the sender it is about and drops datagrams
//! from anywhere else; it never counts what arrived in total. The CLI's
//! integration tests read through here for a child process sending from a
//! port the test picked (`Unsent`), and the TUI's unit tests, which include
//! this file by path, for a session running in the test's own process.

use std::io;
use std::net::SocketAddr;

/// Reads what is queued on a non-blocking socket, through `try_recv`, until
/// a datagram from `sender` turns up or nothing more is queued. Datagrams
/// from any other source are dropped. Gives back the sender's datagram, or
/// `None` once the queue has run dry without one.
pub fn queued_from(
    sender: SocketAddr,
    mut try_recv: impl FnMut(&mut [u8]) -> io::Result<(usize, SocketAddr)>,
) -> io::Result<Option<Vec<u8>>> {
    let mut buffer = [0u8; 1500];
    loop {
        match try_recv(&mut buffer) {
            Ok((length, from)) if from == sender => return Ok(Some(buffer[..length].to_vec())),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        }
    }
}

/// The next datagram from `sender` on `socket`, dropping datagrams from any
/// other source on the way. It waits without blocking the thread, so the
/// code under test can run on the same runtime; the caller bounds the wait.
pub async fn next_from(socket: &tokio::net::UdpSocket, sender: SocketAddr) -> Vec<u8> {
    loop {
        socket.readable().await.expect("the socket stays readable");
        let queued = queued_from(sender, |buffer| socket.try_recv_from(buffer));
        if let Some(datagram) = queued.expect("reading the socket") {
            return datagram;
        }
    }
}
