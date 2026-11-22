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
//! other source are somebody else's and are counted, not failed on.
//!
//! The source port is a free one the helper picked and let go. A refused
//! command never binds it (the refusal precedes any transport), so nothing
//! holds it. A command that did send would bind it, or fail to bind it and
//! exit with another status, which the caller's exit-code check catches.
use std::cell::RefCell;
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::time::Duration;

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

    /// Wait until `count` datagrams have arrived and hold them for the check.
    /// A command that has exited has already sent everything it will, so the
    /// guard tests need no wait; a test that sends from its own sockets and
    /// then reads at once can race the loopback, and uses this as its barrier.
    pub fn wait_for(&self, count: usize) {
        self.socket.set_nonblocking(false).unwrap();
        self.socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0u8; 1500];
        for _ in 0..count {
            let (length, from) = self.socket.recv_from(&mut buffer).expect("a datagram");
            self.held
                .borrow_mut()
                .push((from, buffer[..length].to_vec()));
        }
        self.socket.set_nonblocking(true).unwrap();
    }

    /// Nothing from the command has arrived. The command has exited, so
    /// anything it sent over loopback is already queued here. Returns how many
    /// datagrams from other sources were set aside.
    pub fn assert_nothing_sent(&self, what: &str) -> usize {
        let own = SocketAddr::from((Ipv4Addr::LOCALHOST, self.source));
        let mut seen: Vec<_> = self.held.take();
        let mut buffer = [0u8; 1500];
        loop {
            match self.socket.recv_from(&mut buffer) {
                Ok((length, from)) => seen.push((from, buffer[..length].to_vec())),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("{what}: reading the peer socket: {error}"),
            }
        }
        if let Some((_, bytes)) = seen.iter().find(|(from, _)| *from == own) {
            panic!(
                "{what}: the command sent {} bytes from its own port: {bytes:02x?}",
                bytes.len()
            );
        }
        seen.len()
    }
}
