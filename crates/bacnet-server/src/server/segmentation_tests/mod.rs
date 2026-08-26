use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bytes::Bytes;
use std::sync::{Arc as StdArc, Mutex as StdMutex};
use tokio::sync::{mpsc, watch, Notify};

type SentFrames = StdArc<StdMutex<Vec<(Bytes, MacAddr)>>>;

#[derive(Clone, Default)]
struct RecordingTransport {
    sent_unicast: SentFrames,
    local_mac: Vec<u8>,
}

impl RecordingTransport {
    fn new(sent_unicast: SentFrames) -> Self {
        Self {
            sent_unicast,
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
        }
    }
}

impl TransportPort for RecordingTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sent_unicast
            .lock()
            .unwrap()
            .push((Bytes::copy_from_slice(npdu), MacAddr::from_slice(mac)));
        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

#[derive(Clone)]
struct BlockingSendTransport {
    sent_unicast: SentFrames,
    local_mac: Vec<u8>,
    first_send_started: Arc<Notify>,
    release_first_send: Arc<Notify>,
    block_first_send: Arc<AtomicBool>,
}

impl BlockingSendTransport {
    fn new(
        sent_unicast: SentFrames,
        first_send_started: Arc<Notify>,
        release_first_send: Arc<Notify>,
    ) -> Self {
        Self {
            sent_unicast,
            local_mac: vec![127, 0, 0, 1, 0xBA, 0xC0],
            first_send_started,
            release_first_send,
            block_first_send: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl TransportPort for BlockingSendTransport {
    async fn start(
        &mut self,
    ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.sent_unicast
            .lock()
            .unwrap()
            .push((Bytes::copy_from_slice(npdu), MacAddr::from_slice(mac)));

        if self.block_first_send.swap(false, Ordering::AcqRel) {
            self.first_send_started.notify_waiters();
            self.release_first_send.notified().await;
        }

        Ok(())
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        Ok(())
    }

    fn local_mac(&self) -> &[u8] {
        &self.local_mac
    }
}

fn test_mac(byte: u8) -> MacAddr {
    MacAddr::from_slice(&[127, 0, 0, byte, 0xBA, 0xC0])
}

fn spawn_segmented_complex_ack(
    network: Arc<NetworkLayer<RecordingTransport>>,
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
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
    network: Arc<NetworkLayer<RecordingTransport>>,
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
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
    network: Arc<NetworkLayer<RecordingTransport>>,
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
    seg_send_permits: Arc<Semaphore>,
    request: SegmentedSendTestRequest,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        BACnetServer::<RecordingTransport>::send_segmented_complex_ack_with_options(
            &network,
            &seg_ack_senders,
            &seg_send_permits,
            request.source_mac.as_slice(),
            request.source_network.as_ref(),
            request.invoke_id,
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            &request.service_ack_data,
            50,
            None,
            request.options,
        )
        .await;
    })
}

fn spawn_segmented_complex_ack_with_options(
    network: Arc<NetworkLayer<RecordingTransport>>,
    seg_ack_senders: Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
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

async fn wait_until_sent_len(sent: &SentFrames, expected: usize) {
    loop {
        if sent.lock().unwrap().len() >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for_sent_len(sent: &SentFrames, expected: usize) {
    tokio::time::timeout(Duration::from_secs(1), wait_until_sent_len(sent, expected))
        .await
        .expect("timed out waiting for segmented response frame");
}

async fn send_segment_ack(
    seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
    key: &SegKey,
    ack: SegmentAckPdu,
) {
    let handle = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(handle) = seg_ack_senders.lock().await.get(key).cloned() {
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

async fn dispatch_test_apdu<T: TransportPort + 'static>(
    network: &Arc<NetworkLayer<T>>,
    seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
    source_mac: &MacAddr,
    apdu: Apdu,
) {
    dispatch_test_apdu_from_network(network, seg_ack_senders, source_mac, None, apdu).await;
}

async fn dispatch_test_apdu_from_network<T: TransportPort + 'static>(
    network: &Arc<NetworkLayer<T>>,
    seg_ack_senders: &Arc<Mutex<HashMap<SegKey, Arc<SegmentedSendHandle>>>>,
    source_mac: &MacAddr,
    source_network: Option<NpduAddress>,
    apdu: Apdu,
) {
    let db = Arc::new(RwLock::new(ObjectDatabase::new()));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    let cov_in_flight = Arc::new(Semaphore::new(255));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let notification_transactions = NotificationTransactions::new();
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(None::<JoinHandle<()>>));
    let config = Arc::new(ServerConfig::default());

    BACnetServer::<T>::dispatch(
        &db,
        network,
        &cov_table,
        seg_ack_senders,
        &seg_send_permits,
        &cov_in_flight,
        &server_tsm,
        &notification_transactions,
        &comm_state,
        &dcc_timer,
        &config,
        source_mac.as_slice(),
        apdu,
        bacnet_network::layer::ReceivedApdu {
            apdu: Bytes::new(),
            source_mac: source_mac.clone(),
            source_network,
            link_layer_group: false,
            is_group: false,
            data_attributes: Vec::new(),
            reply_tx: None,
        },
    )
    .await;
}

fn decoded_sent_apdu(sent: &SentFrames, index: usize) -> Apdu {
    let npdu_bytes = {
        let sent = sent.lock().unwrap();
        sent[index].0.clone()
    };
    let npdu = decode_npdu(npdu_bytes).expect("sent frame should decode as NPDU");
    decode_apdu(npdu.payload).expect("sent NPDU payload should decode as APDU")
}

fn sent_npdu_destination(sent: &SentFrames, index: usize) -> Option<NpduAddress> {
    let npdu_bytes = {
        let sent = sent.lock().unwrap();
        sent[index].0.clone()
    };
    decode_npdu(npdu_bytes)
        .expect("sent frame should decode as NPDU")
        .destination
}

fn sent_link_destination(sent: &SentFrames, index: usize) -> MacAddr {
    sent.lock().unwrap()[index].1.clone()
}

fn sent_expecting_reply(sent: &SentFrames, index: usize) -> bool {
    let npdu_bytes = {
        let sent = sent.lock().unwrap();
        sent[index].0.clone()
    };
    decode_npdu(npdu_bytes)
        .expect("sent frame should decode as NPDU")
        .expecting_reply
}

fn complex_ack_sequence(sent: &SentFrames, index: usize) -> u8 {
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

fn abort_reason(sent: &SentFrames, index: usize) -> AbortReason {
    match decoded_sent_apdu(sent, index) {
        Apdu::Abort(abort) => abort.abort_reason,
        other => panic!("expected Abort, got {other:?}"),
    }
}

fn sent_count(sent: &SentFrames) -> usize {
    sent.lock().unwrap().len()
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
mod request_reassembly;
mod routing_overlap;
