//! A router's refusal of a routed confirmed request, the reject it sends
//! back (#2724), on the shared test transport. Reasons 0, 1, 5, 6 and any the
//! standard doesn't define end the request at once with
//! [`Error::NetworkRejected`]; a busy router (2) and reason 3 leave it to
//! its retries. A reject from another station, for another network, queued
//! before the request went out, or one an answered request's frames may
//! still draw, ends nothing. Every test runs on paused time at the client's
//! default timing, 6000 ms and 3 retries.

use std::sync::Arc;

use bacnet_encoding::apdu::{self, Apdu, ComplexAck};
use bacnet_encoding::npdu::{Npdu, decode_npdu};
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_npdu::{NetworkMessageType, RejectMessageReason};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SentFrame, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::data_link::DataLink;
use bacnet_types::enums::{ConfirmedServiceChoice, ErrorClass, ErrorCode, ObjectType};
use bacnet_types::error::{Error, NetworkReject};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, timeout};

use super::*;
use crate::client::routed_path_limit_tests::{inject_apdu, inject_control, inject_simple_ack};
use crate::client::{BACnetClient, ClientConfig, Station};
use crate::tsm::{Tsm, TsmConfig};

const ROUTER: [u8; 1] = [2];
const DADR: [u8; 1] = [3];
const DNET: u16 = 100;
/// One APDU timeout at the default.
const TIMEOUT: Duration = Duration::from_secs(6);
/// The hold after a generation that may still draw a reject: the timeout
/// times four attempts.
const HOLD: Duration = Duration::from_secs(24);

/// The reasons that end a request at once: other, no route, the withdrawn
/// security reason, an addressing error, and one the standard doesn't name.
fn refusing() -> [RejectMessageReason; 5] {
    [
        RejectMessageReason::OTHER,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        RejectMessageReason::from_raw(5),
        RejectMessageReason::ADDRESSING_ERROR,
        RejectMessageReason::from_raw(200),
    ]
}

/// A client on an MS/TP link, station 1, whose every send comes out of
/// `frames` as it is made.
struct Rig {
    client: Arc<BACnetClient<TestTransport>>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    frames: mpsc::UnboundedReceiver<SentFrame>,
}

impl Rig {
    async fn new() -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(16);
        let (frames_tx, frames) = mpsc::unbounded_channel();
        let transport = TestTransport::builder()
            .local_mac(&[1])
            .inbound(inbound_rx)
            .data_link(DataLink::Mstp)
            .egress_limit(480)
            .on_send(move |frame| {
                let _ = frames_tx.send(frame);
                async { Ok(()) }
            })
            .build();
        let client = BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap();
        Self {
            client: Arc::new(client),
            inbound: inbound_tx,
            frames,
        }
    }

    /// A WriteProperty-numbered request of `octets` to `to`.
    fn request(&self, to: Station, octets: usize) -> JoinHandle<Result<Bytes, Error>> {
        let client = Arc::clone(&self.client);
        tokio::spawn(async move {
            client
                .confirmed_request(
                    &to,
                    ConfirmedServiceChoice::WRITE_PROPERTY,
                    &vec![0x41; octets],
                )
                .await
        })
    }

    fn routed(&self, octets: usize) -> JoinHandle<Result<Bytes, Error>> {
        self.request(Station::routed(&ROUTER, DNET, &DADR), octets)
    }

    /// The next frame the client sends, and its confirmed request. On paused
    /// time a frame that never comes fails the test instead of hanging it.
    async fn next(&mut self) -> (SentFrame, apdu::ConfirmedRequest) {
        let frame = timeout(4 * HOLD, self.frames.recv())
            .await
            .expect("no frame within the wait")
            .unwrap();
        let npdu = decode_npdu(frame.npdu.clone()).unwrap();
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected a ConfirmedRequest")
        };
        (frame, request)
    }

    /// Nothing more was sent, nor is about to be.
    async fn quiet(&mut self) {
        tokio::task::yield_now().await;
        assert!(self.frames.try_recv().is_err(), "a frame went out");
    }

    async fn reject(&self, from: &[u8], reason: RejectMessageReason, dnet: u16) {
        let [high, low] = dnet.to_be_bytes();
        inject_control(
            &self.inbound,
            from,
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
            &[reason.to_raw(), high, low],
        )
        .await;
    }

    async fn ack(&self, invoke_id: u8) {
        inject_simple_ack(&self.inbound, &ROUTER, DNET, &DADR, invoke_id).await;
    }

    /// No transaction is left open, in the client or its coordinator.
    fn settled(&self) {
        assert_eq!(self.client.tsm.lock().pending_count(), 0);
        assert_eq!(self.client.tsm.lock().coordinated_active_count(), 0);
    }

    /// A request sent now goes out after `wait` and is answered.
    async fn next_goes_out_after(&mut self, wait: Duration) {
        let from = Instant::now();
        let task = self.routed(1);
        let (_, request) = self.next().await;
        assert_eq!(from.elapsed(), wait, "when the next request went out");
        self.ack(request.invoke_id).await;
        assert!(task.await.unwrap().unwrap().is_empty());
    }

    async fn stop(self) {
        let mut client = Arc::try_unwrap(self.client).ok().unwrap();
        client.stop().await.unwrap();
    }
}

