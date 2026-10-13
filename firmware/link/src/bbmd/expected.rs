//! What the BBMD scenario reports in a good run. `tests/bbmd.rs` compares
//! the native run's summary with it, and the emulation build of the `bbmd`
//! image the emulated CPU's, so the two can't drift apart.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntry, BvlcResultCode, FdtEntry};
use bacnet_bip_core::bbmd::{ForwardRefusal, Registered, Via, TWO_HOP_MASK};

use super::{
    Expected, ExpectedFan, Summary, FOREIGN, LOCAL, MAX_TARGETS, OWN, PEERS, REMOTE, TTL,
    TWO_HOP_PEER,
};
use crate::tally::Sent;

/// The hash over every datagram the BBMD sends and where it goes. It was
/// worked out apart from the codec, from the 12 datagrams written out by
/// hand from the Annex J.2 layouts (204 octets): two BVLC-Results, eight
/// Forwarded-NPDUs and the two table ACKs.
const SENT_FNV: u32 = 0xC89E_516A;

/// A destination as an [`ExpectedFan`] lists it.
type Destination = (SocketAddrV4, Via);

/// The subnet's broadcast address.
const LOCAL_BROADCAST: Destination = (
    SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), 0xBAC0),
    Via::LocalBroadcast,
);
/// The one-hop peer's subnet, at its directed broadcast address.
const ONE_HOP: Destination = (
    SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 255), 0xBAC0),
    Via::Peer,
);
/// The two-hop peer, at its own address.
const TWO_HOP: Destination = (TWO_HOP_PEER, Via::Peer);
/// The foreign device.
const DEVICE: Destination = (FOREIGN, Via::ForeignDevice);

/// A fan of these destinations, the rest unused.
const fn fan<const N: usize>(destinations: [Destination; N]) -> ExpectedFan {
    let mut fan = [None; MAX_TARGETS];
    let mut index = 0;
    while index < N {
        fan[index] = Some(destinations[index]);
        index += 1;
    }
    fan
}

/// The summary [`run`](super::run) returns in a good run.
pub const EXPECTED: Expected = Expected {
    summary: Summary {
        registration: Some(Ok(Registered::New)),
        // Read a second after it registered: its time-to-live plus the 30 s
        // grace period, less that second.
        fdt: [Some(FdtEntry::new(FOREIGN, TTL, 89)), None, None, None],
        // In `Expected::original`, `forwarded` and `distributed`.
        original: [None; MAX_TARGETS],
        forwarded: [None; MAX_TARGETS],
        not_from_peer: Some(ForwardRefusal::NotFromPeer),
        distributed: [None; MAX_TARGETS],
        not_registered: Some(BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK),
        // The configured peers, then the BBMD's own row, which the table
        // appended with the two-hop mask.
        bdt: [
            Some(PEERS[0]),
            Some(PEERS[1]),
            Some(BdtEntry::new(OWN, TWO_HOP_MASK)),
            None,
        ],
        // The Original-Broadcast from its sender, the Forwarded-NPDU from
        // where it started, and the Distribute-Broadcast-To-Network from the
        // foreign device.
        delivered: [Some(LOCAL), Some(REMOTE), Some(FOREIGN), None],
        // The two results of 6 octets, eight Forwarded-NPDUs of 18, the
        // Read-FDT-Ack of one row (14) and the Read-BDT-Ack of three (34).
        sent: Sent {
            count: 12,
            octets: 204,
            fnv: SENT_FNV,
        },
        errors: 0,
    },
    // To both peers' subnets and the foreign device; the BBMD's own subnet
    // has it already.
    original: fan([ONE_HOP, TWO_HOP, DEVICE]),
    // The two-hop peer sent it to this BBMD alone, so it goes on to the
    // subnet's broadcast address, and to the foreign device, but not back
    // to any peer.
    forwarded: fan([LOCAL_BROADCAST, DEVICE]),
    // To the subnet and both peers' subnets; the foreign device that sent
    // it is left out.
    distributed: fan([LOCAL_BROADCAST, ONE_HOP, TWO_HOP]),
};
