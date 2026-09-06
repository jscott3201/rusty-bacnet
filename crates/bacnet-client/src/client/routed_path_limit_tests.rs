use std::sync::Arc;

use bacnet_encoding::apdu::{self, encode_apdu, Apdu, SegmentAck, SimpleAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::port::{ReceivedNpdu, TransportPort};
use bacnet_types::enums::{ConfirmedServiceChoice, NetworkMessageType, RejectMessageReason};
use bacnet_types::error::Error;
use bacnet_types::MacAddr;
use bytes::{Bytes, BytesMut};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration};

use super::{routed_path_limits::MAX_ROUTED_PATH_ENTRIES, BACnetClient, ClientConfig};

const DNET: u16 = 100;

struct SentFrame {
    npdu: Bytes,
    destination: MacAddr,
}

struct CaptureTransport {
    local_mac: MacAddr,
    inbound_rx: Option<mpsc::Receiver<ReceivedNpdu>>,
    outbound_tx: mpsc::UnboundedSender<SentFrame>,
    max_apdu: u16,
}

impl TransportPort for CaptureTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inbound_rx
            .take()
            .ok_or_else(|| Error::Encoding("capture transport already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.outbound_tx
            .send(SentFrame {
                npdu: Bytes::copy_from_slice(npdu),
                destination: MacAddr::from_slice(mac),
            })
            .map_err(|_| Error::Encoding("capture observer closed".into()))
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &[]).await
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }

    fn max_apdu_length(&self) -> u16 {
        self.max_apdu
    }
}

fn harness(
    local_mac: &[u8],
    max_apdu: u16,
) -> (
    CaptureTransport,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::UnboundedReceiver<SentFrame>,
) {
    let (inbound_tx, inbound_rx) = mpsc::channel(64);
    let (outbound_tx, outbound_rx) = mpsc::unbounded_channel();
    (
        CaptureTransport {
            local_mac: MacAddr::from_slice(local_mac),
            inbound_rx: Some(inbound_rx),
            outbound_tx,
            max_apdu,
        },
        inbound_tx,
        outbound_rx,
    )
}

fn routed_request(
    client: Arc<BACnetClient<CaptureTransport>>,
    router: Vec<u8>,
    dnet: u16,
    dadr: Vec<u8>,
    service_data: Vec<u8>,
) -> JoinHandle<Result<Bytes, Error>> {
    tokio::spawn(async move {
        client
            .confirmed_request_routed(
                &router,
                dnet,
                &dadr,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &service_data,
            )
            .await
    })
}

fn confirmed_request(frame: SentFrame, expected_router: &[u8]) -> apdu::ConfirmedRequest {
    assert_eq!(frame.destination.as_slice(), expected_router);
    let npdu = decode_npdu(frame.npdu).unwrap();
    let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
        panic!("expected ConfirmedRequest")
    };
    request
}

async fn inject_apdu(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    immediate_source: &[u8],
    routed_source: Option<(u16, &[u8])>,
    apdu: Apdu,
) {
    let mut apdu_buf = BytesMut::new();
    encode_apdu(&mut apdu_buf, &apdu).unwrap();
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            source: routed_source.map(|(network, mac)| NpduAddress {
                network,
                mac_address: MacAddr::from_slice(mac),
            }),
            payload: apdu_buf.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    inbound
        .send(ReceivedNpdu {
            npdu: npdu_buf.freeze(),
            source_mac: MacAddr::from_slice(immediate_source),
            link_layer_group: false,
            data_attributes: Vec::new(),
            reply_tx: None,
        })
        .await
        .unwrap();
}

async fn inject_control(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    immediate_source: &[u8],
    message_type: u8,
    payload: &[u8],
) {
    let mut npdu_buf = BytesMut::new();
    encode_npdu(
        &mut npdu_buf,
        &Npdu {
            is_network_message: true,
            message_type: Some(message_type),
            payload: Bytes::copy_from_slice(payload),
            ..Npdu::default()
        },
    )
    .unwrap();
    inbound
        .send(ReceivedNpdu {
            npdu: npdu_buf.freeze(),
            source_mac: MacAddr::from_slice(immediate_source),
            link_layer_group: false,
            data_attributes: Vec::new(),
            reply_tx: None,
        })
        .await
        .unwrap();
}

async fn inject_reason_4(inbound: &mpsc::Sender<ReceivedNpdu>, immediate_source: &[u8], dnet: u16) {
    inject_control(
        inbound,
        immediate_source,
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw(),
        &[
            RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
            (dnet >> 8) as u8,
            dnet as u8,
        ],
    )
    .await;
}

