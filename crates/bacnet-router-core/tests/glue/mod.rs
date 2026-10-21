//! Helpers the link glue suites share: a recording datagram sink and the
//! B/IP addresses for the `bip` glue, and an MS/TP master core over boxed
//! storage for the `mstp` glue.

#![allow(dead_code, unused_imports)]

use core::borrow::BorrowMut;

use bacnet_router_core::router::{Action, Frame, Send, SendRefusal};
use bacnet_router_core::{PortId, Router, Storage};

#[cfg(feature = "bip")]
pub use self::bip::*;
#[cfg(feature = "mstp")]
pub use self::mstp::*;

#[cfg(feature = "mstp")]
mod mstp;

/// The B/IP port in the B/IP suites: port 0, on network 1. The router's
/// other port there is port 1 on network 2, with 1-octet MACs.
pub const BIP_PORT: PortId = PortId(0);

/// A global-broadcast Who-Is.
pub const GLOBAL_WHO_IS: [u8; 8] = [0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];

/// `value` with `edit` applied: how a suite changes a field of a
/// `#[non_exhaustive]` struct from a core crate, such as a B/IP `Link` or
/// `Datagram`, which it can't build with a struct expression.
pub fn with<T>(mut value: T, edit: impl FnOnce(&mut T)) -> T {
    edit(&mut value);
    value
}

/// What port `port` received from `source`.
pub fn frame<'f>(port: u8, source: &'f [u8], link_group: bool, npdu: &'f [u8]) -> Frame<'f> {
    Frame {
        port: PortId(port),
        source,
        link_group,
        npdu,
    }
}

