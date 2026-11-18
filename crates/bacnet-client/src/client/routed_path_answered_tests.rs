//! A routed path after a request the device answered (#2651). The path is
//! free at once, however many frames the request sent. Inside what was the
//! hold window, a reason 4 fails only a request at least as large as the
//! answered request's largest frame. Every test runs on paused time at the
//! client's default timing, 6000 ms and 3 retries, where the old hold was
//! 24 s.

use std::sync::Arc;

use bacnet_encoding::apdu::{
    AbortPdu, Apdu, ComplexAck, ConfirmedRequest, ErrorPdu, RejectPdu, SegmentAck, SimpleAck,
};
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::TestTransport;
use bacnet_types::enums::{
    AbortReason, ConfirmedServiceChoice, ErrorClass, ErrorCode, RejectReason,
};
use bacnet_types::error::{Error, RemoteError};
use bytes::Bytes;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, sleep, timeout};

use super::routed_path_limit_tests::{
    DNET, SentFrame, confirmed_request, finish_segmented, harness, inject_apdu, inject_reason_4,
    inject_simple_ack, routed_request,
};
use super::{BACnetClient, ClientConfig};

const ROUTER: [u8; 1] = [2];
const DADR: [u8; 1] = [3];
/// One APDU timeout at the default.
const TIMEOUT: Duration = Duration::from_secs(6);
/// The old hold at the defaults: the timeout times four attempts.
pub(super) const WINDOW: Duration = Duration::from_secs(24);
/// 300 octets of service data: a 304-octet APDU, a 315-octet forwarded NPDU
/// (9 fixed octets of NPCI, a 1-octet DADR and a 1-octet SADR).
pub(super) const LARGE: usize = 300;
/// 220 octets of service data: a 235-octet forwarded NPDU, smaller than
/// [`LARGE`] and just above the 228-octet floor a reason 4 can teach
/// (#2665).
const SMALL: usize = 220;

/// One client and its routed path to [`DNET`] through [`ROUTER`].
pub(super) struct Path {
    pub(super) client: Arc<BACnetClient<TestTransport>>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    outbound: mpsc::UnboundedReceiver<SentFrame>,
}

impl Path {
    /// A client on `config` whose path carries `max_npdu`, or the
    /// conservative 228 octets with `None`.
    pub(super) async fn new(config: ClientConfig, max_npdu: Option<u16>) -> Self {
        let (transport, inbound, outbound) = harness(&[1], 1486);
        let client = Arc::new(BACnetClient::start(config, transport).await.unwrap());
        if let Some(max_npdu) = max_npdu {
            client
                .configure_routed_path_max_npdu(&ROUTER, DNET, max_npdu)
                .await
                .unwrap();
        }
        Self {
            client,
            inbound,
            outbound,
        }
    }

    pub(super) fn request(&self, octets: usize, fill: u8) -> JoinHandle<Result<Bytes, Error>> {
        routed_request(
            Arc::clone(&self.client),
            ROUTER.to_vec(),
            DNET,
            DADR.to_vec(),
            vec![fill; octets],
        )
    }

    /// The next frame the client sends, within twice the window: on paused
    /// time a frame that never comes fails the test instead of hanging it.
    pub(super) async fn next_frame(&mut self) -> ConfirmedRequest {
        let frame = timeout(2 * WINDOW, self.outbound.recv())
            .await
            .expect("no frame within twice the window")
            .unwrap();
        confirmed_request(frame, &ROUTER)
    }

    pub(super) async fn ack(&self, invoke_id: u8) {
        inject_simple_ack(&self.inbound, &ROUTER, DNET, &DADR, invoke_id).await;
    }

    pub(super) async fn reject(&self) {
        inject_reason_4(&self.inbound, &ROUTER, DNET).await;
    }

    /// A request of `octets` the device answers on its retry, one timeout
    /// after its first frame. Returns its invoke ID.
    pub(super) async fn answered_on_retry(&mut self, octets: usize) -> u8 {
        let task = self.request(octets, 0x41);
        let first = self.next_frame().await;
        let sent = Instant::now();
        let retry = self.next_frame().await;
        assert_eq!(sent.elapsed(), TIMEOUT);
        assert_eq!(retry.invoke_id, first.invoke_id);
        self.ack(retry.invoke_id).await;
        assert!(task.await.unwrap().unwrap().is_empty());
        retry.invoke_id
    }