fn rejected(result: Result<Bytes, Error>) -> NetworkReject {
    match result {
        Err(Error::NetworkRejected(reject)) => *reject,
        other => panic!("expected a network reject, got {other:?}"),
    }
}

/// Each refusing reason from the router the request went through, naming
/// its network, ends the request at once: no retry goes, its transaction
/// closes, and the error names the router in the link's form, the network
/// and the reason. One frame was sent, so the path is free at once.
#[tokio::test(start_paused = true)]
async fn each_refusing_reason_ends_the_request_at_once() {
    for reason in refusing() {
        let mut rig = Rig::new().await;
        let task = rig.routed(1);
        let (frame, _) = rig.next().await;
        assert_eq!(frame.mac.as_slice(), ROUTER);
        let sent = Instant::now();
        rig.reject(&ROUTER, reason, DNET).await;
        let reject = rejected(task.await.unwrap());
        assert_eq!(sent.elapsed(), Duration::ZERO, "reason {reason}");
        assert_eq!(reject.router.as_slice(), ROUTER);
        assert_eq!(reject.link, Some(DataLink::Mstp));
        assert_eq!((reject.network, reject.reason), (DNET, reason));
        assert!(reject.to_string().contains("router mstp:2"), "{reject}");
        // Its one frame was refused: the device never received the request.
        assert!(reject.single_frame);
        assert!(
            reject.to_string().ends_with("never reached the device"),
            "{reject}"
        );
        rig.quiet().await;
        rig.settled();
        rig.next_goes_out_after(Duration::ZERO).await;
        rig.stop().await;
    }
}

/// A busy router (reason 2) and reason 3, which answers a network message,
/// end nothing: the request retries one timeout after its first frame, at
/// the normal interval, and takes the device's answer.
#[tokio::test(start_paused = true)]
async fn a_busy_router_and_reason_3_leave_the_request_to_its_retries() {
    for reason in [
        RejectMessageReason::ROUTER_BUSY,
        RejectMessageReason::UNKNOWN_MESSAGE_TYPE,
    ] {
        let mut rig = Rig::new().await;
        let task = rig.routed(1);
        let (_, first) = rig.next().await;
        let sent = Instant::now();
        rig.reject(&ROUTER, reason, DNET).await;
        let (_, retry) = rig.next().await;
        assert_eq!(sent.elapsed(), TIMEOUT, "reason {reason}");
        assert_eq!(retry.invoke_id, first.invoke_id);
        rig.ack(retry.invoke_id).await;
        assert!(task.await.unwrap().unwrap().is_empty());
        rig.stop().await;
    }
}

