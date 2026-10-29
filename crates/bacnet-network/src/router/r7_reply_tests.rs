//! R7 responses retain the endpoint a strict requester addressed.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_npdu::NetworkPriority;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::{MacAddr, error::Error};
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;

use super::{BACnetRouter, RouterOptions, RouterPort};
use crate::layer::{IssuedApdu, NetworkLayer};

#[derive(Clone)]
struct Port {
    incoming: Arc<Mutex<Option<mpsc::Receiver<ReceivedNpdu>>>>,
    wire: mpsc::UnboundedSender<(Bytes, Vec<u8>)>,
    mac: [u8; 1],
}

impl TransportPort for Port {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.incoming.lock().unwrap().take().unwrap())
    }
    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.wire
            .send((Bytes::copy_from_slice(npdu), mac.to_vec()))
            .unwrap();
        Ok(())
    }
    async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        &self.mac
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        1476
    }
}

type WireReceiver = mpsc::UnboundedReceiver<(Bytes, Vec<u8>)>;

fn port(mac: u8) -> (Port, mpsc::Sender<ReceivedNpdu>, WireReceiver) {
    let (frames, incoming) = mpsc::channel(8);
    let (wire, sent) = mpsc::unbounded_channel();
    (
        Port {
            incoming: Arc::new(Mutex::new(Some(incoming))),
            wire,
            mac: [mac],
        },
        frames,
        sent,
    )
}

fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}

#[tokio::test]
async fn r7_response_matches_addressed_endpoint_and_invoke_id() {
    let (a, frames, mut wire_a) = port(1);
    let (b, _, mut wire_b) = port(2);
    let network = NetworkLayer::new(a.clone());
    let mut started = BACnetRouter::start(
        vec![
            RouterPort {
                transport: a,
                network_number: 1000,
            },
            RouterPort {
                transport: b,
                network_number: 2000,
            },
        ],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    let request = Npdu {
        destination: Some(address(2000, &[2])),
        expecting_reply: true,
        payload: Bytes::from_static(&[0x00, 0x05, 0x37, 0x0c]),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &request).unwrap();
    frames
        .send(ReceivedNpdu::unverified(
            bytes.freeze(),
            MacAddr::from_slice(&[9]),
            false,
            vec![],
            None,
        ))
        .await
        .unwrap();
    let received = tokio::time::timeout(Duration::from_secs(1), started.apdus.recv())
        .await
        .unwrap()
        .unwrap();
    network
        .send_response_apdu_on_issuance(
            IssuedApdu {
                apdu: &[0x20, 0x37, 0x0c],
                next_hop: &received.source_mac,
                destination: received.source_network.as_ref(),
                expecting_reply: false,
                priority: NetworkPriority::Normal,
            },
            &received.response_route(),
            || {},
        )
        .await
        .unwrap();
    let (bytes, peer) = tokio::time::timeout(Duration::from_secs(1), wire_a.recv())
        .await
        .unwrap()
        .unwrap();
    let response = decode_npdu(bytes).unwrap();
    assert_eq!(peer, [9], "reply to the captured immediate peer");
    assert_eq!(
        response.payload.as_ref(),
        [0x20, 0x37, 0x0c],
        "strict invoke ID match"
    );
    assert_eq!(
        response.source,
        Some(address(2000, &[2])),
        "strict endpoint identity match"
    );
    assert!(response.destination.is_none());
    assert!(wire_b.try_recv().is_err());
    started.router.stop().await.unwrap();
}

fn poll_once<F: std::future::Future>(future: std::pin::Pin<&mut F>) -> std::task::Poll<F::Output> {
    future.poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
}

fn issued() -> IssuedApdu<'static> {
    // Deliberately different from the captured router peer: a saved context
    // governs return routing, not whichever NetworkLayer a caller supplies.
    IssuedApdu {
        apdu: &[0x20, 0x37, 12],
        next_hop: &[77],
        destination: None,
        expecting_reply: false,
        priority: NetworkPriority::Normal,
    }
}

async fn saved(
    h: &mut super::envelope_harness::Harness,
    destination: Option<NpduAddress>,
    source: Option<NpduAddress>,
) -> crate::layer::ReceivedApdu {
    let npdu = Npdu {
        destination,
        source,
        expecting_reply: true,
        payload: Bytes::from_static(&[0, 5, 0x37, 12]),
        ..Npdu::default()
    };
    h.dispatch(h.ctx(0, &[9], npdu)).await;
    h.apdus.try_recv().unwrap()
}