    /// A request of `octets` sent now goes out after `wait`, in APDUs of at
    /// most `max_apdu` octets, and the device answers it, segment by segment
    /// when it is segmented. Returns its first frame.
    pub(super) async fn next_goes_out_after(
        &mut self,
        octets: usize,
        wait: Duration,
        max_apdu: u16,
    ) -> ConfirmedRequest {
        let from = Instant::now();
        let task = self.request(octets, 0x50);
        let frame = self.next_frame().await;
        assert_eq!(from.elapsed(), wait, "when the next request went out");
        if frame.segmented {
            finish_segmented(
                &self.inbound,
                &mut self.outbound,
                &ROUTER,
                DNET,
                &DADR,
                frame.clone(),
                max_apdu,
            )
            .await;
        } else {
            assert!(4 + frame.service_request.len() <= usize::from(max_apdu));
            self.ack(frame.invoke_id).await;
        }
        assert!(task.await.unwrap().unwrap().is_empty());
        frame
    }

    /// The device answers a segmented request, segment by segment, from its
    /// `first` frame.
    pub(super) async fn finish_segmented(&mut self, first: ConfirmedRequest, max_apdu: u16) {
        finish_segmented(
            &self.inbound,
            &mut self.outbound,
            &ROUTER,
            DNET,
            &DADR,
            first,
            max_apdu,
        )
        .await;
    }

    pub(super) async fn stop(self) {
        let mut client = Arc::try_unwrap(self.client).ok().unwrap();
        client.stop().await.unwrap();
    }
}

pub(super) fn too_long(result: Result<Bytes, Error>) -> bool {
    matches!(result, Err(Error::RoutedPathTooLong { dnet: DNET }))
}

pub(super) fn timed_out(result: Result<Bytes, Error>) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Abort { reason }))
        if reason == AbortReason::TSM_TIMEOUT)
}

/// The issue's probe: a request answered on its retry, one timeout after
/// its first frame, held its path for 24 s. The next request to that
/// network through that router now goes out at once.
#[tokio::test(start_paused = true)]
async fn a_request_answered_on_its_retry_frees_its_path_at_once() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    path.answered_on_retry(1).await;
    path.next_goes_out_after(1, Duration::ZERO, 217).await;
    path.stop().await;
}

/// A two-segment request the device answers with no retry sends two
/// frames, which held its path for 24 s as well. It frees it at once.
#[tokio::test(start_paused = true)]
async fn a_segmented_request_answered_without_a_retry_frees_its_path_at_once() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(LARGE, 0x41);
    // The conservative 228-octet path leaves 217 APDU octets: 211 of
    // service data per segment, so two segments.
    let first = path.next_frame().await;
    assert!(first.segmented && first.more_follows);
    assert_eq!(first.service_request.len(), 211);
    finish_segmented(
        &path.inbound,
        &mut path.outbound,
        &ROUTER,
        DNET,
        &DADR,
        first,
        217,
    )
    .await;
    assert!(task.await.unwrap().unwrap().is_empty());
    path.next_goes_out_after(1, Duration::ZERO, 217).await;
    path.stop().await;
}

/// A request nobody answered still holds its path for the whole window
/// after it timed out: a frame of it may yet draw a reason 4.
#[tokio::test(start_paused = true)]
async fn a_request_with_no_answer_holds_its_path_for_the_full_window() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let task = path.request(1, 0x41);
    for _ in 0..4 {
        path.next_frame().await;
    }
    assert!(timed_out(task.await.unwrap()));
    path.next_goes_out_after(1, WINDOW, 217).await;
    path.stop().await;
}

/// A request its caller cancels once its first frame is on the wire has no
/// answer: the path is held for the whole window, the request waiting
/// behind it goes out when the window ends, and the cancelled transaction
/// leaves no invoke ID leased.
#[tokio::test(start_paused = true)]
async fn a_request_cancelled_after_its_first_frame_holds_the_path_for_the_full_window() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let cancelled = path.request(1, 0x41);
    path.next_frame().await;
    let waiter = path.request(1, 0x42);
    sleep(Duration::from_millis(1)).await;
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());

    let from = Instant::now();
    let frame = path.next_frame().await;
    assert_eq!(from.elapsed(), WINDOW, "when the waiter went out");
    assert_eq!(frame.service_request.as_ref(), [0x42]);
    assert_eq!(
        path.client.tsm.lock().coordinated_active_count(),
        1,
        "only the waiter's invoke ID is leased"
    );
    path.ack(frame.invoke_id).await;
    assert!(waiter.await.unwrap().unwrap().is_empty());
    assert_eq!(path.client.tsm.lock().coordinated_active_count(), 0);
    path.stop().await;
}

/// A reason 4 that ends a request after its retry is the router's refusal,
/// not an answer, and either frame may draw another: the path is held for
/// the whole window, as before.
#[tokio::test(start_paused = true)]
async fn a_reason_4_after_a_retry_holds_the_path_for_the_full_window() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    let task = path.request(LARGE, 0x41);
    path.next_frame().await;
    path.next_frame().await;
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    path.next_goes_out_after(SMALL, WINDOW, 1476).await;
    path.stop().await;
}

