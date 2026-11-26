//! The segmented sender's first window (#2742): segment zero goes alone and
//! its answer grants the window; a peer that leaves it unanswered for a
//! segment timeout is remembered and gets the whole first window at once
//! from then on. Paused time, on the shared test transport.

use std::sync::Arc;

use bacnet_encoding::apdu::{self, Apdu, ConfirmedRequest, SegmentAck, SimpleAck};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_transport::test_transport::{SentFrame, TestTransport};
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice};
use bacnet_types::error::{Error, RemoteError};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Duration, Instant, timeout};

use super::super::routed_path_limit_tests::inject_apdu;
use super::super::{BACnetClient, ClientConfig};

const PEER: [u8; 1] = [5];

/// The segment timeout: the APDU timeout, as it is by default.
const TSEG: Duration = Duration::from_secs(1);

/// Six segments of 44 octets or fewer at a 50-octet APDU.
const SIX_SEGMENTS: usize = 44 * 5 + 10;

struct Rig {
    client: Arc<BACnetClient<TestTransport>>,
    inbound: mpsc::Sender<ReceivedNpdu>,
    frames: mpsc::UnboundedReceiver<SentFrame>,
}

impl Rig {
    async fn new(window: u8, retries: u8) -> Self {
        let config = ClientConfig {
            apdu_timeout_ms: TSEG.as_millis() as u64,
            apdu_retries: retries,
            max_apdu_length: 50,
            proposed_window_size: window,
            ..ClientConfig::default()
        };
        let (inbound, inbound_rx) = mpsc::channel(16);
        let (frames_tx, frames) = mpsc::unbounded_channel();
        let transport = TestTransport::builder()
            .local_mac(&[1])
            .inbound(inbound_rx)
            .on_send(move |frame| {
                let _ = frames_tx.send(frame);
                async { Ok(()) }
            })
            .build();
        let client = BACnetClient::start(config, transport).await.unwrap();
        Self {
            client: Arc::new(client),
            inbound,
            frames,
        }
    }

    fn request(&self, len: usize) -> JoinHandle<Result<bytes::Bytes, Error>> {
        let client = Arc::clone(&self.client);
        let data: Vec<u8> = (0..len).map(|i| i as u8).collect();
        tokio::spawn(async move {
            client
                .confirmed_request(&PEER, ConfirmedServiceChoice::WRITE_PROPERTY, &data)
                .await
        })
    }

    async fn segment(&mut self) -> ConfirmedRequest {
        let frame = timeout(Duration::from_secs(60), self.frames.recv())
            .await
            .expect("no segment")
            .unwrap();
        let npdu = decode_npdu(frame.npdu).unwrap();
        let Apdu::ConfirmedRequest(request) = apdu::decode_apdu(npdu.payload).unwrap() else {
            panic!("expected a segment")
        };
        assert!(request.segmented);
        request
    }

    /// The next segments, their sequence numbers.
    async fn segments(&mut self, count: usize) -> Vec<u8> {
        self.window(count).await.1
    }

    /// The next segments: their invoke ID and sequence numbers.
    async fn window(&mut self, count: usize) -> (u8, Vec<u8>) {
        let mut invoke_id = 0;
        let mut seqs = Vec::new();
        for _ in 0..count {
            let segment = self.segment().await;
            invoke_id = segment.invoke_id;
            seqs.push(segment.sequence_number.unwrap());
        }
        (invoke_id, seqs)
    }

    /// Nothing goes out for a while (well inside a segment timeout).
    async fn quiet(&mut self) {
        let sent = timeout(Duration::from_millis(10), self.frames.recv()).await;
        assert!(sent.is_err(), "expected nothing, got a frame");
    }

    async fn ack(&self, invoke_id: u8, seq: u8, window: u8, negative: bool) {
        let ack = SegmentAck {
            negative_ack: negative,
            sent_by_server: true,
            invoke_id,
            sequence_number: seq,
            actual_window_size: window,
        };
        inject_apdu(&self.inbound, &PEER, None, Apdu::SegmentAck(ack)).await;
    }

    /// A Segment-ACK written by hand, so its window can be one the encoder
    /// refuses.
    async fn raw_ack(&self, invoke_id: u8, seq: u8, window: u8) {
        let npdu = bytes::Bytes::from(vec![0x01, 0x00, 0x41, invoke_id, seq, window]);
        let frame = ReceivedNpdu {
            direct_response: None,
            npdu,
            source_mac: bacnet_types::MacAddr::from_slice(&PEER),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
            reply_tx: None,
        };
        self.inbound.send(frame).await.unwrap();
    }

