use super::*;
use crate::server::test_transport::{
    SendLog, SendMode, TestTransport, TestTransportHandle, BIP_LOCAL_MAC,
};
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::TransportProvenance;
use bytes::Bytes;
use tokio::sync::{mpsc, watch};

/// Records unicasts and ignores broadcasts, from a B/IP-shaped local MAC.
fn recording_transport() -> (TestTransport, SendLog) {
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast(SendMode::Ignore)
        .build();
    let sent = transport.sent();
    (transport, sent)
}

fn recording_network() -> (Arc<NetworkLayer<TestTransport>>, SendLog) {
    let (transport, sent) = recording_transport();
    (Arc::new(NetworkLayer::new(transport)), sent)
}

/// [`recording_network`] whose first send is held until released.
fn blocking_first_send_network() -> (
    Arc<NetworkLayer<TestTransport>>,
    SendLog,
    TestTransportHandle,
) {
    let (transport, sent) = recording_transport();
    let first_send = transport.handle();
    first_send.block_next_send();
    (Arc::new(NetworkLayer::new(transport)), sent, first_send)
}

fn test_mac(byte: u8) -> MacAddr {
    MacAddr::from_slice(&[127, 0, 0, byte, 0xBA, 0xC0])
}

fn spawn_segmented_complex_ack(
    network: Arc<NetworkLayer<TestTransport>>,
    seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    source_mac: MacAddr,
    invoke_id: u8,
    service_ack_data: Vec<u8>,
) -> JoinHandle<()> {
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    spawn_segmented_complex_ack_from_network(
        network,
        seg_ack_senders,
        seg_send_permits,
        source_mac,
        None,
        invoke_id,
        service_ack_data,
    )
}

fn spawn_segmented_complex_ack_from_network(
    network: Arc<NetworkLayer<TestTransport>>,
    seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    seg_send_permits: Arc<Semaphore>,
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    invoke_id: u8,
    service_ack_data: Vec<u8>,
) -> JoinHandle<()> {
    spawn_segmented_complex_ack_from_network_with_options(
        network,
        seg_ack_senders,
        seg_send_permits,
        SegmentedSendTestRequest {
            source_mac,
            source_network,
            invoke_id,
            service_ack_data,
            options: SegmentedSendOptions::default(),
        },
    )
}

struct SegmentedSendTestRequest {
    source_mac: MacAddr,
    source_network: Option<NpduAddress>,
    invoke_id: u8,
    service_ack_data: Vec<u8>,
    options: SegmentedSendOptions,
}

fn spawn_segmented_complex_ack_from_network_with_options(
    network: Arc<NetworkLayer<TestTransport>>,
    seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    seg_send_permits: Arc<Semaphore>,
    request: SegmentedSendTestRequest,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        BACnetServer::<TestTransport>::send_segmented_complex_ack_with_options(
            SegmentedSendResources {
                network: &network,
                seg_ack_senders: &seg_ack_senders,
                seg_send_permits: &seg_send_permits,
            },
            ResponseTarget {
                source_mac: request.source_mac.as_slice(),
                source_network: request.source_network.as_ref(),
                route: &bacnet_network::response_route::ResponseRoute::unverified(),
            },
            ComplexAckParams {
                invoke_id: request.invoke_id,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
                client_max_apdu: 50,
                client_max_segments: None,
            },
            &request.service_ack_data,
            request.options,
            None,
        )
        .await;
    })
}

fn spawn_segmented_complex_ack_with_options(
    network: Arc<NetworkLayer<TestTransport>>,
    seg_ack_senders: Arc<segmented_send::SegmentedSendRegistry>,
    source_mac: MacAddr,
    invoke_id: u8,
    service_ack_data: Vec<u8>,
    options: SegmentedSendOptions,
) -> JoinHandle<()> {
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    spawn_segmented_complex_ack_from_network_with_options(
        network,
        seg_ack_senders,
        seg_send_permits,
        SegmentedSendTestRequest {
            source_mac,
            source_network: None,
            invoke_id,
            service_ack_data,
            options,
        },
    )
}

async fn wait_for_sent_len(sent: &SendLog, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), sent.wait_for_len(expected))
        .await
        .expect("timed out waiting for segmented response frame");
}