/// A reason 4 for a request's only frame, on a path with no answered
/// frames on record, is that frame's among the client's confirmed requests
/// on the path: the path is free at once.
#[tokio::test(start_paused = true)]
async fn a_reason_4_for_the_only_frame_frees_the_path_at_once() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    let task = path.request(LARGE, 0x41);
    path.next_frame().await;
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    path.next_goes_out_after(SMALL, Duration::ZERO, 1476).await;
    path.stop().await;
}

/// Inside the window, a request at least as large as the answered one's
/// largest frame takes a reason 4: whichever frame it was for, this
/// request's cannot pass either. It fails, and its size is the bound.
#[tokio::test(start_paused = true)]
async fn inside_the_window_a_request_as_large_takes_the_reject_and_learns() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    path.answered_on_retry(LARGE).await;
    let task = path.request(LARGE, 0x42);
    assert!(!path.next_frame().await.segmented);
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    assert_eq!(path.client.too_long_reject_drops(), 0);

    // The reject could have been the answered request's, so the path is
    // held; then the 315-octet NPDU bounds the next one to 303 APDU octets.
    let first = path.next_goes_out_after(LARGE, WINDOW, 303).await;
    assert!(first.segmented);
    path.stop().await;
}

/// Inside the window, a smaller request drops a reason 4: it may be for the
/// answered request's larger frame, which says nothing of this one. The
/// drop is counted, the request completes on its own answer, and nothing
/// is learned.
#[tokio::test(start_paused = true)]
async fn inside_the_window_a_smaller_request_drops_the_reject_and_completes() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    path.answered_on_retry(LARGE).await;
    let task = path.request(SMALL, 0x42);
    let frame = path.next_frame().await;
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert!(!task.is_finished(), "the reject failed the smaller request");
    assert_eq!(path.client.too_long_reject_drops(), 1);
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());

    let next = path.next_goes_out_after(LARGE, Duration::ZERO, 1476).await;
    assert!(!next.segmented, "a dropped reject taught a bound");
    path.stop().await;
}

/// The record keeps the largest frame of every answered request in its
/// window: a smaller request answered on its retry after a larger one does
/// not shrink it, so a reject that may be the larger frame's still cannot
/// fail a request of the smaller size.
#[tokio::test(start_paused = true)]
async fn a_smaller_answered_request_does_not_shrink_the_record() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    path.answered_on_retry(LARGE).await;
    path.answered_on_retry(SMALL).await;
    let task = path.request(SMALL, 0x42);
    let frame = path.next_frame().await;
    path.reject().await;
    sleep(Duration::from_millis(10)).await;
    assert!(
        !task.is_finished(),
        "a reject that may be the larger frame's failed the smaller request"
    );
    assert_eq!(path.client.too_long_reject_drops(), 1);
    path.ack(frame.invoke_id).await;
    assert!(task.await.unwrap().unwrap().is_empty());
    path.stop().await;
}

/// After the window the path has no history: a reason 4 fails a smaller
/// request and teaches its bound, as on any path.
#[tokio::test(start_paused = true)]
async fn after_the_window_a_smaller_request_takes_the_reject() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    path.answered_on_retry(LARGE).await;
    sleep(WINDOW).await;
    let task = path.request(SMALL, 0x42);
    path.next_frame().await;
    path.reject().await;
    assert!(too_long(task.await.unwrap()));
    assert_eq!(path.client.too_long_reject_drops(), 0);

    // The 235-octet NPDU bounds the next request to 223 APDU octets.
    let first = path.next_goes_out_after(LARGE, Duration::ZERO, 223).await;
    assert!(first.segmented);
    path.stop().await;
}

/// The gate's cost: after the path shrinks below a request the device
/// answered, a smaller request that really is too long is not failed by
/// its rejects inside the window. Each is dropped and counted, and the
/// request ends by its own timeout, not as too long.
#[tokio::test(start_paused = true)]
async fn after_a_path_shrinks_a_smaller_request_too_long_ends_by_timeout() {
    let mut path = Path::new(ClientConfig::default(), Some(1497)).await;
    path.answered_on_retry(LARGE).await;
    let task = path.request(SMALL, 0x42);
    // Its four frames go out 0, 6, 12 and 18 s into the 24 s window.
    for _ in 0..4 {
        path.next_frame().await;
        path.reject().await;
    }
    assert!(timed_out(task.await.unwrap()));
    assert_eq!(path.client.too_long_reject_drops(), 4);
    path.stop().await;
}