async fn inject_simple_ack(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    router: &[u8],
    dnet: u16,
    dadr: &[u8],
    invoke_id: u8,
) {
    inject_apdu(
        inbound,
        router,
        Some((dnet, dadr)),
        Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
}

async fn finish_segmented(
    inbound: &mpsc::Sender<ReceivedNpdu>,
    outbound: &mut mpsc::UnboundedReceiver<SentFrame>,
    router: &[u8],
    dnet: u16,
    dadr: &[u8],
    first: apdu::ConfirmedRequest,
    max_apdu: u16,
) {
    let mut request = first;
    loop {
        assert!(request.segmented);
        assert!(6 + request.service_request.len() <= usize::from(max_apdu));
        let seq = request.sequence_number.unwrap();
        inject_apdu(
            inbound,
            router,
            Some((dnet, dadr)),
            Apdu::SegmentAck(SegmentAck {
                negative_ack: false,
                sent_by_server: true,
                invoke_id: request.invoke_id,
                sequence_number: seq,
                actual_window_size: 1,
            }),
        )
        .await;
        if !request.more_follows {
            inject_simple_ack(inbound, router, dnet, dadr, request.invoke_id).await;
            return;
        }
        request = confirmed_request(
            timeout(Duration::from_secs(1), outbound.recv())
                .await
                .unwrap()
                .unwrap(),
            router,
        );
    }
}

#[tokio::test]
async fn routed_sizing_uses_conservative_and_configured_forwarded_envelopes() {
    let local = vec![1, 2, 3, 4, 5, 6];
    let router = vec![11, 12, 13, 14, 15, 16];
    let dadr = vec![21, 22, 23, 24, 25, 26];
    let (transport, inbound, mut outbound) = harness(&local, 1490);
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );

    // 228 NPDU - (9 + 6-byte DADR + 6-byte forwarded source) = 207 APDU.
    let first_task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x55; 204],
    );
    let first = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(first.segmented);
    finish_segmented(&inbound, &mut outbound, &router, DNET, &dadr, first, 207).await;
    assert!(first_task.await.unwrap().unwrap().is_empty());

    client
        .configure_routed_path_max_npdu(&router, DNET, 1497)
        .await
        .unwrap();
    let configured_task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x66; 1472],
    );
    let configured = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(!configured.segmented);
    assert_eq!(4 + configured.service_request.len(), 1476);
    inject_simple_ack(&inbound, &router, DNET, &dadr, configured.invoke_id).await;
    assert!(configured_task.await.unwrap().unwrap().is_empty());

    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn subminimum_path_cap_fails_before_registration_or_emission() {
    let router = vec![2; 6];
    let dadr = vec![3; 6];
    let (transport, _inbound, mut outbound) = harness(&[1; 6], 1490);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();
    client
        .configure_routed_path_max_npdu(&router, DNET, 70)
        .await
        .unwrap();

    let result = client
        .confirmed_request_routed(
            &router,
            DNET,
            &dadr,
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0x0c],
        )
        .await;
    assert!(matches!(
        result,
        Err(Error::RoutedPathTooLong { dnet: DNET })
    ));
    assert!(outbound.try_recv().is_err());
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn configured_capacity_exhaustion_fails_before_registration_or_emission() {
    let (transport, _inbound, mut outbound) = harness(&[1; 6], 1490);
    let mut client = BACnetClient::generic_builder()
        .transport(transport)
        .build()
        .await
        .unwrap();

    for path in 1..=MAX_ROUTED_PATH_ENTRIES {
        let path = u16::try_from(path).unwrap();
        client
            .configure_routed_path_max_npdu(&path.to_be_bytes(), path, 1497)
            .await
            .unwrap();
    }

    let result = client
        .confirmed_request_routed(
            &[0xff, 0xfe],
            u16::try_from(MAX_ROUTED_PATH_ENTRIES + 1).unwrap(),
            &[3],
            ConfirmedServiceChoice::READ_PROPERTY,
            &[0x0c],
        )
        .await;
    assert!(matches!(
        result,
        Err(Error::RoutedPathCapacityExceeded {
            capacity: MAX_ROUTED_PATH_ENTRIES
        })
    ));
    assert!(outbound.try_recv().is_err());
    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    assert_eq!(client.tsm.lock().await.coordinated_active_count(), 0);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn reason_4_completes_exact_caller_learns_bound_and_does_not_retry() {
    let router = vec![2];
    let dadr = vec![3];
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(
            ClientConfig {
                apdu_timeout_ms: 40,
                apdu_retries: 3,
                ..ClientConfig::default()
            },
            transport,
        )
        .await
        .unwrap(),
    );
    client
        .configure_routed_path_max_npdu(&router, DNET, 1497)
        .await
        .unwrap();

    let rejected_task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x44; 300],
    );
    let rejected = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(!rejected.segmented);
    assert_eq!(4 + rejected.service_request.len(), 304);
    inject_reason_4(&inbound, &router, DNET).await;
    assert!(matches!(
        timeout(Duration::from_millis(100), rejected_task)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::RoutedPathTooLong { dnet: DNET })
    ));
    assert!(timeout(Duration::from_millis(150), outbound.recv())
        .await
        .is_err());

    // The rejected 315-octet forwarded NPDU is an exclusive upper bound,
    // leaving 303 APDU octets for the next request.
    let learned_task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x45; 300],
    );
    let learned = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(learned.segmented);
    finish_segmented(&inbound, &mut outbound, &router, DNET, &dadr, learned, 303).await;
    assert!(learned_task.await.unwrap().unwrap().is_empty());

    // An explicit override deliberately resets learned negative evidence.
    client
        .configure_routed_path_max_npdu(&router, DNET, 1497)
        .await
        .unwrap();
    let reset_task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x46; 300],
    );
    let reset = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(!reset.segmented);
    inject_simple_ack(&inbound, &router, DNET, &dadr, reset.invoke_id).await;
    assert!(reset_task.await.unwrap().unwrap().is_empty());

    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn unmatched_controls_and_direct_requests_do_not_change_path_policy() {
    let router = vec![2];
    let wrong_router = vec![9];
    let dadr = vec![3];
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );
    client
        .configure_routed_path_max_npdu(&router, DNET, 1497)
        .await
        .unwrap();

    let task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x31; 300],
    );
    let request = confirmed_request(outbound.recv().await.unwrap(), &router);
    inject_reason_4(&inbound, &wrong_router, DNET).await;
    inject_reason_4(&inbound, &router, DNET + 1).await;
    inject_control(
        &inbound,
        &router,
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw(),
        &[RejectMessageReason::ROUTER_BUSY.to_raw(), 0, DNET as u8],
    )
    .await;
    inject_control(
        &inbound,
        &router,
        NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw(),
        &[RejectMessageReason::MESSAGE_TOO_LONG.to_raw(), 0],
    )
    .await;
    inject_control(
        &inbound,
        &router,
        NetworkMessageType::ROUTER_BUSY_TO_NETWORK.to_raw(),
        &[
            RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
            0,
            DNET as u8,
        ],
    )
    .await;
    sleep(Duration::from_millis(25)).await;
    assert!(!task.is_finished());
    inject_simple_ack(&inbound, &router, DNET, &dadr, request.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());

    // A stale matching control with no active path has no effect either.
    inject_reason_4(&inbound, &router, DNET).await;
    sleep(Duration::from_millis(25)).await;
    let next_task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x32; 300],
    );
    let next = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(!next.segmented, "unmatched controls must not learn a bound");
    inject_simple_ack(&inbound, &router, DNET, &dadr, next.invoke_id).await;
    assert!(next_task.await.unwrap().unwrap().is_empty());

    let direct_mac = vec![7];
    let direct_client = Arc::clone(&client);
    let direct_target = direct_mac.clone();
    let direct_task = tokio::spawn(async move {
        direct_client
            .confirmed_request(
                &direct_target,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                &[0x0c],
            )
            .await
    });
    let direct = confirmed_request(outbound.recv().await.unwrap(), &direct_mac);
    inject_reason_4(&inbound, &router, DNET).await;
    sleep(Duration::from_millis(25)).await;
    assert!(!direct_task.is_finished());
    inject_apdu(
        &inbound,
        &direct_mac,
        None,
        Apdu::SimpleAck(SimpleAck {
            invoke_id: direct.invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .await;
    assert!(direct_task.await.unwrap().unwrap().is_empty());

    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn same_path_serializes_while_a_second_router_remains_concurrent() {
    let router_a = vec![2];
    let router_b = vec![8];
    let dadr_a = vec![3];
    let dadr_b = vec![4];
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::generic_builder()
            .transport(transport)
            .apdu_timeout_ms(1000)
            .build()
            .await
            .unwrap(),
    );

    let first_task = routed_request(
        Arc::clone(&client),
        router_a.clone(),
        DNET,
        dadr_a.clone(),
        vec![1],
    );
    let first = confirmed_request(outbound.recv().await.unwrap(), &router_a);
    let blocked_task = routed_request(
        Arc::clone(&client),
        router_a.clone(),
        DNET,
        dadr_b.clone(),
        vec![2],
    );
    let concurrent_task = routed_request(
        Arc::clone(&client),
        router_b.clone(),
        DNET,
        dadr_b.clone(),
        vec![3],
    );

    let concurrent = confirmed_request(
        timeout(Duration::from_millis(100), outbound.recv())
            .await
            .expect("different router path should remain concurrent")
            .unwrap(),
        &router_b,
    );
    assert!(!blocked_task.is_finished());
    inject_simple_ack(&inbound, &router_b, DNET, &dadr_b, concurrent.invoke_id).await;
    assert!(concurrent_task.await.unwrap().unwrap().is_empty());

    inject_simple_ack(&inbound, &router_a, DNET, &dadr_a, first.invoke_id).await;
    assert!(first_task.await.unwrap().unwrap().is_empty());
    let blocked = confirmed_request(
        timeout(Duration::from_millis(100), outbound.recv())
            .await
            .expect("same-path waiter did not resume")
            .unwrap(),
        &router_a,
    );
    inject_simple_ack(&inbound, &router_a, DNET, &dadr_b, blocked.invoke_id).await;
    assert!(blocked_task.await.unwrap().unwrap().is_empty());

    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn cancelled_generation_quarantines_delayed_reason_4_before_next_send() {
    let router = vec![2];
    let dadr = vec![3];
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(
            ClientConfig {
                apdu_timeout_ms: 100,
                apdu_retries: 0,
                ..ClientConfig::default()
            },
            transport,
        )
        .await
        .unwrap(),
    );
    client
        .configure_routed_path_max_npdu(&router, DNET, 1497)
        .await
        .unwrap();

    let cancelled = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x61; 300],
    );
    let _cancelled_request = confirmed_request(outbound.recv().await.unwrap(), &router);
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());

    let next = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x62; 300],
    );
    assert!(
        timeout(Duration::from_millis(20), outbound.recv())
            .await
            .is_err(),
        "same-path request became active before the stale-control quarantine"
    );

    // This control belongs to the cancelled generation. It reaches network
    // ingress while the next request is waiting, and must be drained without
    // completing or teaching the next generation.
    inject_reason_4(&inbound, &router, DNET).await;
    sleep(Duration::from_millis(10)).await;
    assert!(!next.is_finished());

    let next_request = confirmed_request(
        timeout(Duration::from_millis(200), outbound.recv())
            .await
            .expect("quarantine did not release the next request")
            .unwrap(),
        &router,
    );
    assert!(
        !next_request.segmented,
        "delayed reason-4 must not poison the waiting generation"
    );
    inject_simple_ack(&inbound, &router, DNET, &dadr, next_request.invoke_id).await;
    assert!(next.await.unwrap().unwrap().is_empty());

    let proof = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x63; 300],
    );
    let proof_request = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(
        !proof_request.segmented,
        "stale control changed learned policy"
    );
    inject_simple_ack(&inbound, &router, DNET, &dadr, proof_request.invoke_id).await;
    assert!(proof.await.unwrap().unwrap().is_empty());

    assert_eq!(client.tsm.lock().await.pending_count(), 0);
    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn reason_4_during_segmented_send_prevents_window_retransmission() {
    let router = vec![2];
    let dadr = vec![3];
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(
            ClientConfig {
                apdu_timeout_ms: 40,
                apdu_retries: 3,
                ..ClientConfig::default()
            },
            transport,
        )
        .await
        .unwrap(),
    );
    client
        .configure_routed_path_max_npdu(&router, DNET, 128)
        .await
        .unwrap();

    let task = routed_request(
        Arc::clone(&client),
        router.clone(),
        DNET,
        dadr.clone(),
        vec![0x77; 300],
    );
    let first = confirmed_request(outbound.recv().await.unwrap(), &router);
    assert!(first.segmented);
    inject_reason_4(&inbound, &router, DNET).await;
    assert!(matches!(
        timeout(Duration::from_millis(100), task)
            .await
            .unwrap()
            .unwrap(),
        Err(Error::RoutedPathTooLong { dnet: DNET })
    ));
    assert!(timeout(Duration::from_millis(150), outbound.recv())
        .await
        .is_err());

    let mut client = Arc::try_unwrap(client).ok().unwrap();
    client.stop().await.unwrap();
}
