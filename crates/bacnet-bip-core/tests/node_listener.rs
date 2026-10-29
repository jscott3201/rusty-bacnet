//! `Link::listener_keeps` gives the verdict `Link::receive` gives a
//! broadcast listener's datagram, so a host that reports what a listener
//! keeps before the node acts on it, as a frame tap does, can't drift from
//! the node (#1856).

mod node_support;

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{Addressed, Discard, Link, Mode, Path, Received};
use node_support::*;

const NPDU: &[u8] = &[0x01, 0x00, 0x10, 0x08];

/// Whether `receive` refused `datagram` by a listener rule.
fn refused_by_listener(link: &Link, datagram: &bacnet_bip_core::node::Datagram<'_>) -> bool {
    matches!(
        receive_datagram(link, datagram),
        Received::Discarded(Discard::ListenerMismatch { .. })
    )
}

/// Every pairing of mode, path, destination and frame: the predicate and
/// the receive agree on each, and each verdict the matrix depends on shows
/// up at least once.
#[test]
fn the_predicate_and_receive_agree() {
    let both = Addressed {
        node: true,
        broadcast: true,
    };
    let neither = Addressed {
        node: false,
        broadcast: false,
    };
    let frames = [
        wire(Bvll::OriginalBroadcastNpdu(NPDU)),
        wire(Bvll::OriginalUnicastNpdu(NPDU)),
        wire(Bvll::ForwardedNpdu {
            origin: REMOTE,
            npdu: NPDU,
        }),
        wire(Bvll::DistributeBroadcastToNetwork(NPDU)),
        result(BvlcResultCode::SUCCESSFUL_COMPLETION),
        raw(
            BvlcFunction::READ_BROADCAST_DISTRIBUTION_TABLE.to_raw(),
            &[],
        ),
        // Doesn't decode: the node's to count.
        vec![0x81, 0x0B, 0x00],
    ];
    let links = [
        link(Mode::Normal),
        foreign_link(foreign()),
        link(Mode::Bbmd),
    ];
    let paths = [
        Path::Primary,
        Path::Listener {
            other_interface: false,
        },
        Path::Listener {
            other_interface: true,
        },
    ];
    let (mut kept, mut refused) = (0, 0);
    for (l, link) in links.iter().enumerate() {
        for (p, path) in paths.into_iter().enumerate() {
            for to in [UNICAST, TO_BROADCAST, both, neither] {
                for (f, frame) in frames.iter().enumerate() {
                    // From the BBMD, so a foreign device trusts its
                    // Forwarded-NPDU.
                    let datagram = with(datagram(frame, BBMD, to), |d| d.path = path);
                    let keeps = link.listener_keeps(&datagram);
                    assert_eq!(
                        keeps,
                        !refused_by_listener(link, &datagram),
                        "link {l}, path {p}, to {to:?}, frame {f}"
                    );
                    if matches!(path, Path::Listener { .. }) {
                        if keeps { kept += 1 } else { refused += 1 }
                    }
                }
            }
        }
    }
    assert!(kept > 0 && refused > 0, "{kept} kept, {refused} refused");
}

/// A foreign device whose broadcast address is its own, as in a loopback
/// test, takes its BBMD's Forwarded-NPDU as the unicast it was (#1851), so
/// a listener refuses it; a NORMAL node's listener keeps it as a broadcast.
/// This is where a host's own copy of the rule had drifted from the node's.
#[test]
fn a_foreign_listener_refuses_a_forwarded_npdu_to_its_own_address() {
    let both = Addressed {
        node: true,
        broadcast: true,
    };
    let frame = wire(Bvll::ForwardedNpdu {
        origin: REMOTE,
        npdu: NPDU,
    });
    let on_listener = with(datagram(&frame, BBMD, both), |d| {
        d.path = Path::Listener {
            other_interface: false,
        }
    });
    let foreign = foreign_link(foreign());
    assert!(!foreign.listener_keeps(&on_listener));
    assert!(refused_by_listener(&foreign, &on_listener));
    let normal = link(Mode::Normal);
    assert!(normal.listener_keeps(&on_listener));
    assert!(!refused_by_listener(&normal, &on_listener));
}
