//! Reject addressing and relay at the send-queue level (#1158), and the
//! rejects that stop at the router's own network-control consumer (#1175).
//! The loopback tests in `crate::reject_route_tests` cover the same paths on
//! the wire, port, destination MAC (#1243) and data attributes (#1289)
//! included.

use std::sync::Arc;

use bacnet_encoding::npdu::{Npdu, NpduAddress};
use bacnet_npdu::NetworkMessageType;
use bacnet_transport::port::DataAttribute;
use bacnet_types::MacAddr;
use bytes::Bytes;

use super::control_policy::ControlGate;
use super::envelope_harness::{attributes, Harness, IngressContext, TableSeed, PORT_MACS};
use super::SendRequest;
use crate::router_table::Reachability;

/// Reason 1 for network 5000: the payload every reject below carries.
const NO_ROUTE_TO_5000: [u8; 3] = [0x01, 0x13, 0x88];

fn unicast(request: SendRequest) -> (Vec<u8>, MacAddr, Vec<DataAttribute>) {
    match request {
        SendRequest::Unicast {
            npdu,
            mac,
            data_attributes,
            ..
        } => (npdu.to_vec(), mac, data_attributes),
        SendRequest::Broadcast { .. } => panic!("expected a unicast"),
    }
}

/// The link sender of every NPDU refused below.
const SENDER: [u8; 4] = [0x0A, 0x00, 0x01, 0x01];

fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

#[tokio::test]
async fn no_reject_goes_out_when_the_originator_is_the_router() {
    // An APDU from [`SENDER`] on port 0 (network 1000) for 5000, which the
    // table holds as unreachable, so it is refused with reason 1 and no
    // search. Its SNET/SADR is the router's own MAC on the arrival network,
    // or on the other one: no reject goes out on either port (#1219).
    for origin in [
        address(1000, &[PORT_MACS[0]]),
        address(2000, &[PORT_MACS[1]]),
    ] {
        let mut h = Harness::two_port();
        h.learned(5000, 1, &[9]);
        h.table().mark_unreachable(5000);
        let npdu = Npdu {
            destination: Some(address(5000, &[5])),
            source: Some(origin.clone()),
            hop_count: 255,
            payload: Bytes::from_static(&[0x10, 0x08]),
            ..Npdu::default()
        };
        h.dispatch(h.ctx(0, &SENDER, npdu)).await;
        h.assert_quiet();
    }
}

/// Direct 1000/0 and 2000/1, and 3000 learned behind [9] on port 0.
fn relay_table() -> TableSeed {
    let mut table = TableSeed::new();
    table.add_direct(1000, 0);
    table.add_direct(2000, 1);
    table.add_learned(3000, 0, MacAddr::from_slice(&[9]));
    table
}

fn relay_harness() -> Harness {
    Harness::with_table(relay_table())
}

fn received_reject(
    destination: Option<(u16, &[u8])>,
    source: Option<(u16, &[u8])>,
    hop: u8,
) -> Npdu {
    let address = |(network, mac): (u16, &[u8])| NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    };
    Npdu {
        message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK),
        destination: destination.map(address),
        source: source.map(address),
        hop_count: hop,
        payload: Bytes::copy_from_slice(&NO_ROUTE_TO_5000),
        ..Npdu::default()
    }
}

/// A received reject's DNET/DADR, and the frame and link MAC it is relayed
/// with.
struct Relayed {
    dnet: u16,
    dadr: &'static [u8],
    link_mac: &'static [u8],
    npdu: &'static [u8],
}

#[tokio::test]
async fn relay_routes_a_received_reject_by_its_dnet() {
    let cases = [
        // Directly connected: DNET/DADR come off, SNET 2000 / SADR [2] go on,
        // and the reject goes straight to the DADR.
        Relayed {
            dnet: 1000,
            dadr: &[0x0A],
            link_mac: &[0x0A],
            npdu: &[0x01, 0x88, 0x07, 0xD0, 0x01, 0x02, 0x03, 0x01, 0x13, 0x88],
        },
        // Behind another router: DNET/DADR stay, one hop is spent, and the
        // reject goes to the next hop.
        Relayed {
            dnet: 3000,
            dadr: &[0x30],
            link_mac: &[9],
            npdu: &[
                0x01, 0xA8, 0x0B, 0xB8, 0x01, 0x30, 0x07, 0xD0, 0x01, 0x02, 0xFE, 0x03, 0x01, 0x13,
                0x88,
            ],
        },
    ];
    for case in cases {
        let dnet = case.dnet;
        let mut h = relay_harness();
        let mut ctx = h.ctx(1, &[2], received_reject(Some((dnet, case.dadr)), None, 255));
        ctx.data_attributes = attributes();
        h.handle(ctx).await;

        let mut relayed = h.drain(0);
        assert_eq!(relayed.len(), 1, "DNET {dnet}");
        let (npdu, mac, data_attributes) = unicast(relayed.pop().unwrap());
        assert_eq!(npdu, case.npdu, "DNET {dnet}");
        assert_eq!(mac.as_slice(), case.link_mac, "DNET {dnet}");
        assert_eq!(data_attributes, attributes());
        assert!(h.drain(1).is_empty(), "DNET {dnet}: nothing goes back");
    }
}

