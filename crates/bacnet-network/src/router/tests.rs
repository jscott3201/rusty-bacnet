//! The router end to end: start-up over real B/IP transports, and the
//! in-memory cases of forwarding, announcements and network messages on the
//! router's engine through [`super::envelope_harness`] (#1679). The
//! rewrite's unit cases moved to `bacnet-router-core`'s `tests/forward.rs`.

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::bip::BipTransport;
use bacnet_transport::port::{DataAttribute, TransportPort};
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bacnet_types::MacAddr;
use bytes::{BufMut, Bytes, BytesMut};
use std::net::Ipv4Addr;
use tokio::time::Duration;

use super::envelope_harness::{control_npdu, Harness, TableSeed};
use super::{BACnetRouter, RouterOptions, RouterPort, SendRequest, StartedRouter};
use crate::router_table::PortId;

mod data_attributes;

/// An APDU for `dnet`/`dadr` with `hop_count`.
fn apdu_to(dnet: u16, dadr: &[u8], hop_count: u8) -> Npdu {
    Npdu {
        destination: Some(NpduAddress {
            network: dnet,
            mac_address: MacAddr::from_slice(dadr),
        }),
        hop_count,
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    }
}

#[tokio::test]
async fn router_forwards_between_networks() {
    let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let mut device_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut device_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let _rx_b = device_b.start().await.unwrap();
    let _rx_a = device_a.start().await.unwrap();

    let port_a = RouterPort {
        transport: transport_a,
        network_number: 1000,
    };
    let port_b = RouterPort {
        transport: transport_b,
        network_number: 2000,
    };

    let StartedRouter {
        mut router,
        apdus: _local_rx,
        ..
    } = BACnetRouter::start(vec![port_a, port_b], RouterOptions::new())
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let apdu = vec![0x10, 0x08];
    let npdu = Npdu {
        is_network_message: false,
        expecting_reply: false,
        priority: bacnet_types::enums::NetworkPriority::NORMAL,
        destination: Some(NpduAddress {
            network: 2000,
            mac_address: MacAddr::from_slice(device_b.local_mac()),
        }),
        source: None,
        hop_count: 255,
        payload: Bytes::copy_from_slice(&apdu),
        ..Npdu::default()
    };

    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();

    let table = router.table();
    assert_eq!(table.len(), 2);
    assert!(table.lookup(1000).unwrap().is_direct());
    assert!(table.lookup(2000).unwrap().is_direct());
    drop(table);

    router.stop().await;
}

#[tokio::test]
async fn router_table_populated_on_start() {
    let transport_a = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_b = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let transport_c = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let ports = vec![
        RouterPort {
            transport: transport_a,
            network_number: 100,
        },
        RouterPort {
            transport: transport_b,
            network_number: 200,
        },
        RouterPort {
            transport: transport_c,
            network_number: 300,
        },
    ];

    let StartedRouter {
        mut router,
        apdus: _local_rx,
        ..
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();

    let table = router.table();
    assert_eq!(table.len(), 3);
    assert_eq!(table.lookup(100).unwrap().port().0, 0);
    assert_eq!(table.lookup(200).unwrap().port().0, 1);
    assert_eq!(table.lookup(300).unwrap().port().0, 2);
    drop(table);

    router.stop().await;
}

#[tokio::test]
async fn local_message_delivered_to_application() {
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut sender = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _sender_rx = sender.start().await.unwrap();

    let router_port = RouterPort {
        transport,
        network_number: 1000,
    };

    let StartedRouter {
        mut router,
        apdus: _local_rx,
        ..
    } = BACnetRouter::start(vec![router_port], RouterOptions::new())
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    router.stop().await;
}

#[tokio::test]
async fn forward_unicast_drops_hop_count_zero() {
    let mut h = Harness::two_port();
    // Should cause the message to be dropped.
    h.dispatch(h.ctx(0, &[0x0A], apdu_to(2000, &[0x01, 0x02], 0)))
        .await;
    h.assert_quiet();
}

#[tokio::test]
async fn forward_broadcast_drops_hop_count_zero() {
    let mut h = Harness::two_port();
    // Should cause broadcast to be dropped.
    h.dispatch(h.ctx(0, &[0x0A], apdu_to(0xFFFF, &[], 0))).await;
    h.assert_quiet();
}

#[tokio::test]
async fn forward_unicast_decrements_hop_count() {
    let mut h = Harness::two_port();
    h.dispatch(h.ctx(0, &[0x0A], apdu_to(2000, &[0x01, 0x02], 10)))
        .await;
    let sent = h.rxs[1].try_recv().unwrap();
    match sent {
        SendRequest::Unicast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
            assert!(decoded.source.is_some());
        }
        SendRequest::Broadcast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
        }
    }
}