/// A reject from a station that isn't the request's router, and one from
/// the router naming another network, find no request: the request runs on
/// to its retry and the device's answer.
#[tokio::test(start_paused = true)]
async fn a_reject_from_another_station_or_for_another_network_is_not_the_requests() {
    let mut rig = Rig::new().await;
    let task = rig.routed(1);
    let (_, first) = rig.next().await;
    let sent = Instant::now();
    rig.reject(&[9], RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
        .await;
    rig.reject(
        &ROUTER,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        DNET + 1,
    )
    .await;
    let (_, retry) = rig.next().await;
    assert_eq!(sent.elapsed(), TIMEOUT);
    assert_eq!(retry.invoke_id, first.invoke_id);
    rig.ack(retry.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    rig.stop().await;
}

/// A request answered on its retry leaves its frames able to draw a reject
/// for the hold's length. A refusing reject then ends nothing, as nothing
/// says it isn't for one of those frames: the next request runs on to its
/// retry and answer. Once the window has passed, one ends the request.
#[tokio::test(start_paused = true)]
async fn an_answered_requests_frames_keep_a_reject_from_ending_the_next_request() {
    let mut rig = Rig::new().await;
    for _ in 0..2 {
        let task = rig.routed(1);
        let (_, first) = rig.next().await;
        if rig
            .client
            .routed_path_limits
            .answered_window_open(&ROUTER, DNET)
        {
            // The second request, inside the first one's window.
            rig.reject(&ROUTER, RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
                .await;
        }
        let (_, retry) = rig.next().await;
        assert_eq!(retry.invoke_id, first.invoke_id);
        rig.ack(retry.invoke_id).await;
        assert!(task.await.unwrap().unwrap().is_empty());
    }
    // Each answered on its retry, so the second opened a window too.
    let answered = Instant::now();

    tokio::time::sleep_until(answered + HOLD).await;
    assert!(
        !rig.client
            .routed_path_limits
            .answered_window_open(&ROUTER, DNET)
    );
    let task = rig.routed(1);
    rig.next().await;
    rig.reject(&ROUTER, RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
        .await;
    assert_eq!(
        rejected(task.await.unwrap()).reason,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED
    );
    rig.stop().await;
}

/// A request refused after its retry sent two frames, and the first may
/// still draw a reject: the path is held for the horizon, as after a
/// reason 4 or no answer, and the next request goes out when it ends.
#[tokio::test(start_paused = true)]
async fn a_request_refused_after_several_frames_holds_its_path() {
    let mut rig = Rig::new().await;
    let task = rig.routed(1);
    rig.next().await;
    rig.next().await;
    rig.reject(&ROUTER, RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
        .await;
    // The first frame may have reached the device before the route went.
    let reject = rejected(task.await.unwrap());
    assert!(!reject.single_frame);
    assert!(
        reject.to_string().ends_with("may have reached the device"),
        "{reject}"
    );
    rig.settled();
    rig.next_goes_out_after(HOLD).await;
    rig.stop().await;
}

/// A segmented request refused after its first segment ends at once, sends
/// no further segment, and leaves its path free.
#[tokio::test(start_paused = true)]
async fn a_segmented_request_ends_on_a_refusing_reject() {
    let mut rig = Rig::new().await;
    // 300 octets over the 228-octet path: segments.
    let task = rig.routed(300);
    let (_, first) = rig.next().await;
    assert!(first.segmented && first.more_follows);
    rig.reject(&ROUTER, RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
        .await;
    rejected(task.await.unwrap());
    rig.quiet().await;
    rig.settled();
    rig.next_goes_out_after(Duration::ZERO).await;
    rig.stop().await;
}

/// A request whose answer is arriving in segments still holds its path, so
/// a refusing reject ends it there too: the router can no longer carry the
/// client's SegmentACKs. As for a request its timeout or its caller cuts off
/// mid-reassembly, the client sends the device nothing more, and the
/// dispatch task drops the reassembly.
#[tokio::test(start_paused = true)]
async fn a_request_taking_a_segmented_answer_ends_on_a_refusing_reject() {
    let mut rig = Rig::new().await;
    let client = Arc::clone(&rig.client);
    let task = tokio::spawn(async move {
        client
            .confirmed_request(
                &Station::routed(&ROUTER, DNET, &DADR),
                ConfirmedServiceChoice::READ_PROPERTY,
                &[0x0C, 0x00, 0x00, 0x00, 0x00, 0x19, 0x55],
            )
            .await
    });
    let (_, request) = rig.next().await;
    inject_apdu(
        &rig.inbound,
        &ROUTER,
        Some((DNET, &DADR)),
        Apdu::ComplexAck(ComplexAck {
            segmented: true,
            more_follows: true,
            invoke_id: request.invoke_id,
            sequence_number: Some(0),
            proposed_window_size: Some(1),
            service_choice: ConfirmedServiceChoice::READ_PROPERTY,
            service_ack: Bytes::from_static(&[0x0C; 16]),
        }),
    )
    .await;
    // The client acknowledges the segment through the router.
    let segment_ack = timeout(HOLD, rig.frames.recv()).await.unwrap().unwrap();
    assert_eq!(segment_ack.mac.as_slice(), ROUTER);
    assert!(matches!(segment_ack.apdu(), Apdu::SegmentAck(_)));
    rig.reject(&ROUTER, RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
        .await;
    rejected(task.await.unwrap());
    assert!(
        timeout(HOLD, rig.client.segmented_cleanup.wait_processed())
            .await
            .expect("the dispatch task was not asked to drop the reassembly"),
        "the reassembly of the answer was left behind"
    );
    rig.quiet().await;
    rig.settled();
    rig.stop().await;
}

/// A COV subscription goes through the same request path, and ends the
/// same way.
#[tokio::test(start_paused = true)]
async fn a_cov_subscription_ends_on_a_refusing_reject() {
    let mut rig = Rig::new().await;
    let client = Arc::clone(&rig.client);
    let task = tokio::spawn(async move {
        client
            .subscribe_cov(
                &Station::routed(&ROUTER, DNET, &DADR),
                1,
                ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 0).unwrap(),
                false,
                Some(60),
            )
            .await
    });
    let (_, request) = rig.next().await;
    assert_eq!(
        request.service_choice,
        ConfirmedServiceChoice::SUBSCRIBE_COV
    );
    rig.reject(&ROUTER, RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
        .await;
    assert!(matches!(
        task.await.unwrap(),
        Err(Error::NetworkRejected(reject)) if reject.network == DNET
    ));
    rig.stop().await;
}

/// A request sent by one broadcast on this link takes no routed path, so
/// no reject is tied to it: any router may hear the broadcast, and one
/// router's refusal says nothing of another's. It runs on to its retry and
/// the answer a router relays.
#[tokio::test(start_paused = true)]
async fn a_request_by_local_broadcast_takes_no_reject() {
    let mut rig = Rig::new().await;
    let task = rig.request(Station::routed_via_local_broadcast(DNET, &DADR), 1);
    let (frame, first) = rig.next().await;
    assert!(frame.broadcast);
    let sent = Instant::now();
    rig.reject(&ROUTER, RejectMessageReason::NOT_DIRECTLY_CONNECTED, DNET)
        .await;
    let (_, retry) = rig.next().await;
    assert_eq!(sent.elapsed(), TIMEOUT);
    assert_eq!(retry.invoke_id, first.invoke_id);
    rig.ack(retry.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    rig.stop().await;
}

/// An audited request's record says how a router refused it: the
/// COMMUNICATION code Clause 18 gives the reason, and the request counts
/// as attempted.
#[test]
fn an_audit_record_carries_the_reasons_code() {
    use crate::client::source_audit_targets::{attempted, audit_result};
    let error = Error::NetworkRejected(Box::new(NetworkReject::new(
        bacnet_npdu::Mac::from_array(ROUTER),
        None,
        DNET,
        RejectMessageReason::NOT_DIRECTLY_CONNECTED,
    )));
    assert_eq!(
        audit_result(&error),
        (ErrorClass::COMMUNICATION, ErrorCode::NOT_ROUTER_TO_DNET)
    );
    assert!(attempted(&Err(error)));
}

fn control(reason: RejectMessageReason, ingress_sequence: u64) -> ReceivedNetworkControl {
    let [high, low] = DNET.to_be_bytes();
    ReceivedNetworkControl {
        npdu: Npdu {
            message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK),
            payload: Bytes::from(vec![reason.to_raw(), high, low]),
            ..Npdu::default()
        },
        source_mac: MacAddr::from_slice(&ROUTER),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        ingress_sequence,
    }
}

/// A refusing reject the network layer decoded before the request was
/// installed on its path, at or below its floor, is not its own; one after
/// it ends the request.
#[tokio::test]
async fn a_reject_queued_before_the_request_went_out_is_not_its_own() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(2, HOLD));
    let target = ConfirmedTarget::Routed {
        router_mac: &ROUTER,
        dest_network: DNET,
        dest_mac: &DADR,
        one_station: false,
    };
    let lease = limits.acquire(&ROUTER, DNET).await.unwrap();
    let tsm_mac = target.transaction_peer().tsm_mac;
    let mut tsm = Tsm::new(TsmConfig::default());
    let mut registration = tsm.register_transaction_with_progress(
        tsm_mac.clone(),
        9,
        ConfirmedServiceChoice::WRITE_PROPERTY,
    );
    let tsm = Arc::new(SyncMutex::new(tsm));
    limits.install_active(target, &tsm_mac, 9, registration.owner.clone(), 5, 5);
    assert!(limits.authorize_attempt(target, &tsm_mac, 9, &registration.owner, 10));

    let reason = RejectMessageReason::NOT_DIRECTLY_CONNECTED;
    limits
        .handle_network_control(&tsm, control(reason, 5), Some(DataLink::Mstp))
        .await;
    assert!(
        tsm.lock()
            .owner_is_current(&tsm_mac, 9, &registration.owner)
    );
    assert!(registration.response.try_recv().is_err());

    limits
        .handle_network_control(&tsm, control(reason, 6), Some(DataLink::Mstp))
        .await;
    let TsmResponse::Failed(error) = registration.response.await.unwrap() else {
        panic!("expected a network reject")
    };
    let Error::NetworkRejected(reject) = *error else {
        panic!("expected a network reject")
    };
    assert_eq!(
        *reject,
        NetworkReject::new(
            bacnet_npdu::Mac::from_array(ROUTER),
            Some(DataLink::Mstp),
            DNET,
            reason
        )
        .after_one_frame()
    );
    drop(lease);
}

impl RoutedPathLimits {
    /// Whether an answered generation's frames on the path may still draw
    /// a reject.
    fn answered_window_open(&self, router_mac: &[u8], dnet: u16) -> bool {
        self.state()
            .entries
            .get(&RoutedPathKey::new(router_mac, dnet))
            .is_some_and(|entry| entry.answered_now().is_some())
    }
}

#[path = "routed_path_reject_forms_tests.rs"]
mod forms;