/// A reason 4 that arrives after a segment window's smaller final segment
/// went out may be for a full segment sent before it: the bound learned is
/// the largest frame's, not the latest's.
#[tokio::test(start_paused = true)]
async fn a_reject_inside_a_segment_window_teaches_the_largest_frame() {
    let config = ClientConfig {
        proposed_window_size: 4,
        ..ClientConfig::default()
    };
    let mut path = Path::new(config, Some(300)).await;
    // Segments of 283, 283 and 34 octets: forwarded NPDUs of 300, 300, 51.
    let task = path.request(600, 0x41);
    let first = path.next_frame().await;
    assert_eq!(first.service_request.len(), 283);
    inject_apdu(
        &path.inbound,
        &ROUTER,
        Some((DNET, &DADR)),
        Apdu::SegmentAck(SegmentAck {
            negative_ack: false,
            sent_by_server: true,
            invoke_id: first.invoke_id,
            sequence_number: 0,
            actual_window_size: 2,
        }),
    )
    .await;
    assert_eq!(path.next_frame().await.service_request.len(), 283);
    let last = path.next_frame().await;
    assert!(!last.more_follows);
    assert_eq!(last.service_request.len(), 34);
    path.reject().await;
    assert!(too_long(task.await.unwrap()));

    // Bound by the 300-octet NPDU: 299, so 288 APDU octets, and a segment
    // of 282. The final segment's 51 octets would have left 40.
    let first = path.next_goes_out_after(LARGE, WINDOW, 288).await;
    assert_eq!(6 + first.service_request.len(), 288);
    path.stop().await;
}

/// A late duplicate of the answer to a request answered on its retry, of
/// each kind, finds no transaction and is dropped, as an idle client drops
/// it (Clause 5.4.4.1). The next request, out at once, has another invoke
/// ID from the round-robin pool, and completes only on its own answer.
#[tokio::test(start_paused = true)]
async fn a_late_duplicate_answer_cannot_answer_the_next_request() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    for kind in 0..5 {
        let earlier = path.answered_on_retry(1).await;
        let task = path.request(1, 0x42);
        let frame = path.next_frame().await;
        assert_ne!(frame.invoke_id, earlier);
        inject_apdu(
            &path.inbound,
            &ROUTER,
            Some((DNET, &DADR)),
            duplicate(kind, earlier),
        )
        .await;
        sleep(Duration::from_millis(10)).await;
        assert!(
            !task.is_finished(),
            "duplicate {kind} answered the next request"
        );
        path.ack(frame.invoke_id).await;
        assert!(task.await.unwrap().unwrap().is_empty());
    }
    path.stop().await;
}

/// One of the five answers a device gives, under `invoke_id`, for the
/// service the tests request.
fn duplicate(kind: u8, invoke_id: u8) -> Apdu {
    let service_choice = ConfirmedServiceChoice::WRITE_PROPERTY;
    match kind {
        0 => Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice,
        }),
        1 => Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice,
            service_ack: Bytes::from_static(&[0x0c]),
        }),
        2 => Apdu::Error(ErrorPdu {
            invoke_id,
            service_choice,
            error_class: ErrorClass::PROPERTY,
            error_code: ErrorCode::WRITE_ACCESS_DENIED,
            error_data: Bytes::new(),
        }),
        3 => Apdu::Reject(RejectPdu {
            invoke_id,
            reject_reason: RejectReason::OTHER,
        }),
        _ => Apdu::Abort(AbortPdu {
            sent_by_server: true,
            invoke_id,
            abort_reason: AbortReason::OTHER,
        }),
    }
}

/// Requests waiting behind a busy path get it in the order they came, the
/// moment the request ahead is answered on its retry; one cancelled while
/// it waited holds nothing.
#[tokio::test(start_paused = true)]
async fn waiters_take_a_freed_path_in_order_and_a_cancelled_one_holds_nothing() {
    let mut path = Path::new(ClientConfig::default(), None).await;
    let ahead = path.request(1, 0x41);
    let sent = path.next_frame().await;
    let mut waiters = Vec::new();
    for fill in [0x42, 0x43, 0x44] {
        waiters.push(path.request(1, fill));
        sleep(Duration::from_millis(1)).await;
    }
    let cancelled = waiters.remove(1);
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());

    let retry = path.next_frame().await;
    assert_eq!(retry.invoke_id, sent.invoke_id);
    path.ack(retry.invoke_id).await;
    assert!(ahead.await.unwrap().unwrap().is_empty());
    for (waiter, fill) in waiters.into_iter().zip([0x42, 0x44]) {
        let freed = Instant::now();
        let frame = path.next_frame().await;
        assert_eq!(freed.elapsed(), Duration::ZERO);
        assert_eq!(frame.service_request.as_ref(), [fill]);
        path.ack(frame.invoke_id).await;
        assert!(waiter.await.unwrap().unwrap().is_empty());
    }
    path.stop().await;
}
