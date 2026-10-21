//! RB-06: freshness split, standard vs hardened convergence, bounded discovery.
//!
//! Clauses 6.6.3.1 (a reject goes out only after both the table lookup and
//! Who-Is discovery come up empty), 6.6.3.2 (a later I-Am for a network
//! supersedes earlier ones), 6.6.3.3 (replace MAC/port on difference) and
//! 6.5 (attempt Who-Is when the next router is unknown).
//! In-memory only, no sleeps. The table-level and discovery cases moved to
//! `bacnet-router-core`'s `tests/table.rs`, `tests/table_claims.rs` and
//! `tests/discovery.rs` in #1688; these run the router on its engine.

use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::port::{DataAttribute, ReceivedNpdu, TransportProvenance};
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::envelope_harness::{broadcast_data, control_npdu, Harness};
use super::{BACnetRouter, RouterOptions, RouterPort, SendRequest, StartedRouter};
use crate::router_table::PortId;

// --- Router harness: directed unknown solicits, coalesces, learns ---

#[tokio::test]
async fn directed_unknown_solicits_once_then_rejects_twice_coalesced() {
    let mut h = Harness::two_port();
    for _ in 0..2 {
        let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x27, 0x0f]);
        npdu.destination = Some(NpduAddress {
            network: 9999,
            mac_address: MacAddr::from_slice(&[9]),
        });
        npdu.hop_count = 255;
        h.dispatch(h.ctx(0, &[7], npdu)).await;
    }
    let solicited = broadcast_data(h.drain(1));
    assert_eq!(solicited.len(), 1);
    let who_is = decode_npdu(solicited[0].clone()).unwrap();
    assert_eq!(
        who_is.message_type,
        Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
    );
    assert_eq!(who_is.payload.as_ref(), &[0x27, 0x0f]);
    let rejects = h.drain(0);
    assert_eq!(rejects.len(), 2);
    for req in rejects {
        match req {
            SendRequest::Unicast { npdu, mac, .. } => {
                assert_eq!(mac.as_slice(), &[7]);
                let decoded = decode_npdu(npdu).unwrap();
                assert_eq!(
                    decoded.message_type,
                    Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
                );
                assert_eq!(
                    decoded.payload.as_ref(),
                    &[
                        RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
                        0x27,
                        0x0f
                    ]
                );
            }
            SendRequest::Broadcast { .. } => panic!("expected unicast reject"),
        }
    }
}

#[tokio::test]
async fn directed_unknown_discovered_via_answer_routes_next_packet() {
    let mut h = Harness::two_port();
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.destination = Some(NpduAddress {
        network: 3000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;
    assert_eq!(broadcast_data(h.drain(1)).len(), 1);
    assert_eq!(h.drain(0).len(), 1);
    assert!(h.table().lookup(3000).is_none());
    // Answer installs the route (standard last-wins, no corroboration wait).
    h.handle(h.ctx(
        1,
        &[2],
        control_npdu(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[0x0b, 0xb8]),
    ))
    .await;
    // Consume the unconditional I-Am rebroadcast before the next dispatch.
    for port in 0..2 {
        for req in h.drain(port) {
            match req {
                SendRequest::Broadcast { npdu, .. } => {
                    assert_eq!(
                        decode_npdu(npdu).unwrap().message_type,
                        Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK)
                    );
                }
                SendRequest::Unicast { .. } => panic!("expected only I-Am rebroadcast"),
            }
        }
    }
    assert!(h.table().lookup(3000).is_some());
    assert_eq!(h.table().lookup(3000).unwrap().port(), PortId(1));
    // Next packet to the now-known network forwards instead of rejecting.
    let mut npdu = control_npdu(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK, &[0x0b, 0xb8]);
    npdu.destination = Some(NpduAddress {
        network: 3000,
        mac_address: MacAddr::from_slice(&[9]),
    });
    npdu.hop_count = 255;
    h.dispatch(h.ctx(0, &[7], npdu)).await;
    assert!(h.drain(0).is_empty());
    match h.drain(1).pop().expect("expected forward") {
        SendRequest::Unicast { mac, .. } => assert_eq!(mac.as_slice(), &[2]),
        SendRequest::Broadcast { .. } => panic!("expected unicast forward"),
    }
}

// --- Real router: APDU unknown path + stop cancellation ---

struct MemTransport {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    outgoing: mpsc::Sender<SendRequest>,
    mac: [u8; 1],
}

impl bacnet_transport::port::TransportPort for MemTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, bacnet_types::error::Error> {
        self.incoming
            .take()
            .ok_or_else(|| bacnet_types::error::Error::Encoding("started".into()))
    }
    async fn stop(&mut self) -> Result<(), bacnet_types::error::Error> {
        Ok(())
    }
    async fn send_unicast(
        &self,
        npdu: &[u8],
        mac: &[u8],
    ) -> Result<(), bacnet_types::error::Error> {
        self.send_unicast_with_data_attributes(npdu, mac, &[]).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), bacnet_types::error::Error> {
        self.send_broadcast_with_data_attributes(npdu, &[]).await
    }
    async fn send_unicast_with_data_attributes(
        &self,
        npdu: &[u8],
        mac: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), bacnet_types::error::Error> {
        self.outgoing
            .try_send(SendRequest::Unicast {
                npdu: Bytes::copy_from_slice(npdu),
                mac: MacAddr::from_slice(mac),
                data_attributes: data_attributes.to_vec(),

                response: None,
                reject_origin: None,
            })
            .map_err(|_| bacnet_types::error::Error::Encoding("wire full".into()))
    }
    async fn send_broadcast_with_data_attributes(
        &self,
        npdu: &[u8],
        data_attributes: &[DataAttribute],
    ) -> Result<(), bacnet_types::error::Error> {
        self.outgoing
            .try_send(SendRequest::Broadcast {
                npdu: Bytes::copy_from_slice(npdu),
                data_attributes: data_attributes.to_vec(),

                reject_origin: None,
            })
            .map_err(|_| bacnet_types::error::Error::Encoding("wire full".into()))
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }

    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
}