    async fn answer(&self, invoke_id: u8) {
        let ack = SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        };
        inject_apdu(&self.inbound, &PEER, None, Apdu::SimpleAck(ack)).await;
    }

    fn remembers(&self) -> Option<bool> {
        let peers = self.client.device_table.whole_first_window.lock();
        peers.as_ref().map(|peers| peers.contains(&PEER))
    }

    async fn stop(self) {
        let mut client = Arc::try_unwrap(self.client).ok().unwrap();
        client.stop().await.unwrap();
    }
}

/// A conforming peer answers segment zero at once: each window goes out
/// once, nothing is sent twice, and the peer is never remembered. A window
/// of one doesn't touch the table at all.
#[tokio::test(start_paused = true)]
async fn a_peer_answering_segment_zero_gets_each_window_once() {
    for window in [1u8, 2, 8] {
        let mut rig = Rig::new(window, 2).await;
        let total = 2 * usize::from(window) + 2;
        let task = rig.request(44 * (total - 1) + 10);
        let first = rig.segment().await;
        assert_eq!(first.sequence_number, Some(0));
        assert_eq!(first.proposed_window_size, Some(window));
        rig.quiet().await;
        let id = first.invoke_id;
        rig.ack(id, 0, window, false).await;
        let mut sent = vec![0u8];
        let mut next = 1;
        while next < total {
            let end = (next + usize::from(window)).min(total);
            sent.extend(rig.segments(end - next).await);
            rig.quiet().await;
            rig.ack(id, (end - 1) as u8, window, false).await;
            next = end;
        }
        assert_eq!(
            sent,
            (0..total as u8).collect::<Vec<_>>(),
            "window {window}"
        );
        rig.answer(id).await;
        task.await.unwrap().unwrap();
        rig.quiet().await;
        assert_eq!(rig.remembers(), None, "window {window}: nothing remembered");
        rig.stop().await;
    }
}

