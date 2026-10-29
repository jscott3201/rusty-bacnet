//! What the BBMD scenario reports in a good run. `tests/bbmd.rs` compares
//! the native run's summary with it, and the emulation build of the `bbmd`
//! image the emulated CPU's, so the two can't drift apart.

use core::net::{Ipv4Addr, SocketAddrV4};

use bacnet_bip_codec::{BdtEntry, BvlcResultCode, FdtEntry};
use bacnet_bip_core::bbmd::{DeleteRefusal, ForwardRefusal, Registered, TWO_HOP_MASK};

use super::{
    FOREIGN, Fan, LOCAL, MAX_TARGETS, OWN, PEERS, REMOTE, STRANGER, Summary, TTL, TWO_HOP_PEER,
};
use crate::tally::Sent;

/// The hash over every datagram the BBMD sends and where it goes. It was
/// worked out apart from the codec, from the 14 datagrams written out by
/// hand from the Annex J.2 layouts (216 octets): four BVLC-Results, eight
/// Forwarded-NPDUs and the two table ACKs. The first 12, all but the two
/// Delete-FDT-Entry answers, hash to 0xC89E_516A, as they did before the
/// port answered management requests itself (#1766).
const SENT_FNV: u32 = 0xA309_6E02;

/// The subnet's broadcast address.
const LOCAL_BROADCAST: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 255), 0xBAC0);
/// The one-hop peer's subnet, at its directed broadcast address.
const ONE_HOP: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(10, 0, 2, 255), 0xBAC0);

/// A fan of these destinations, the rest unused.
const fn fan<const N: usize>(destinations: [SocketAddrV4; N]) -> Fan {
    let mut fan = [None; MAX_TARGETS];
    let mut index = 0;
    while index < N {
        fan[index] = Some(destinations[index]);
        index += 1;
    }
    fan
}

const SUCCESS: BvlcResultCode = BvlcResultCode::SUCCESSFUL_COMPLETION;

/// The summary [`run`](super::run) returns in a good run.
pub const EXPECTED: Summary = Summary {
    registration: Some(Ok(Registered::New)),
    // Read a second after it registered: its time-to-live plus the 30 s
    // grace period, less that second.
    fdt: [Some(FdtEntry::new(FOREIGN, TTL, 89)), None, None, None],
    // To both peers' subnets and the foreign device; the BBMD's own subnet
    // has it already.
    original: fan([ONE_HOP, TWO_HOP_PEER, FOREIGN]),
    // The two-hop peer sent it to this BBMD alone, so it goes on to the
    // subnet's broadcast address, and to the foreign device, but not back
    // to any peer.
    forwarded: fan([LOCAL_BROADCAST, FOREIGN]),
    not_from_peer: Some(ForwardRefusal::NotFromPeer),
    // To the subnet and both peers' subnets; the foreign device that sent
    // it is left out.
    distributed: fan([LOCAL_BROADCAST, ONE_HOP, TWO_HOP_PEER]),
    // The configured peers, then the BBMD's own row, which the table
    // appended with the two-hop mask.
    bdt: [
        Some(PEERS[0]),
        Some(PEERS[1]),
        Some(BdtEntry::new(OWN, TWO_HOP_MASK)),
        None,
    ],
    // The registration is deleted; the second request finds none.
    deleted: [Some(Ok(())), Some(Err(DeleteRefusal::NotRegistered))],
    // The registration's success, the unregistered sender's
    // Distribute-Broadcast-To-Network NAK, then the two Delete-FDT-Entry
    // answers: success, and the Delete NAK.
    results: [
        Some((FOREIGN, SUCCESS)),
        Some((
            STRANGER,
            BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK,
        )),
        Some((LOCAL, SUCCESS)),
        Some((LOCAL, BvlcResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK)),
    ],
    // The Original-Broadcast from its sender, the Forwarded-NPDU from
    // where it started, and the Distribute-Broadcast-To-Network from the
    // foreign device.
    delivered: [Some(LOCAL), Some(REMOTE), Some(FOREIGN), None],
    // The four results of 6 octets, eight Forwarded-NPDUs of 18, the
    // Read-FDT-Ack of one row (14) and the Read-BDT-Ack of three (34).
    sent: Sent {
        count: 14,
        octets: 216,
        fnv: SENT_FNV,
    },
    refused: 0,
    errors: 0,
};