fn apdu_to(net: u16) -> Bytes {
    let npdu = Npdu {
        destination: Some(NpduAddress {
            network: net,
            mac_address: MacAddr::from_slice(&[9]),
        }),
        hop_count: 255,
        payload: Bytes::from_static(&[0x10, 0x20]),
        ..Npdu::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    buf.freeze()
}

fn ingress(npdu: Bytes) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu,
        source_mac: MacAddr::from_slice(&[7]),
        link_layer_group: true,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

#[tokio::test]
async fn apdu_unknown_solicits_rejects_then_forwards_after_answer() {
    let (tx0, rx0) = mpsc::channel(16);
    let (out0, mut wire0) = mpsc::channel(16);
    let (_tx1, _rx1) = mpsc::channel(16);
    let (out1, mut wire1) = mpsc::channel(16);
    let ports = vec![
        RouterPort {
            transport: MemTransport {
                incoming: Some(rx0),
                outgoing: out0,
                mac: [1],
            },
            network_number: 1000,
        },
        RouterPort {
            transport: MemTransport {
                incoming: Some(_rx1),
                outgoing: out1,
                mac: [2],
            },
            network_number: 2000,
        },
    ];
    let StartedRouter {
        mut router,
        apdus: _local,
        ..
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    for _ in [wire0.try_recv(), wire1.try_recv()] {}
    tx0.send(ingress(apdu_to(3000))).await.unwrap();
    // Unknown APDU: one Who-Is out the other port, one retryable reject back.
    let mut saw_who_is = false;
    let mut saw_reject = false;
    for _ in 0..4 {
        tokio::task::yield_now().await;
        while let Ok(req) = wire1.try_recv() {
            if let SendRequest::Broadcast { npdu, .. } = req {
                let decoded = decode_npdu(npdu).unwrap();
                if decoded.message_type == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
                    && decoded.payload.as_ref() == [0x0b, 0xb8]
                {
                    saw_who_is = true;
                }
            }
        }
        while let Ok(req) = wire0.try_recv() {
            if let SendRequest::Unicast { npdu, mac, .. } = req {
                let decoded = decode_npdu(npdu).unwrap();
                if decoded.message_type == Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
                    && decoded.payload.as_ref()
                        == [
                            RejectMessageReason::NOT_DIRECTLY_CONNECTED.to_raw(),
                            0x0b,
                            0xb8,
                        ]
                    && mac.as_slice() == [7]
                {
                    saw_reject = true;
                }
            }
        }
        if saw_who_is && saw_reject {
            break;
        }
    }
    assert!(saw_who_is, "expected bounded Who-Is solicitation");
    assert!(saw_reject, "expected honest retryable reject");
    // Answer installs the route; the retry forwards without a reject.
    {
        let now = router.now();
        let mac = bacnet_npdu::Mac::from_array([2]);
        let port = crate::router_table::PortId(1);
        let _ = router.table().learn(3000, port, mac, now);
    }
    tx0.send(ingress(apdu_to(3000))).await.unwrap();
    let mut forwarded = false;
    let mut rejected = false;
    for _ in 0..20 {
        tokio::task::yield_now().await;
        while let Ok(req) = wire1.try_recv() {
            if let SendRequest::Unicast { mac, .. } = req {
                if mac.as_slice() == [2] {
                    forwarded = true;
                }
            }
        }
        while let Ok(req) = wire0.try_recv() {
            if let SendRequest::Unicast { npdu, .. } = req {
                if decode_npdu(npdu).unwrap().message_type
                    == Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
                {
                    rejected = true;
                }
            }
        }
        if forwarded {
            break;
        }
    }
    assert!(forwarded, "expected forward after discovery");
    assert!(!rejected, "no reject once the route is known");
    router.stop().await;
}

#[tokio::test]
async fn stop_cancels_pending_discovery() {
    let (tx0, rx0) = mpsc::channel(16);
    let (out0, mut wire0) = mpsc::channel(16);
    let (_tx1, _rx1) = mpsc::channel(16);
    let (out1, _wire1) = mpsc::channel(16);
    let ports = vec![
        RouterPort {
            transport: MemTransport {
                incoming: Some(rx0),
                outgoing: out0,
                mac: [1],
            },
            network_number: 1000,
        },
        RouterPort {
            transport: MemTransport {
                incoming: Some(_rx1),
                outgoing: out1,
                mac: [2],
            },
            network_number: 2000,
        },
    ];
    let StartedRouter {
        mut router,
        apdus: _local,
        ..
    } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .unwrap();
    for _ in [wire0.try_recv()] {}
    tx0.send(ingress(apdu_to(4000))).await.unwrap();
    for _ in 0..10 {
        tokio::task::yield_now().await;
        if router.searches().0 == 1 {
            break;
        }
    }
    assert_eq!(router.searches(), (1, false));
    router.stop().await;
    assert_eq!(router.searches(), (0, true));
}