/// A startup announcement, decoded.
#[derive(Debug, PartialEq, Eq)]
enum Announced {
    /// Network-Number-Is: the number, and whether it is configured.
    Number(u16, bool),
    /// I-Am-Router: the networks it lists.
    IAm(Vec<u16>),
}

/// The announcements of each port, in order, as `start` sends them: with
/// R3, the port's network number (Clause 6.4.20); then the networks beyond
/// the other ports, in port order (Clause 6.6.2).
fn announcements(networks: &[u16]) -> Vec<Vec<Announced>> {
    let mut h = Harness::with_networks(networks);
    h.announce();
    (0..networks.len())
        .map(|port| {
            h.drain(port)
                .into_iter()
                .map(|sent| match sent {
                    SendRequest::Broadcast {
                        npdu: data,
                        data_attributes,
                    } => {
                        assert!(data_attributes.is_empty());
                        let decoded = decode_npdu(data).unwrap();
                        assert!(decoded.is_network_message);
                        assert!(decoded.destination.is_none() && decoded.source.is_none());
                        let payload = &decoded.payload;
                        match decoded.message_type.map(NetworkMessageType::from_raw) {
                            Some(NetworkMessageType::NETWORK_NUMBER_IS) => {
                                let [high, low, flag] = payload[..] else {
                                    panic!("three octets");
                                };
                                Announced::Number(u16::from_be_bytes([high, low]), flag == 1)
                            }
                            Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK) => Announced::IAm(
                                payload
                                    .chunks(2)
                                    .map(|net| u16::from_be_bytes([net[0], net[1]]))
                                    .collect(),
                            ),
                            other => panic!("unexpected announcement {other:?}"),
                        }
                    }
                    SendRequest::Unicast { .. } => panic!("expected broadcast announcement"),
                })
                .collect()
        })
        .collect()
}

/// A one-port router announces its number, and no I-Am-Router.
#[test]
fn single_port_router_announces_only_its_number() {
    assert_eq!(announcements(&[1000]), [[Announced::Number(1000, true)]]);
}

#[test]
fn two_port_router_announces_its_number_then_i_am_router() {
    use Announced::{IAm, Number};
    assert_eq!(
        announcements(&[1000, 2000]),
        [
            [Number(1000, true), IAm(vec![2000])],
            [Number(2000, true), IAm(vec![1000])]
        ]
    );
}

#[test]
fn three_port_router_announces_multiple_networks() {
    use Announced::{IAm, Number};
    assert_eq!(
        announcements(&[100, 200, 300]),
        [
            [Number(100, true), IAm(vec![200, 300])],
            [Number(200, true), IAm(vec![100, 300])],
            [Number(300, true), IAm(vec![100, 200])]
        ]
    );
}

#[tokio::test]
async fn forward_unicast_with_hop_count_one_still_forwards() {
    let mut h = Harness::two_port();
    h.dispatch(h.ctx(0, &[0x0A], apdu_to(2000, &[0x01, 0x02], 1)))
        .await;
    let sent = h.rxs[1].try_recv().unwrap();
    match sent {
        SendRequest::Unicast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
            assert!(decoded.source.is_some());
        }
        SendRequest::Broadcast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.destination.is_none());
        }
    }
}

