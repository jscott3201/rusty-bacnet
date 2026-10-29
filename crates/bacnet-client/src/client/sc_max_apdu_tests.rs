use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Instant;

use bacnet_encoding::apdu::{self, Apdu, SegmentAck, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, decode_npdu, encode_npdu};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_transport::sc::{LoopbackWebSocket, ScReconnectConfig, ScTransport, WebSocketPort};
use bacnet_transport::sc_frame::{
    ScFunction, ScMessage, Vmac, decode_sc_message, encode_sc_message,
};
use bacnet_types::MacAddr;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};

use crate::discovery::DiscoveredDevice;

use super::BACnetClient;

/// A loopback transport whose egress APDU limit a test sets, and can change
/// while the client runs through the shared handle.
struct LimitedTransport {
    inner: LoopbackTransport,
    max_apdu_length: Arc<AtomicU16>,
}

impl LimitedTransport {
    fn new(inner: LoopbackTransport, max_apdu_length: u16) -> (Self, Arc<AtomicU16>) {
        let limit = Arc::new(AtomicU16::new(max_apdu_length));
        let transport = Self {
            inner,
            max_apdu_length: Arc::clone(&limit),
        };
        (transport, limit)
    }
}

impl TransportPort for LimitedTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, bacnet_types::error::Error> {
        self.inner.start().await
    }

    async fn stop(&mut self) -> Result<(), bacnet_types::error::Error> {
        self.inner.stop().await
    }

    async fn send_unicast(
        &self,
        npdu: &[u8],
        mac: &[u8],
    ) -> Result<(), bacnet_types::error::Error> {
        self.inner.send_unicast(npdu, mac).await
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), bacnet_types::error::Error> {
        self.inner.send_broadcast(npdu).await
    }

    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }

    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }

    fn egress_apdu_limit(&self) -> u16 {
        self.max_apdu_length.load(Ordering::Relaxed)
    }
}

async fn send_peer_apdu(
    transport: &LoopbackTransport,
    client_mac: &[u8],
    apdu: Apdu,
) -> Result<(), bacnet_types::error::Error> {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu)?;
    let npdu = Npdu {
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu)?;
    transport.send_unicast(&npdu_buf, client_mac).await
}

async fn send_peer_routed_apdu(
    transport: &LoopbackTransport,
    client_mac: &[u8],
    source_network: u16,
    source_mac: &[u8],
    apdu: Apdu,
) -> Result<(), bacnet_types::error::Error> {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu)?;
    let npdu = Npdu {
        source: Some(NpduAddress {
            network: source_network,
            mac_address: MacAddr::from_slice(source_mac),
        }),
        payload: apdu_buf.freeze(),
        ..Npdu::default()
    };
    let mut npdu_buf = BytesMut::new();
    encode_npdu(&mut npdu_buf, &npdu)?;
    transport.send_unicast(&npdu_buf, client_mac).await
}

async fn accept_sc_with_limits(
    ws_hub: &LoopbackWebSocket,
    hub_vmac: Vmac,
    hub_max_bvlc_length: u16,
    hub_max_npdu_length: u16,
) {
    let data = ws_hub.recv().await.unwrap();
    let req = decode_sc_message(&data).unwrap();
    assert_eq!(req.function, ScFunction::ConnectRequest);

    let mut payload = Vec::with_capacity(26);
    payload.extend_from_slice(&hub_vmac);
    payload.extend_from_slice(&[0x33; 16]);
    payload.extend_from_slice(&hub_max_bvlc_length.to_be_bytes());
    payload.extend_from_slice(&hub_max_npdu_length.to_be_bytes());

    let accept = ScMessage {
        function: ScFunction::ConnectAccept,
        message_id: req.message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &accept);
    ws_hub.send(&buf).await.unwrap();
}

