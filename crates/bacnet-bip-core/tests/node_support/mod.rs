//! Helpers the node engine's tests share.

#![allow(dead_code)]

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll, BvllFrame};
use bacnet_bip_core::node::{
    Action, Addressed, Datagram, Foreign, Lease, Link, Mode, Node, Outgoing, Permissive, Policy,
    Received,
};
use bacnet_bip_core::{Duration, Instant, LocalSubnet};

/// The default B/IP port.
pub const PORT: u16 = 0xBAC0;

/// `a.b.c.d:port`.
pub const fn addr(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
}

/// The node under test.
pub const OWN: SocketAddrV4 = addr(192, 168, 1, 10, PORT);
/// Its subnet's broadcast address.
pub const BROADCAST: SocketAddrV4 = addr(192, 168, 1, 255, PORT);
/// Its subnet.
pub const SUBNET: LocalSubnet = LocalSubnet::new(BROADCAST);
/// A peer on the subnet.
pub const PEER: SocketAddrV4 = addr(192, 168, 1, 20, PORT);
/// A foreign device's BBMD, on another subnet.
pub const BBMD: SocketAddrV4 = addr(10, 0, 0, 1, PORT);
/// A device elsewhere, the origin of forwarded broadcasts.
pub const REMOTE: SocketAddrV4 = addr(172, 16, 0, 7, PORT);

/// Sent to the node's own address.
pub const UNICAST: Addressed = Addressed {
    node: true,
    broadcast: false,
};
/// Sent to the broadcast address.
pub const TO_BROADCAST: Addressed = Addressed {
    node: false,
    broadcast: true,
};

/// A link for [`OWN`] in `mode`, with the default budget.
pub fn link(mode: Mode) -> Link {
    Link::new(OWN, SUBNET, mode).expect("valid link")
}

/// Foreign-device registration with [`BBMD`] for 60 s, as `Foreign::new`
/// sets it up: renewal 30 s, response timeout 3 s, backoff.
pub fn foreign() -> Foreign {
    Foreign::new(BBMD, 60)
}

/// A link for a foreign device registered as `foreign` says.
pub fn foreign_link(foreign: Foreign) -> Link {
    link(Mode::Foreign(foreign))
}

/// `message` on the wire.
pub fn wire(message: Bvll<'_>) -> Vec<u8> {
    let mut out = vec![0; message.encoded_len().expect("encodable")];
    message.encode_into(&mut out).expect("fits");
    out
}

/// A frame with `function` and a raw `payload`, its Length field right.
pub fn raw(function: u8, payload: &[u8]) -> Vec<u8> {
    let length = u16::try_from(4 + payload.len()).expect("short");
    let mut out = vec![0x81, function];
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(payload);
    BvllFrame::decode(&out).expect("header checks out");
    out
}

/// A datagram on the primary socket, sent as `to` says.
pub fn datagram(payload: &[u8], source: SocketAddrV4, to: Addressed) -> Datagram<'_> {
    Datagram::new(payload, source, Some(to))
}

/// Where an [`Outgoing`] goes and what it carries. `Outgoing` is
/// `#[non_exhaustive]`, so a test outside the crate compares this pair.
pub fn sent(outgoing: Outgoing<'_>) -> (SocketAddrV4, Bvll<'_>) {
    (outgoing.to, outgoing.message)
}

/// A [`Lease`]'s start and end, for the same reason as [`sent`].
pub fn span(lease: Lease) -> (Instant, Instant) {
    (lease.since, lease.until)
}

/// `value` with `edit` applied: how a test changes a field of a
/// `#[non_exhaustive]` struct, such as [`Foreign`] or [`Link`], which it
/// can't build with a struct expression.
pub fn with<T>(mut value: T, edit: impl FnOnce(&mut T)) -> T {
    edit(&mut value);
    value
}

/// What `link` makes of `payload` from `source`, sent as `to`, on the
/// primary socket with a permissive policy.
pub fn receive<'d>(
    link: &Link,
    payload: &'d [u8],
    source: SocketAddrV4,
    to: Addressed,
) -> Received<'d> {
    link.receive(
        &datagram(payload, source, to),
        Instant::ZERO,
        &mut Permissive,
    )
}

/// What `link` makes of `datagram` with a permissive policy.
pub fn receive_datagram<'d>(link: &Link, datagram: &Datagram<'d>) -> Received<'d> {
    link.receive(datagram, Instant::ZERO, &mut Permissive)
}

/// `millis` milliseconds after the epoch.
pub const fn at(millis: u64) -> Instant {
    Instant::from_millis(millis)
}

/// `millis` milliseconds.
pub const fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// A BVLC-Result with `code`.
pub fn result(code: BvlcResultCode) -> Vec<u8> {
    wire(Bvll::BvlcResult(code))
}

/// Success.
pub const SUCCESS: BvlcResultCode = BvlcResultCode::SUCCESSFUL_COMPLETION;

/// Every action the node has queued, in order.
pub fn drain<const X: usize>(node: &mut Node<X>) -> Vec<Action> {
    let mut actions = Vec::new();
    loop {
        match node.poll_action() {
            Action::Idle => return actions,
            action => actions.push(action),
        }
    }
}

/// A policy that records what it is asked and answers as set.
#[derive(Debug, Default)]
pub struct Recording {
    /// Admit management requests.
    pub admit: bool,
    /// Accept table ACKs.
    pub accept: bool,
    /// The functions asked about, in order.
    pub asked: Vec<BvlcFunction>,
}

impl Recording {
    pub fn new(admit: bool, accept: bool) -> Self {
        Self {
            admit,
            accept,
            asked: Vec::new(),
        }
    }
}

impl Policy for Recording {
    fn admit_management(&mut self, _: SocketAddrV4, function: BvlcFunction, _: Instant) -> bool {
        self.asked.push(function);
        self.admit
    }

    fn accept_table(&mut self, ack: &Bvll<'_>) -> bool {
        self.asked.push(ack.function());
        self.accept
    }
}