#[tokio::test]
async fn r7_clone_retains_owned_route_and_slice_without_repeating_der_authority() {
    let (a, frames, mut wire_a) = port(1);
    let (b, _, mut wire_b) = port(2);
    let (fallback, _, mut unrelated) = port(99);
    let network = NetworkLayer::new(fallback);
    let mut started = BACnetRouter::start(
        vec![
            RouterPort {
                transport: a,
                network_number: 1000,
            },
            RouterPort {
                transport: b,
                network_number: 2000,
            },
        ],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    let request = Npdu {
        destination: Some(address(2000, &[2])),
        source: Some(address(9000, &[3, 4])),
        expecting_reply: true,
        payload: Bytes::from_static(&[0, 5, 0x37, 12]),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &request).unwrap();
    let bytes = bytes.freeze();
    let apdu_pointer = bytes[bytes.len() - 4..].as_ptr();
    let (tx, rx) = tokio::sync::oneshot::channel();
    frames
        .send(ReceivedNpdu::unverified(
            bytes,
            MacAddr::from_slice(&[9]),
            false,
            vec![],
            Some(tx),
        ))
        .await
        .unwrap();
    let received = started.apdus.recv().await.unwrap();
    assert_eq!(
        received.apdu.as_ptr(),
        apdu_pointer,
        "local APDU stays a slice"
    );
    let clone = received.clone();
    assert!(clone.reply_tx.is_none());
    assert!(
        clone
            .response_route()
            .same_origin(&received.response_route())
    );
    let route = clone.response_route();
    assert_eq!(
        route.max_apdu_length(u16::MAX, None).unwrap(),
        1485,
        "12-octet source+destination header"
    );
    // Exercise the engine scratch after saving the route.
    frames
        .send(ReceivedNpdu::unverified(
            Bytes::from_static(&[1, 0x80, 0]),
            MacAddr::from_slice(&[8]),
            false,
            vec![],
            None,
        ))
        .await
        .unwrap();
    network
        .send_response_apdu_on_issuance(issued(), &route, || {})
        .await
        .unwrap();
    let (bytes, peer) = wire_a.recv().await.unwrap();
    assert_eq!(peer, [9]);
    assert_eq!(
        bytes.as_ref(),
        [
            1, 0x28, 0x23, 0x28, 2, 3, 4, 7, 0xd0, 1, 2, 255, 0x20, 0x37, 12
        ]
    );
    assert!(wire_b.try_recv().is_err() && unrelated.try_recv().is_err());
    drop(received);
    assert!(
        rx.await.is_err(),
        "only the original envelope held DER authority"
    );
    started.router.stop().await.unwrap();
    assert!(
        network
            .send_response_apdu_on_issuance(issued(), &route, || {})
            .await
            .is_err()
    );
    assert!(
        route
            .encode_reply_npdu(&[0x20, 0x37, 12], None, false, NetworkPriority::Normal)
            .is_err()
    );
    assert!(wire_a.try_recv().is_err() && unrelated.try_recv().is_err());
}

#[tokio::test]
async fn r7_queued_reply_cancellation_and_both_owner_retirements_prevent_issuance() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    for mode in 0..3 {
        let mut h = super::envelope_harness::Harness::two_port();
        let received = saved(&mut h, Some(address(2000, &[2])), None).await;
        let route = received.response_route();
        let (transport, _, mut wire) = port(99);
        let network = NetworkLayer::new(transport);
        let issued_count = Arc::new(AtomicUsize::new(0));
        let count = issued_count.clone();
        let mut send =
            Box::pin(
                network.send_response_apdu_on_issuance(issued(), &route, move || {
                    count.fetch_add(1, Ordering::SeqCst);
                }),
            );
        assert!(poll_once(send.as_mut()).is_pending());
        let mut queued = h.drain(0);
        assert_eq!(queued.len(), 1);
        let super::SendRequest::Unicast {
            response: Some(response),
            ..
        } = queued.remove(0)
        else {
            panic!("queued reply")
        };
        assert_eq!(
            issued_count.load(Ordering::SeqCst),
            0,
            "queue admission is not transport issuance"
        );
        match mode {
            0 => {
                drop(send);
                assert!(response.start().is_none());
            }
            1 => {
                network.seal_responses();
                assert!(response.start().is_none());
                assert!(send.await.is_err());
            }
            _ => {
                drop(h);
                assert!(response.start().is_none());
                assert!(send.await.is_err());
            }
        }
        assert_eq!(issued_count.load(Ordering::SeqCst), 0);
        assert!(wire.try_recv().is_err());
    }
}

#[tokio::test]
async fn r7_full_queue_is_bounded_and_issuance_callback_runs_only_when_selected() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut h = super::envelope_harness::Harness::two_port();
    let received = saved(&mut h, Some(address(2000, &[2])), None).await;
    let route = received.response_route();
    let (transport, _, _) = port(99);
    let network = NetworkLayer::new(transport);
    let count = Arc::new(AtomicUsize::new(0));
    let mut sends = Vec::new();
    for _ in 0..16 {
        let count = count.clone();
        let mut send =
            Box::pin(
                network.send_response_apdu_on_issuance(issued(), &route, move || {
                    count.fetch_add(1, Ordering::SeqCst);
                }),
            );
        assert!(poll_once(send.as_mut()).is_pending());
        sends.push(send);
    }
    assert!(
        network
            .send_response_apdu_on_issuance(issued(), &route, || panic!("full queue cannot issue"))
            .await
            .is_err()
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    for queued in h.drain(0) {
        let super::SendRequest::Unicast {
            response: Some(response),
            ..
        } = queued
        else {
            panic!("queued reply")
        };
        response.start().unwrap().send(Ok(())).unwrap();
    }
    for send in sends {
        send.await.unwrap();
    }
    assert_eq!(count.load(Ordering::SeqCst), 16);
}

#[tokio::test]
async fn r7_prompt_handoff_checks_both_network_and_router_lifetime() {
    for seal_network in [false, true] {
        let mut h = super::envelope_harness::Harness::two_port();
        let received = saved(&mut h, Some(address(2000, &[2])), Some(address(9000, &[3]))).await;
        let route = received.response_route();
        let (transport, _, _) = port(99);
        let network = NetworkLayer::new(transport);
        let bytes = network
            .encode_response_for_handoff(issued(), &route)
            .unwrap();
        let parsed = decode_npdu(bytes.freeze()).unwrap();
        assert_eq!(parsed.source, Some(address(2000, &[2])));
        assert_eq!(parsed.destination, Some(address(9000, &[3])));
        if seal_network {
            network.seal_responses();
        } else {
            drop(h);
        }
        assert!(
            network
                .encode_response_for_handoff(issued(), &route)
                .is_err()
        );
    }
}