async fn send_segment_ack(
    seg_ack_senders: &Arc<segmented_send::SegmentedSendRegistry>,
    key: &SegKey,
    ack: SegmentAckPdu,
) {
    let handle = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(handle) = seg_ack_senders.lock().get(key).cloned() {
                return handle;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("timed out waiting for SegmentAck sender");

    handle
        .segment_ack_tx
        .send(ack)
        .await
        .expect("segmented response task should still be waiting for SegmentAck");
}

async fn dispatch_test_apdu(
    network: &Arc<NetworkLayer<TestTransport>>,
    seg_ack_senders: &Arc<segmented_send::SegmentedSendRegistry>,
    source_mac: &MacAddr,
    apdu: Apdu,
) {
    dispatch_test_apdu_from_network(network, seg_ack_senders, source_mac, None, apdu).await;
}

async fn dispatch_test_apdu_from_network(
    network: &Arc<NetworkLayer<TestTransport>>,
    seg_ack_senders: &Arc<segmented_send::SegmentedSendRegistry>,
    source_mac: &MacAddr,
    source_network: Option<NpduAddress>,
    apdu: Apdu,
) {
    BACnetServer::<TestTransport>::dispatch(
        &DispatchContext::for_test(RequestServices {
            seg_ack_senders: Arc::clone(seg_ack_senders),
            cov_in_flight: Arc::new(Semaphore::new(255)),
            ..RequestServices::for_test(Arc::clone(network), ServerConfig::default())
        }),
        source_mac.as_slice(),
        apdu,
        bacnet_network::layer::ReceivedApdu {
            direct_response: None,
            apdu: Bytes::new(),
            source_mac: source_mac.clone(),
            ingress_network: None,
            source_network,
            link_layer_group: false,
            is_group: false,
            global_broadcast: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        },
    )
    .await;
}

fn decoded_sent_apdu(sent: &SendLog, index: usize) -> Apdu {
    sent.frame(index).apdu()
}

fn sent_npdu_destination(sent: &SendLog, index: usize) -> Option<NpduAddress> {
    sent.frame(index).decode_npdu().destination
}

fn sent_link_destination(sent: &SendLog, index: usize) -> MacAddr {
    sent.frame(index).mac
}

fn sent_expecting_reply(sent: &SendLog, index: usize) -> bool {
    sent.frame(index).decode_npdu().expecting_reply
}

fn complex_ack_sequence(sent: &SendLog, index: usize) -> u8 {
    match decoded_sent_apdu(sent, index) {
        Apdu::ComplexAck(ack) => {
            assert!(ack.segmented);
            assert_eq!(
                ack.service_choice,
                ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE
            );
            ack.sequence_number
                .expect("segmented ComplexAck should carry a sequence number")
        }
        other => panic!("expected segmented ComplexAck, got {other:?}"),
    }
}

fn abort_reason(sent: &SendLog, index: usize) -> AbortReason {
    match decoded_sent_apdu(sent, index) {
        Apdu::Abort(abort) => abort.abort_reason,
        other => panic!("expected Abort, got {other:?}"),
    }
}

fn sent_count(sent: &SendLog) -> usize {
    sent.len()
}

fn segment_ack(invoke_id: u8, negative_ack: bool, sequence_number: u8) -> SegmentAckPdu {
    SegmentAckPdu {
        negative_ack,
        sent_by_server: false,
        invoke_id,
        sequence_number,
        actual_window_size: 1,
    }
}

fn server_segment_ack(invoke_id: u8, negative_ack: bool, sequence_number: u8) -> SegmentAckPdu {
    SegmentAckPdu {
        negative_ack,
        sent_by_server: true,
        invoke_id,
        sequence_number,
        actual_window_size: 1,
    }
}

fn routed_address(network: u16, byte: u8) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(&[byte, byte.wrapping_add(1), byte.wrapping_add(2)]),
    }
}

fn fake_segmented_send_handle(
    capacity: usize,
    total_segments: usize,
    current_sequence: u16,
) -> (
    Arc<SegmentedSendHandle>,
    mpsc::Receiver<SegmentAckPdu>,
    watch::Receiver<Option<SegmentedSendControlEvent>>,
) {
    let (segment_ack_tx, segment_ack_rx) = mpsc::channel(capacity);
    let (control_tx, control_rx) = watch::channel(None);
    let handle = Arc::new(SegmentedSendHandle::new(
        segment_ack_tx,
        control_tx,
        total_segments,
    ));
    handle
        .current_sequence
        .store(current_sequence, Ordering::Release);
    (handle, segment_ack_rx, control_rx)
}

mod ack_window;
mod control_limits;
mod duplicate_window;
mod get_event_information;
mod request_byte_budget;
mod request_byte_budget_failures;
mod request_payload_detachment;
mod request_peer_quota;
mod request_peer_quota_state;
mod request_progress;
mod request_progress_expiry;
mod request_reassembly;
mod request_receive_timeout;
mod routing_overlap;

mod segment_timeout_config;