#[tokio::test]
async fn relay_drops_a_received_reject_it_cannot_route() {
    let cases = [
        // No DNET: addressed to this router, even with SNET/SADR present.
        ("no DNET", received_reject(None, Some((1000, &[0x0A])), 255)),
        (
            "global DNET",
            received_reject(Some((0xFFFF, &[])), None, 255),
        ),
        (
            "arrival network",
            received_reject(Some((2000, &[0x20])), None, 255),
        ),
        (
            "unknown DNET",
            received_reject(Some((6000, &[0x60])), None, 255),
        ),
        (
            "no hops left",
            received_reject(Some((3000, &[0x30])), None, 0),
        ),
    ];
    for (case, npdu) in cases {
        let mut h = relay_harness();
        h.handle(h.ctx(1, &[2], npdu)).await;
        assert!(h.drain(0).is_empty(), "{case}");
        assert!(
            h.drain(1).is_empty(),
            "{case}: never answered with a reject"
        );
    }
}

/// A reject from [0B] on port 1 (network 2000), reason 2 for network 3000,
/// with the given DNET and one-octet DADR.
fn busy_3000_reject(h: &Harness, destination: Option<(u16, u8)>) -> (Npdu, IngressContext) {
    let mut npdu = received_reject(None, None, 255);
    npdu.destination = destination.map(|(network, mac)| NpduAddress {
        network,
        mac_address: MacAddr::from_slice(&[mac]),
    });
    npdu.payload = Bytes::from_static(&[0x02, 0x0B, 0xB8]);
    let mut ctx = h.ctx(1, &[0x0B], npdu.clone());
    ctx.data_attributes = attributes();
    (npdu, ctx)
}

#[tokio::test]
async fn a_reject_addressed_to_the_router_reaches_its_consumer_and_the_table() {
    // The router's own MACs are [01] on 1000 and [02] on 2000.
    let cases = [
        ("no DNET", None),
        ("own MAC on the arrival network", Some((2000, PORT_MACS[1]))),
        (
            "own MAC on the other port's network",
            Some((1000, PORT_MACS[0])),
        ),
    ];
    for (case, destination) in cases {
        let mut h = relay_harness();
        let mut controls = h.network_control();
        let (npdu, ctx) = busy_3000_reject(&h, destination);
        h.handle(ctx).await;

        let control = controls.try_recv().expect(case);
        assert_eq!(control.npdu, npdu, "{case}");
        assert_eq!(control.source_mac.as_slice(), [0x0B], "{case}");
        assert_eq!(control.data_attributes, attributes(), "{case}");
        assert_eq!(control.ingress_sequence, 1, "{case}");
        assert!(controls.try_recv().is_err(), "{case}");
        assert_eq!(
            h.table().effective_reachability(3000, h.now()),
            Some(Reachability::Busy),
            "{case}: the table still learns from it"
        );
        assert!(h.drain(0).is_empty(), "{case}: not relayed");
        assert!(h.drain(1).is_empty(), "{case}: not relayed");
    }
}

#[tokio::test]
async fn a_reject_for_another_node_or_a_denied_one_skips_the_routers_consumer() {
    // Meant for other nodes, so relayed out port 0: a device on 1000, and
    // the router's port 1 MAC paired with the wrong network.
    for dadr in [0x0A, PORT_MACS[1]] {
        let mut h = relay_harness();
        let mut controls = h.network_control();
        let (_, ctx) = busy_3000_reject(&h, Some((1000, dadr)));
        h.handle(ctx).await;
        assert_eq!(h.drain(0).len(), 1, "DADR {dadr:02X}");
        assert!(controls.try_recv().is_err(), "DADR {dadr:02X}");
    }

    // A reject the control policy denies changes nothing and reaches no one.
    let mut h = Harness::with_gate(relay_table(), Arc::new(ControlGate::hardened()));
    let mut controls = h.network_control();
    let (_, ctx) = busy_3000_reject(&h, None);
    h.handle(ctx).await;
    assert!(controls.try_recv().is_err());
    assert_eq!(
        h.table().effective_reachability(3000, h.now()),
        Some(Reachability::Reachable)
    );
    h.assert_quiet();
}