/// A peer that answers segment zero only when it comes again (with NAK(0),
/// as this workspace's server did before #2742): the first request waits
/// one segment timeout, and the next sends its whole first window at once.
#[tokio::test(start_paused = true)]
async fn a_peer_that_answers_only_the_repeat_costs_one_segment_timeout_once() {
    let mut rig = Rig::new(4, 2).await;
    let start = Instant::now();
    let task = rig.request(SIX_SEGMENTS);
    let first = rig.segment().await;
    let id = first.invoke_id;
    // Unanswered: after a segment timeout it comes again.
    assert_eq!(rig.segments(1).await, [0]);
    assert!(start.elapsed() >= TSEG);
    assert_eq!(rig.remembers(), Some(true));
    rig.ack(id, 0, 4, true).await;
    assert_eq!(rig.segments(4).await, [1, 2, 3, 4]);
    rig.ack(id, 4, 4, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 4, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();
    assert!(start.elapsed() < TSEG * 2, "one segment timeout, once");

    let start = Instant::now();
    let task = rig.request(SIX_SEGMENTS);
    let (invoke_id, first) = rig.window(4).await;
    assert_eq!(first, [0, 1, 2, 3], "the whole first window");
    rig.quiet().await;
    rig.ack(invoke_id, 3, 4, false).await;
    assert_eq!(rig.segments(2).await, [4, 5]);
    rig.ack(invoke_id, 5, 4, false).await;
    rig.answer(invoke_id).await;
    task.await.unwrap().unwrap();
    assert!(start.elapsed() < TSEG, "no wait the second time");
    assert_eq!(rig.remembers(), Some(true), "it never answers segment zero");
    rig.stop().await;
}

/// A conforming peer that once missed segment zero (lost, offline, slow) is
/// remembered, sent its next request's whole first window, and forgotten
/// when it acknowledges segment zero at once; the request after that sends
/// segment zero alone again.
#[tokio::test(start_paused = true)]
async fn a_remembered_peer_answering_segment_zero_at_once_is_forgotten() {
    let mut rig = Rig::new(4, 2).await;
    let task = rig.request(SIX_SEGMENTS);
    let (id, lost) = rig.window(1).await;
    assert_eq!(lost, [0]);
    assert_eq!(rig.segments(1).await, [0], "sent again");
    rig.ack(id, 0, 4, false).await;
    assert_eq!(rig.segments(4).await, [1, 2, 3, 4]);
    rig.ack(id, 4, 4, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 4, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();
    assert_eq!(rig.remembers(), Some(true));

    let task = rig.request(SIX_SEGMENTS);
    let (id, first) = rig.window(4).await;
    assert_eq!(first, [0, 1, 2, 3], "the whole first window");
    rig.ack(id, 0, 4, false).await;
    assert_eq!(rig.segments(4).await, [1, 2, 3, 4]);
    assert_eq!(rig.remembers(), Some(false), "forgotten");
    rig.ack(id, 4, 4, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 4, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();

    let task = rig.request(SIX_SEGMENTS);
    let (id, first) = rig.window(1).await;
    assert_eq!(first, [0]);
    rig.quiet().await;
    rig.ack(id, 0, 4, false).await;
    assert_eq!(rig.segments(4).await, [1, 2, 3, 4]);
    rig.ack(id, 4, 4, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 4, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();
    rig.stop().await;
}

/// The window an answer grants is taken from one to the window proposed. A
/// window of 0 or of 200 can't be written in a Segment-ACK: such an answer
/// doesn't decode and is dropped, and the transfer waits for a good one. A
/// grant of 100 against a proposal of four sends four.
#[tokio::test(start_paused = true)]
async fn the_window_granted_is_at_least_one_and_at_most_the_window_proposed() {
    let mut rig = Rig::new(4, 2).await;
    let task = rig.request(SIX_SEGMENTS);
    let (id, first) = rig.window(1).await;
    assert_eq!(first, [0]);
    for window in [0, 200] {
        rig.raw_ack(id, 0, window).await;
        rig.quiet().await;
    }
    rig.ack(id, 0, 100, false).await;
    assert_eq!(rig.segments(4).await, [1, 2, 3, 4]);
    rig.quiet().await;
    rig.ack(id, 4, 1, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 1, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();
    rig.stop().await;
}

/// A peer that drops segment zero's repeat too: that request fails, the
/// peer is remembered, and the next request completes.
#[tokio::test(start_paused = true)]
async fn a_peer_that_never_answers_segment_zero_alone_fails_once_then_works() {
    let mut rig = Rig::new(4, 1).await;
    let task = rig.request(SIX_SEGMENTS);
    assert_eq!(rig.segments(2).await, [0, 0], "sent, and sent again");
    let failed = task.await.unwrap();
    assert!(
        matches!(failed, Err(Error::Remote(RemoteError::Abort { reason }))
            if reason == AbortReason::TSM_TIMEOUT),
        "{failed:?}"
    );
    assert_eq!(rig.remembers(), Some(true));

    let task = rig.request(SIX_SEGMENTS);
    let (id, first) = rig.window(4).await;
    assert_eq!(first, [0, 1, 2, 3]);
    rig.ack(id, 3, 4, false).await;
    assert_eq!(rig.segments(2).await, [4, 5]);
    rig.ack(id, 5, 4, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();
    rig.stop().await;
}

/// A later window that goes unanswered is sent again, as ever, and teaches
/// nothing: only segment zero, sent alone, does.
#[tokio::test(start_paused = true)]
async fn a_later_window_going_unanswered_is_not_remembered() {
    let mut rig = Rig::new(2, 2).await;
    let task = rig.request(SIX_SEGMENTS);
    let (id, first) = rig.window(1).await;
    assert_eq!(first, [0]);
    rig.ack(id, 0, 2, false).await;
    assert_eq!(rig.segments(2).await, [1, 2]);
    // Unanswered: the window goes again after a segment timeout.
    assert_eq!(rig.segments(2).await, [1, 2]);
    rig.ack(id, 2, 2, false).await;
    assert_eq!(rig.segments(2).await, [3, 4]);
    rig.ack(id, 4, 2, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 2, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();
    assert_eq!(rig.remembers(), None);
    rig.stop().await;
}

/// At a window of one segment zero always went alone: a peer that never
/// answers fails the request as before, and nothing is remembered.
#[tokio::test(start_paused = true)]
async fn a_window_of_one_never_touches_the_table() {
    let mut rig = Rig::new(1, 1).await;
    let task = rig.request(SIX_SEGMENTS);
    assert_eq!(rig.segments(2).await, [0, 0]);
    assert!(task.await.unwrap().is_err());
    assert_eq!(rig.remembers(), None);
    rig.stop().await;
}

/// A peer that answers segment zero only after the segment timeout, every
/// time, stays remembered: its acknowledgment of segment zero, coming after
/// the whole first window's timeout ran out, forgets nothing (else it would
/// be forgotten and learned again on every other request).
#[tokio::test(start_paused = true)]
async fn a_peer_answering_segment_zero_late_stays_remembered() {
    let mut rig = Rig::new(4, 2).await;
    let task = rig.request(SIX_SEGMENTS);
    let (id, _) = rig.window(1).await;
    assert_eq!(rig.segments(1).await, [0], "sent again");
    rig.ack(id, 0, 4, false).await;
    assert_eq!(rig.segments(4).await, [1, 2, 3, 4]);
    rig.ack(id, 4, 4, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 4, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();

    let task = rig.request(SIX_SEGMENTS);
    let (id, first) = rig.window(4).await;
    assert_eq!(first, [0, 1, 2, 3], "the whole first window");
    assert_eq!(rig.segments(4).await, [0, 1, 2, 3], "sent again");
    rig.ack(id, 0, 4, false).await;
    assert_eq!(rig.segments(4).await, [1, 2, 3, 4]);
    rig.ack(id, 4, 4, false).await;
    assert_eq!(rig.segments(1).await, [5]);
    rig.ack(id, 5, 4, false).await;
    rig.answer(id).await;
    task.await.unwrap().unwrap();
    assert_eq!(rig.remembers(), Some(true), "a late answer forgets nothing");
    rig.stop().await;
}

/// Segment zero went out and its answer granted the window; the client's
/// link then refused the next segment's first send for its state, as a
/// link that stopped does (#1833). The request ends cut off as one that
/// went out, not with that refusal, which says nothing was done.
#[tokio::test(start_paused = true)]
async fn a_later_segment_its_stopped_link_refuses_ends_the_request_cut_off_as_sent() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let config = ClientConfig {
        apdu_timeout_ms: TSEG.as_millis() as u64,
        apdu_retries: 0,
        max_apdu_length: 50,
        proposed_window_size: 2,
        ..ClientConfig::default()
    };
    let (inbound, inbound_rx) = mpsc::channel(16);
    let (frames_tx, mut frames) = mpsc::unbounded_channel();
    let unicasts = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&unicasts);
    let transport = TestTransport::builder()
        .local_mac(&[1])
        .inbound(inbound_rx)
        .on_send(move |frame: SentFrame| {
            let later = !frame.broadcast && counted.fetch_add(1, Ordering::SeqCst) > 0;
            let _ = frames_tx.send(frame);
            async move {
                match later {
                    true => Err(bacnet_transport::port::stopped()),
                    false => Ok(()),
                }
            }
        })
        .build();
    let client = Arc::new(BACnetClient::start(config, transport).await.unwrap());
    let request = tokio::spawn({
        let client = Arc::clone(&client);
        let data: Vec<u8> = (0..SIX_SEGMENTS).map(|i| i as u8).collect();
        async move {
            let service = ConfirmedServiceChoice::WRITE_PROPERTY;
            client.confirmed_request(&PEER, service, &data).await
        }
    });
    let zero = timeout(Duration::from_secs(60), frames.recv())
        .await
        .unwrap()
        .unwrap();
    let npdu = decode_npdu(zero.npdu).unwrap();
    let Apdu::ConfirmedRequest(zero) = apdu::decode_apdu(npdu.payload).unwrap() else {
        panic!("expected segment zero")
    };
    assert_eq!(zero.sequence_number, Some(0));
    let ack = SegmentAck {
        negative_ack: false,
        sent_by_server: true,
        invoke_id: zero.invoke_id,
        sequence_number: 0,
        actual_window_size: 2,
    };
    inject_apdu(&inbound, &PEER, None, Apdu::SegmentAck(ack)).await;
    let ended = timeout(Duration::from_secs(60), request)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(ended, Err(Error::RequestCutOff { sent: true })),
        "{ended:?}"
    );
    assert_eq!(unicasts.load(Ordering::SeqCst), 2, "segment 1 was refused");
}