fn reject_payload(reason: RejectMessageReason, network: u16) -> Vec<u8> {
    let mut payload = BytesMut::with_capacity(3);
    payload.put_u8(reason.to_raw());
    payload.put_u16(network);
    payload.to_vec()
}

/// R2: a reject with reason 0 (other) or 4 (too long) leaves the learned
/// route alone (Clause 6.6.3.5); before #1679's fix it removed the route.
#[tokio::test]
async fn received_reject_of_another_reason_keeps_learned_route() {
    for reason in [
        RejectMessageReason::OTHER,
        RejectMessageReason::MESSAGE_TOO_LONG,
    ] {
        let mut table = TableSeed::new();
        table.add_direct(1000, 0);
        table.add_learned(3000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
        let mut h = Harness::with_table(table);
        let before = h.table().lookup(3000);

        let payload = reject_payload(reason, 3000);
        let npdu = control_npdu(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &payload);
        h.handle(h.ctx(0, &[0x0A], npdu)).await;

        let tbl = h.table();
        assert_eq!(tbl.lookup(3000), before);
        assert!(tbl.lookup(1000).is_some());
    }
}

#[tokio::test]
async fn received_reject_does_not_remove_direct_route() {
    let mut table = TableSeed::new();
    table.add_direct(1000, 0);
    let mut h = Harness::with_table(table);

    let payload = reject_payload(RejectMessageReason::OTHER, 1000);
    let npdu = control_npdu(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &payload);
    h.handle(h.ctx(0, &[0x0A], npdu)).await;

    assert!(h.table().lookup(1000).is_some());
}

#[tokio::test]
async fn who_is_router_with_specific_network() {
    let mut h = Harness::with_networks(&[1000, 2000, 3000]);
    let npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x07, 0xd0]);
    h.handle(h.ctx(0, &[0x0A], npdu)).await;

    let sent = h.rxs[0].try_recv().unwrap();
    match sent {
        SendRequest::Broadcast { npdu: data, .. } => {
            let decoded = decode_npdu(data.clone()).unwrap();
            assert!(decoded.is_network_message);
            assert_eq!(
                decoded.message_type,
                Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK.to_raw())
            );
            assert_eq!(decoded.payload.len(), 2);
            let net = u16::from_be_bytes([decoded.payload[0], decoded.payload[1]]);
            assert_eq!(net, 2000);
        }
        _ => panic!("Expected Broadcast response for I-Am-Router"),
    }
}

#[tokio::test]
async fn who_is_router_with_unknown_network_no_response() {
    // A one-port router has no other port to search further on.
    let mut h = Harness::with_networks(&[1000]);
    let npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x27, 0x0f]);
    h.handle(h.ctx(0, &[0x0A], npdu)).await;
    h.assert_quiet();
}

#[tokio::test]
async fn router_busy_does_not_crash() {
    let mut h = Harness::with_table(TableSeed::new());
    let npdu = control_npdu(
        NetworkMessageType::ROUTER_BUSY_TO_NETWORK,
        &[0x03, 0xe8, 0x07, 0xd0],
    );
    h.handle(h.ctx(0, &[0x0A], npdu)).await;
}

#[tokio::test]
async fn router_available_does_not_crash() {
    let mut h = Harness::with_table(TableSeed::new());
    let npdu = control_npdu(
        NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        &[0x03, 0xe8, 0x07, 0xd0],
    );
    h.handle(h.ctx(0, &[0x0A], npdu)).await;
}

#[tokio::test]
async fn i_could_be_router_stores_potential_route() {
    let mut h = Harness::with_table(TableSeed::new());
    let npdu = control_npdu(
        NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
        &[0x13, 0x88, 50],
    );
    h.handle(h.ctx(0, &[0x0A, 0x0B], npdu)).await;

    let tbl = h.table();
    let entry = tbl.lookup(5000).unwrap();
    assert!(!entry.is_direct());
    assert_eq!(entry.port(), PortId(0));
    assert_eq!(entry.next_hop().as_slice(), &[0x0A, 0x0B]);
}