async fn wait_for_client_transport_max_apdu_length<T: TransportPort + 'static>(
    client: &BACnetClient<T>,
    expected: u16,
    timeout_duration: Duration,
) {
    let deadline = Instant::now() + timeout_duration;
    loop {
        if client.transport_max_apdu_length() == expected {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for client transport max APDU {expected}, got {}",
            client.transport_max_apdu_length()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn client_segments_known_device_requests_to_live_transport_limit() {
    let client_mac = vec![0x01];
    let peer_mac = vec![0x02];
    let (client_inner, mut peer_transport) =
        LoopbackTransport::pair(client_mac.clone(), peer_mac.clone());
    let (client_transport, _) = LimitedTransport::new(client_inner, 478);
    let mut peer_rx = peer_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(5000)
        .build()
        .await
        .unwrap();
    assert_eq!(client.transport_max_apdu_length(), 478);
    assert_eq!(client.max_apdu_length(), 206);

    client.device_table.lock().await.upsert(DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap(),
        mac_address: MacAddr::from_slice(&peer_mac),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::BOTH,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: Instant::now(),
        source_network: None,
        source_address: None,
    });

    let service_data: Vec<u8> = (0..900).map(|i| (i % 251) as u8).collect();
    let expected_service_data = service_data.clone();
    let request_future = client.confirmed_request(
        &peer_mac,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &service_data,
    );
    let peer_future = async {
        let mut received_service_data = Vec::new();
        let invoke_id = loop {
            let received = timeout(Duration::from_secs(3), peer_rx.recv())
                .await
                .expect("timed out waiting for segmented NPDU")
                .expect("peer channel closed");
            assert!(
                received.npdu.len() <= 480,
                "NPDU exceeded simulated SC hub Max-NPDU-Length"
            );

            let npdu = decode_npdu(Bytes::copy_from_slice(&received.npdu)).unwrap();
            let decoded = apdu::decode_apdu(npdu.payload).unwrap();
            let Apdu::ConfirmedRequest(req) = decoded else {
                panic!("expected segmented ConfirmedRequest");
            };
            assert!(req.segmented);
            assert_eq!(req.max_apdu_length, 206);
            let seq = req.sequence_number.unwrap();
            received_service_data.extend_from_slice(&req.service_request);

            send_peer_apdu(
                &peer_transport,
                &client_mac,
                Apdu::SegmentAck(SegmentAck {
                    negative_ack: false,
                    sent_by_server: true,
                    invoke_id: req.invoke_id,
                    sequence_number: seq,
                    actual_window_size: 1,
                }),
            )
            .await
            .unwrap();

            if !req.more_follows {
                break req.invoke_id;
            }
        };

        send_peer_apdu(
            &peer_transport,
            &client_mac,
            Apdu::SimpleAck(SimpleAck {
                invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            }),
        )
        .await
        .unwrap();
        received_service_data
    };

    let (result, received_service_data) = tokio::join!(request_future, peer_future);

    assert!(result.unwrap().is_empty());
    assert_eq!(received_service_data, expected_service_data);

    client.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

#[tokio::test]
async fn client_routed_unsegmented_request_accounts_for_npdu_overhead_bucket() {
    let client_mac = vec![0x01];
    let router_mac = vec![0x02];
    let remote_network = 100;
    let remote_mac = vec![0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    let (client_inner, mut router_transport) =
        LoopbackTransport::pair(client_mac.clone(), router_mac.clone());
    let (client_transport, _) = LimitedTransport::new(client_inner, 1024);
    let mut router_rx = router_transport.start().await.unwrap();

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(5000)
        .build()
        .await
        .unwrap();
    assert_eq!(client.transport_max_apdu_length(), 1024);
    assert_eq!(client.max_apdu_length(), 1024);
    client
        .configure_routed_path_max_npdu(&router_mac, remote_network, 1497)
        .await
        .unwrap();

    let service_data: Vec<u8> = (0..900).map(|i| (i % 251) as u8).collect();
    let request_future = client.confirmed_request_routed(
        &router_mac,
        remote_network,
        &remote_mac,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &service_data,
    );
    let router_future = async {
        let received = timeout(Duration::from_secs(3), router_rx.recv())
            .await
            .expect("timed out waiting for routed NPDU")
            .expect("router channel closed");
        assert!(
            received.npdu.len() <= 1026,
            "routed NPDU exceeded simulated SC local Max-NPDU budget"
        );

        let npdu = decode_npdu(Bytes::copy_from_slice(&received.npdu)).unwrap();
        let destination = npdu.destination.expect("routed NPDU destination");
        assert_eq!(destination.network, remote_network);
        assert_eq!(&destination.mac_address[..], &remote_mac[..]);

        let decoded = apdu::decode_apdu(npdu.payload).unwrap();
        let Apdu::ConfirmedRequest(req) = decoded else {
            panic!("expected ConfirmedRequest");
        };
        assert!(!req.segmented);
        assert_eq!(req.max_apdu_length, 480);
        assert_eq!(&req.service_request[..], &service_data[..]);

        send_peer_routed_apdu(
            &router_transport,
            &client_mac,
            remote_network,
            &remote_mac,
            Apdu::SimpleAck(SimpleAck {
                invoke_id: req.invoke_id,
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            }),
        )
        .await
        .unwrap();
    };

    let (result, ()) = tokio::join!(request_future, router_future);

    assert!(result.unwrap().is_empty());

    client.stop().await.unwrap();
    router_transport.stop().await.unwrap();
}

#[tokio::test]
async fn client_max_apdu_length_reflects_sc_failover_transport_limit() {
    let (primary_client, primary_hub) = LoopbackWebSocket::pair();
    let (failover_client, failover_hub) = LoopbackWebSocket::pair();
    let primary_hub_vmac = [0x10; 6];
    let failover_hub_vmac = [0x20; 6];
    let sc_transport = ScTransport::new(primary_client, [0x01; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_heartbeat_interval_ms(5_000)
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 25,
            max_delay_ms: 25,
            max_retries: 1,
        })
        .with_failover(failover_client);

    let primary_task = tokio::spawn(async move {
        accept_sc_with_limits(&primary_hub, primary_hub_vmac, 1476, 1026).await;
        primary_hub
    });

    let mut client = BACnetClient::generic_builder()
        .transport(sc_transport)
        .apdu_timeout_ms(5000)
        .build()
        .await
        .unwrap();
    let primary_hub = primary_task.await.unwrap();
    assert_eq!(client.transport_max_apdu_length(), 1024);
    assert_eq!(client.max_apdu_length(), 1024);

    let failover_task = tokio::spawn(async move {
        accept_sc_with_limits(&failover_hub, failover_hub_vmac, 1200, 480).await;
        failover_hub
    });

    drop(primary_hub);
    let failover_hub = timeout(Duration::from_secs(2), failover_task)
        .await
        .expect("timed out waiting for failover handshake")
        .unwrap();

    wait_for_client_transport_max_apdu_length(&client, 478, Duration::from_secs(1)).await;
    assert_eq!(client.max_apdu_length(), 206);

    client.stop().await.unwrap();
    drop(failover_hub);
}

/// One ConfirmedRequest PDU as the peer saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SeenRequest {
    advertised_max_apdu: u16,
    npdu_len: usize,
    segmented: bool,
}

/// Send a `len`-octet WriteProperty from `client` to a peer it has no
/// device-table row for, ack every segment and the request, and return
/// each request PDU the peer saw.
async fn write_to_unknown_peer(
    client: &BACnetClient<LimitedTransport>,
    peer: &LoopbackTransport,
    peer_rx: &mut mpsc::Receiver<ReceivedNpdu>,
    len: usize,
) -> Vec<SeenRequest> {
    let (client_mac, peer_mac) = ([0x01], [0x02]);
    let service_data: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    let request = client.confirmed_request(
        &peer_mac,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        &service_data,
    );
    let peer_side = async {
        let mut seen = Vec::new();
        let mut received = Vec::new();
        loop {
            let frame = timeout(Duration::from_secs(3), peer_rx.recv())
                .await
                .expect("timed out waiting for the request")
                .expect("peer channel closed");
            let npdu = decode_npdu(Bytes::copy_from_slice(&frame.npdu)).unwrap();
            let Apdu::ConfirmedRequest(req) = apdu::decode_apdu(npdu.payload).unwrap() else {
                panic!("expected a ConfirmedRequest");
            };
            seen.push(SeenRequest {
                advertised_max_apdu: req.max_apdu_length,
                npdu_len: frame.npdu.len(),
                segmented: req.segmented,
            });
            received.extend_from_slice(&req.service_request);
            if req.segmented {
                let ack = Apdu::SegmentAck(SegmentAck {
                    negative_ack: false,
                    sent_by_server: true,
                    invoke_id: req.invoke_id,
                    sequence_number: req.sequence_number.unwrap(),
                    actual_window_size: 1,
                });
                send_peer_apdu(peer, &client_mac, ack).await.unwrap();
            }
            if !req.more_follows {
                let ack = Apdu::SimpleAck(SimpleAck {
                    invoke_id: req.invoke_id,
                    service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                });
                send_peer_apdu(peer, &client_mac, ack).await.unwrap();
                break;
            }
        }
        assert_eq!(received, service_data);
        seen
    };
    let (result, seen) = tokio::join!(request, peer_side);
    assert!(result.unwrap().is_empty());
    seen
}

/// A client on a transport whose limit changes while it runs.
async fn limited_client(
    configured_max_apdu: u16,
    start_limit: u16,
) -> (
    BACnetClient<LimitedTransport>,
    Arc<AtomicU16>,
    LoopbackTransport,
    mpsc::Receiver<ReceivedNpdu>,
) {
    let (client_inner, mut peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let (transport, limit) = LimitedTransport::new(client_inner, start_limit);
    let peer_rx = peer.start().await.unwrap();
    let client = BACnetClient::generic_builder()
        .transport(transport)
        .max_apdu_length(configured_max_apdu)
        .apdu_timeout_ms(5000)
        .build()
        .await
        .unwrap();
    (client, limit, peer, peer_rx)
}

/// #2086: a limit that grows after start (a router port or an SC hub
/// failover publishing a larger one) raises what the client advertises and
/// the segment size it sends, up to the configured maximum.
#[tokio::test(start_paused = true)]
async fn client_follows_a_rising_transport_limit_up_to_the_configured_maximum() {
    // (configured, raised limit, advertised and segment APDU size after)
    for (configured, raised, expected) in [(1476, 1024, 1024), (480, 1476, 480)] {
        let (mut client, limit, mut peer, mut peer_rx) = limited_client(configured, 206).await;
        assert_eq!(client.max_apdu_length(), 206);
        limit.store(raised, Ordering::Relaxed);
        assert_eq!(client.max_apdu_length(), expected);

        let seen = write_to_unknown_peer(&client, &peer, &mut peer_rx, 1400).await;
        let segment_payload = usize::from(expected) - 6;
        assert_eq!(seen.len(), 1400usize.div_ceil(segment_payload), "{seen:?}");
        // A full segment fills the APDU; the NPDU header adds two octets.
        assert_eq!(seen[0].npdu_len, usize::from(expected) + 2, "{seen:?}");
        for request in &seen {
            assert!(request.segmented, "{seen:?}");
            assert_eq!(request.advertised_max_apdu, expected, "{seen:?}");
        }
        client.stop().await.unwrap();
        peer.stop().await.unwrap();
    }
}

/// A limit that falls after start still lowers both, as before #2086.
#[tokio::test(start_paused = true)]
async fn client_follows_a_falling_transport_limit() {
    let (mut client, limit, mut peer, mut peer_rx) = limited_client(1476, 1476).await;
    assert_eq!(client.max_apdu_length(), 1476);
    let seen = write_to_unknown_peer(&client, &peer, &mut peer_rx, 900).await;
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].advertised_max_apdu, 1476);
    assert!(!seen[0].segmented);

    limit.store(478, Ordering::Relaxed);
    assert_eq!(client.max_apdu_length(), 206);
    let seen = write_to_unknown_peer(&client, &peer, &mut peer_rx, 900).await;
    // Segments fill the live limit; the advertised bucket is the one below it.
    assert_eq!(seen.len(), 900usize.div_ceil(478 - 6), "{seen:?}");
    assert_eq!(seen[0].npdu_len, 480, "{seen:?}");
    for request in &seen {
        assert!(request.segmented, "{seen:?}");
        assert_eq!(request.advertised_max_apdu, 206, "{seen:?}");
    }
    client.stop().await.unwrap();
    peer.stop().await.unwrap();
}