/// Poll `router` until idle, handing each send to `each` and telling the
/// router of each refusal, as a firmware loop does. Returns each send's
/// port and result.
pub fn pump<S, const P: usize, const R: usize, const H: usize, const D: usize, const N: usize>(
    router: &mut Router<S, P, R, H, D, N>,
    mut each: impl FnMut(&Send<'_>) -> Result<(), SendRefusal>,
) -> Vec<(u8, Result<(), SendRefusal>)>
where
    S: BorrowMut<Storage<P, R, H, D, N>>,
{
    let mut results = Vec::new();
    loop {
        let (port, result) = match router.poll_action() {
            Action::Idle => return results,
            Action::Send(send) => (send.port.0, each(&send)),
        };
        if let Err(why) = result {
            router.refused(why);
        }
        results.push((port, result));
        assert!(results.len() < 1_000, "the router never went idle");
    }
}

#[cfg(feature = "bip")]
mod bip {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use bacnet_bip_core::codec::{address_to_octets, BvlcFunction, Bvll, BvllFrame};
    use bacnet_bip_core::node::{Addressed, Datagram, Foreign, Link, Mode, Node, SendRefusal};
    use bacnet_bip_core::port::Datagrams;
    use bacnet_bip_core::LocalSubnet;
    use bacnet_router_core::Instant;

    /// The B/IP port of the network's devices.
    pub const PORT: u16 = 0xBAC0;

    pub const fn sock(a: u8, b: u8, c: u8, d: u8, port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::new(a, b, c, d), port)
    }

    /// The router's own B/IP address.
    pub const OWN: SocketAddrV4 = sock(192, 0, 2, 10, PORT);
    /// Its subnet, by broadcast address, with a /24 mask.
    pub const SUBNET: LocalSubnet = {
        let mut subnet = LocalSubnet::new(sock(192, 0, 2, 255, PORT));
        subnet.mask = Some(Ipv4Addr::new(255, 255, 255, 0));
        subnet
    };
    /// A device on the subnet.
    pub const PEER: SocketAddrV4 = sock(192, 0, 2, 20, PORT);
    /// A BBMD on another subnet: the foreign device's, or a BDT peer's.
    pub const BBMD: SocketAddrV4 = sock(198, 51, 100, 1, PORT);
    /// A device behind it, the origin of a forwarded broadcast.
    pub const ORIGIN: SocketAddrV4 = sock(198, 51, 100, 10, 0xBAC1);
    /// A foreign device registered with the router's BBMD.
    pub const DEVICE: SocketAddrV4 = sock(203, 0, 113, 7, PORT);

    /// The six octets of `address`.
    pub fn mac(address: SocketAddrV4) -> Vec<u8> {
        address_to_octets(address).to_vec()
    }

    /// A link at [`OWN`] in `mode`, with the default budget.
    pub fn link(mode: Mode) -> Link {
        Link::new(OWN, SUBNET, mode).unwrap()
    }

    /// A node on a link at [`OWN`] in `mode`, built at `now`.
    pub fn node(mode: Mode, now: Instant) -> Node {
        Node::new(link(mode), now).unwrap()
    }

    /// A foreign device's registration with [`BBMD`] for 60 s, holding
    /// `hold` broadcasts.
    pub fn foreign(hold: u16) -> Mode {
        Mode::Foreign(super::with(Foreign::new(BBMD, 60), |foreign| {
            foreign.hold = hold;
        }))
    }

    /// Sent to the node's own address.
    pub const UNICAST: Addressed = Addressed {
        node: true,
        broadcast: false,
    };
    /// Sent to the subnet's broadcast address.
    pub const TO_BROADCAST: Addressed = Addressed {
        node: false,
        broadcast: true,
    };

    /// `message`, encoded.
    pub fn encode(message: &Bvll<'_>) -> Vec<u8> {
        let mut out = vec![0; message.encoded_len().unwrap()];
        message.encode_into(&mut out).unwrap();
        out
    }

    /// `payload` from `source`, sent as `to` says, read by the primary
    /// socket.
    pub fn datagram(payload: &[u8], source: SocketAddrV4, to: Addressed) -> Datagram<'_> {
        Datagram::new(payload, source, Some(to))
    }

    /// A [`Datagrams`] sink that records what it sends, and refuses with
    /// `refuse` when set.
    #[derive(Debug, Default)]
    pub struct Wire {
        /// Each datagram sent: where, and its octets.
        pub sent: Vec<(SocketAddrV4, Vec<u8>)>,
        /// Every call, refused ones included.
        pub calls: usize,
        /// Refuse every send with this: `SocketFull` or `SocketError`.
        pub refuse: Option<SendRefusal>,
    }

    impl Datagrams for Wire {
        fn send(&mut self, to: SocketAddrV4, message: &Bvll<'_>) -> Result<(), SendRefusal> {
            self.calls += 1;
            if let Some(why) = self.refuse {
                return Err(why);
            }
            self.sent.push((to, encode(message)));
            Ok(())
        }
    }

    /// A datagram the sink sent, decoded.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Out {
        /// Where it went.
        pub to: SocketAddrV4,
        /// Its function.
        pub function: BvlcFunction,
        /// A Forwarded-NPDU's origin.
        pub origin: Option<SocketAddrV4>,
        /// Its NPDU, or for any other message its payload's octets.
        pub npdu: Vec<u8>,
    }

    impl Out {
        /// A message of `function` to `to`, carrying `npdu`.
        pub fn new(to: SocketAddrV4, function: BvlcFunction, npdu: &[u8]) -> Self {
            Self {
                to,
                function,
                origin: None,
                npdu: npdu.to_vec(),
            }
        }

        /// A Forwarded-NPDU from `origin` to `to`, carrying `npdu`.
        pub fn forwarded(to: SocketAddrV4, origin: SocketAddrV4, npdu: &[u8]) -> Self {
            Self {
                origin: Some(origin),
                ..Self::new(to, BvlcFunction::FORWARDED_NPDU, npdu)
            }
        }
    }

    impl Wire {
        /// Each datagram sent, decoded, and forget them.
        pub fn take(&mut self) -> Vec<Out> {
            self.sent
                .drain(..)
                .map(|(to, octets)| {
                    let frame = BvllFrame::decode(&octets).unwrap();
                    let (origin, npdu) = match frame.message().unwrap() {
                        Bvll::OriginalUnicastNpdu(npdu)
                        | Bvll::OriginalBroadcastNpdu(npdu)
                        | Bvll::DistributeBroadcastToNetwork(npdu) => (None, npdu.to_vec()),
                        Bvll::ForwardedNpdu { origin, npdu } => (Some(origin), npdu.to_vec()),
                        _ => (None, octets[4..].to_vec()),
                    };
                    Out {
                        to,
                        function: frame.function,
                        origin,
                        npdu,
                    }
                })
                .collect()
        }
    }
}