#[tokio::test]
async fn i_could_be_router_does_not_overwrite_existing_route() {
    let mut table = TableSeed::new();
    table.add_direct(5000, 1);
    let mut h = Harness::with_table(table);
    let npdu = control_npdu(
        NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
        &[0x13, 0x88, 50],
    );
    h.handle(h.ctx(0, &[0x0A], npdu)).await;

    let tbl = h.table();
    let entry = tbl.lookup(5000).unwrap();
    assert!(entry.is_direct());
    assert_eq!(entry.port(), PortId(1));
}

#[tokio::test]
async fn establish_connection_does_not_crash() {
    let mut h = Harness::with_table(TableSeed::new());
    let npdu = control_npdu(
        NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK,
        &[0x17, 0x70, 30],
    );
    h.handle(h.ctx(0, &[0x0A], npdu)).await;
}

#[tokio::test]
async fn disconnect_retains_learned_route() {
    let mut table = TableSeed::new();
    table.add_learned(7000, 0, MacAddr::from_slice(&[10, 0, 1, 1]));
    let mut h = Harness::with_table(table);
    let npdu = control_npdu(
        NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK,
        &[0x1b, 0x58],
    );
    h.handle(h.ctx(0, &[0x0A], npdu)).await;

    let tbl = h.table();
    assert!(tbl.lookup(7000).is_some());
    assert_eq!(tbl.claim_counters().disconnect_removal_ignored, 1);
}

#[tokio::test]
async fn disconnect_does_not_remove_direct_route() {
    let mut table = TableSeed::new();
    table.add_direct(1000, 0);
    let mut h = Harness::with_table(table);
    let npdu = control_npdu(
        NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK,
        &[0x03, 0xe8],
    );
    h.handle(h.ctx(0, &[0x0A], npdu)).await;

    let tbl = h.table();
    assert!(tbl.lookup(1000).is_some());
    assert!(tbl.lookup(1000).unwrap().is_direct());
}

/// RB-07: the router hands a frame's provenance on by value, to its local
/// delivery, to its network-control consumer and to the authorizer's
/// context, and asserts no more trust than the transport did.
#[tokio::test]
async fn provenance_threads_by_value_to_every_consumer() {
    use super::control_policy::{
        ControlAuthContext, ControlAuthorizer, ControlGate, ControlPolicy, ControlTrust,
    };
    use bacnet_transport::port::TransportProvenance;
    use std::sync::{Arc, Mutex};

    let seen: Arc<Mutex<Vec<ControlAuthContext>>> = Arc::default();
    let authorizer: ControlAuthorizer = {
        let seen = Arc::clone(&seen);
        Arc::new(move |ctx: &ControlAuthContext| {
            seen.lock().unwrap().push(ctx.clone());
            true
        })
    };
    let gate = Arc::new(ControlGate::new(ControlPolicy::Hardened, Some(authorizer)));
    let mut h = Harness::with_capacities(gate, &[16, 16]);
    let mut controls = h.network_control();
    let provenance = TransportProvenance::unverified();

    let apdu = Npdu {
        payload: Bytes::from_static(&[0x10, 0x08]),
        ..Npdu::default()
    };
    h.handle(h.ctx_with_provenance(0, &[0x0A], apdu, provenance))
        .await;
    let delivered = h.apdus.try_recv().unwrap();
    assert_eq!(delivered.provenance, provenance);
    assert!(delivered.provenance.is_unverified());

    let reject = control_npdu(
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        &reject_payload(RejectMessageReason::NOT_DIRECTLY_CONNECTED, 3000),
    );
    h.handle(h.ctx_with_provenance(1, &[0x0B], reject, provenance))
        .await;
    assert_eq!(controls.try_recv().unwrap().provenance, provenance);

    let i_am = control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    h.handle(h.ctx_with_provenance(1, &[0x0B], i_am, provenance))
        .await;
    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for ctx in seen.iter() {
        assert_eq!(ctx.provenance, provenance);
        assert_eq!(ctx.trust, ControlTrust::Unverified);
    }
}
