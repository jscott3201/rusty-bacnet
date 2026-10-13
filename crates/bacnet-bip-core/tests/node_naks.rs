//! The NAK matrix: what a node answers to the requests only a BBMD serves
//! (Annex J.2.1, J.4.4, J.4.5, J.5.2).

mod node_support;

use core::net::Ipv4Addr;

use bacnet_bip_codec::{BdtEntries, BdtEntry, BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{Discard, Mode, Received};
use node_support::*;

const NPDU: &[u8] = &[0x01, 0x00];

/// The six requests only a BBMD serves, well formed, each with the NAK
/// code Annex J.2.1.1 gives it.
fn bbmd_requests() -> Vec<(Vec<u8>, u16)> {
    let rows = [BdtEntry::new(PEER, Ipv4Addr::BROADCAST)];
    vec![
        (
            wire(Bvll::WriteBroadcastDistributionTable(
                BdtEntries::from_entries(&rows),
            )),
            0x0010,
        ),
        (wire(Bvll::ReadBroadcastDistributionTable), 0x0020),
        (wire(Bvll::RegisterForeignDevice { ttl: 60 }), 0x0030),
        (wire(Bvll::ReadForeignDeviceTable), 0x0040),
        (wire(Bvll::DeleteForeignDeviceTableEntry(REMOTE)), 0x0050),
        (wire(Bvll::DistributeBroadcastToNetwork(NPDU)), 0x0060),
    ]
}

fn nak(to: core::net::SocketAddrV4, code: u16, malformed: bool) -> Received<'static> {
    Received::Nak {
        to,
        code: BvlcResultCode::from_raw(code),
        malformed,
    }
}

#[test]
fn each_bbmd_only_request_draws_its_own_nak() {
    for mode in [Mode::Normal, Mode::Foreign(foreign())] {
        let link = link(mode);
        for (frame, code) in bbmd_requests() {
            assert_eq!(
                receive(&link, &frame, PEER, UNICAST),
                nak(PEER, code, false)
            );
            // The one table the matrix is built on.
            let function = BvlcFunction::from_raw(frame[1]);
            assert_eq!(function.nak(), Some(BvlcResultCode::from_raw(code)));
        }
    }
}

#[test]
fn a_broadcast_bbmd_request_draws_nothing() {
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        let link = link(mode);
        for (frame, _) in bbmd_requests() {
            assert_eq!(
                receive(&link, &frame, PEER, TO_BROADCAST),
                Received::Discarded(Discard::DestinationMismatch)
            );
        }
    }
}

#[test]
fn write_bdt_is_refused_in_every_mode() {
    let (write, code) = bbmd_requests().swap_remove(0);
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        assert_eq!(
            receive(&link(mode), &write, PEER, UNICAST),
            nak(PEER, code, false)
        );
    }
    // A BBMD takes the other five itself.
    let link = link(Mode::Bbmd);
    for (frame, _) in bbmd_requests().into_iter().skip(1) {
        assert!(matches!(
            receive(&link, &frame, PEER, UNICAST),
            Received::Bbmd(_)
        ));
    }
}

#[test]
fn no_nak_to_an_unanswerable_source() {
    let unanswerable = Received::Discarded(Discard::UnanswerableSource);
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        let link = link(mode);
        for (frame, _) in bbmd_requests() {
            let claimed = mode == Mode::Bbmd && frame[1] != 0x01;
            let distribute = frame[1] == 0x09;
            for source in [addr(0, 0, 0, 0, PORT), addr(192, 168, 1, 20, 0)] {
                let received = receive(&link, &frame, source, UNICAST);
                if claimed {
                    assert!(matches!(received, Received::Bbmd(_)), "{received:?}");
                } else {
                    assert_eq!(received, unanswerable, "{source} {:#04X}", frame[1]);
                }
            }
            // From the node itself: a Distribute-Broadcast-To-Network is its
            // own echo, and nothing else is answered.
            let from_self = receive(&link, &frame, OWN, UNICAST);
            match (distribute, claimed) {
                (true, _) => assert_eq!(from_self, Received::Discarded(Discard::OwnEcho)),
                (false, true) => assert!(matches!(from_self, Received::Bbmd(_))),
                (false, false) => assert_eq!(from_self, unanswerable),
            }
        }
    }
}

#[test]
fn a_malformed_request_still_draws_its_nak() {
    // Five of the six with a payload their layout doesn't allow. The sixth,
    // Distribute-Broadcast-To-Network, takes any NPDU, even an empty one.
    let malformed: [(u8, &[u8], u16); 5] = [
        (0x01, &[0; 9], 0x0010),
        (0x02, &[0], 0x0020),
        (0x05, &[0], 0x0030),
        (0x06, &[0, 0], 0x0040),
        (0x08, &[0; 5], 0x0050),
    ];
    for mode in [Mode::Normal, Mode::Foreign(foreign()), Mode::Bbmd] {
        let link = link(mode);
        for (function, payload, code) in malformed {
            let frame = raw(function, payload);
            assert_eq!(
                receive(&link, &frame, PEER, UNICAST),
                nak(PEER, code, true),
                "{function:#04X}"
            );
            // From an unanswerable source, it is only counted malformed.
            assert_eq!(
                receive(&link, &frame, addr(0, 0, 0, 0, PORT), UNICAST),
                Received::Discarded(Discard::MalformedPayload)
            );
        }
    }
}
