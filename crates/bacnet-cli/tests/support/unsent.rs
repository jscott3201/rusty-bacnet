//! A loopback peer for a command that must be refused before it sends
//! anything (#2627, #2680).
//!
//! A bare "nothing arrived on this socket" check is open to the whole
//! loopback: nextest runs every test in its own process, in parallel, and the
//! OS hands a freed ephemeral port to the next socket that asks, so a late
//! datagram meant for another test's CLI can land on this one. The check here
//! names the sender instead. The command is run with `--port` set to
//! [`Unsent::source_port`], so everything it sends leaves from that port, and
//! only a datagram from there counts as the command's. Datagrams from any
//! other source are somebody else's and are set aside ([`super::sender`],
//! which the TUI's tests share). No outcome here, in the helper or in its own
//! tests, depends on how many datagrams in total reach the socket.
//!
//! The source port is a free one the helper picked and let go. A refused
//! command never binds it (the refusal precedes any transport), so nothing
//! holds it. A command that did send would bind it, or fail to bind it and
//! exit with another status, which the caller's exit-code check catches.
use std::cell::RefCell;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

use super::sender::queued_from;

pub struct Unsent {
    socket: UdpSocket,
    source: u16,
    /// Datagrams [`Self::wait_for`] has read and the check has not yet seen.
    held: RefCell<Vec<(SocketAddr, Vec<u8>)>>,
}

impl Unsent {
    pub fn new() -> Self {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        socket.set_nonblocking(true).unwrap();
        // Bound while the peer's own socket is held, so the two differ.
        let probe = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let source = probe.local_addr().unwrap().port();
        Self {
            socket,
            source,
            held: RefCell::default(),
        }
    }

    /// The port the command is pointed at, as its BBMD or its device.
    pub fn port(&self) -> u16 {
        self.socket.local_addr().unwrap().port()
    }

    /// The command's `--port`: the only source that counts as its own.
    pub fn source_port(&self) -> u16 {
        self.source
    }

    /// Wait until `count` datagrams from `sender`'s port have arrived,
    /// holding every datagram read on the way for the check. A command that
    /// has exited has already sent everything it will, so the guard tests
    /// need no wait; a test that sends from its own sockets and then reads at
    /// once can race the loopback, and uses this as its barrier. It waits for
    /// a named sender, so a foreign datagram never satisfies it.
    pub fn wait_for_from(&self, sender: u16, count: usize) {
        self.socket.set_nonblocking(false).unwrap();
        self.socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0u8; 1500];
        let mut arrived = self
            .held
            .borrow()
            .iter()
            .filter(|(from, _)| from.port() == sender)
            .count();
        let result = loop {
            if arrived >= count {
                break Ok(());
            }
            match self.socket.recv_from(&mut buffer) {
                Ok((length, from)) => {
                    arrived += usize::from(from.port() == sender);
                    self.held
                        .borrow_mut()
                        .push((from, buffer[..length].to_vec()));
                }
                Err(error) => break Err(error),
            }
        };
        // Restored before any failure, so the socket is never left blocking.
        self.socket.set_nonblocking(true).unwrap();
        result.expect("the datagram from the named sender");
    }

    /// Nothing from the command has arrived. The command has exited, so
    /// anything it sent over loopback is already queued here. Datagrams from
    /// other sources are set aside.
    pub fn assert_nothing_sent(&self, what: &str) {
        let own = SocketAddr::from((Ipv4Addr::LOCALHOST, self.source));
        let held = self.held.take().into_iter().find(|(from, _)| *from == own);
        let sent = match held {
            Some((_, bytes)) => Some(bytes),
            None => queued_from(own, |buffer| self.socket.recv_from(buffer))
                .unwrap_or_else(|error| panic!("{what}: reading the peer socket: {error}")),
        };
        if let Some(bytes) = sent {
            panic!(
                "{what}: the command sent {} bytes from its own port: {bytes:02x?}",
                bytes.len()
            );
        }
    }
}
