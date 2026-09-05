//! Routed byte refusal, failure and saved-owner release at real async boundaries.

use super::*;
use crate::server::segmented_receive::tests::observe_payload_drops;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_transport::port::ReceivedNpdu;
use request_peer_quota::{assert_positive_ack, assert_server_abort, next_routed_apdu};
use request_reassembly::{
    inject_routed_segment, present_value, write_property_payload, RoutedInjectionTransport,
};
use std::sync::atomic::AtomicUsize;
use tokio::time::timeout;

#[derive(Default)]
struct SendControl {
    block_next: AtomicBool,
    fail_next: AtomicBool,
    started: Notify,
    release: Notify,
}

struct ControlledTransport {
    inner: RoutedInjectionTransport,
    control: Arc<SendControl>,
}

impl TransportPort for ControlledTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await?;
        let fail = self.control.fail_next.swap(false, Ordering::SeqCst);
        if self.control.block_next.swap(false, Ordering::SeqCst) {
            self.control.started.notify_one();
            self.control.release.notified().await;
        }
        if fail {
            return Err(Error::Encoding(
                "injected send failure after recording attempt".into(),
            ));
        }
        Ok(())
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}

struct Fixture {
    server: BACnetServer<ControlledTransport>,
    incoming: mpsc::Sender<ReceivedNpdu>,
    sent: SentFrames,
    control: Arc<SendControl>,
    router: MacAddr,
    remote: NpduAddress,
    index: usize,
}

impl Fixture {
    async fn start() -> Self {
        let sent = StdArc::new(StdMutex::new(Vec::new()));
        let (inner, incoming) = RoutedInjectionTransport::new(Arc::clone(&sent));
        let control = Arc::new(SendControl::default());
        let transport = ControlledTransport {
            inner,
            control: Arc::clone(&control),
        };
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            CharacterStringValueObject::new(1, "CSV-1").unwrap(),
        ))
        .unwrap();
        let config = ServerConfig {
            segmentation_supported: Segmentation::BOTH,
            ..ServerConfig::default()
        };
        let server = BACnetServer::start(config, db, transport).await.unwrap();
        Self {
            server,
            incoming,
            sent,
            control,
            router: test_mac(30),
            remote: routed_address(400, 40),
            index: 0,
        }
    }

    async fn segment(&self, invoke: u8, seq: u8, more: bool, data: &[u8]) {
        inject_routed_segment(
            &self.incoming,
            &self.router,
            &self.remote,
            invoke,
            seq,
            more,
            data,
        )
        .await;
    }

    async fn reply(&mut self) -> Apdu {
        next_routed_apdu(&self.sent, &mut self.index, &self.router, &self.remote).await
    }

    async fn fill(&mut self, bytes: usize) -> [u8; 12] {
        let mut next = [0; 12];
        let mut remaining = bytes;
        let mut turn = 0;
        while remaining != 0 {
            let invoke = turn % 12;
            let size = remaining.min(1476);
            self.segment(invoke as u8, next[invoke], true, &vec![invoke as u8; size])
                .await;
            assert_positive_ack(self.reply().await, invoke as u8, next[invoke]);
            next[invoke] += 1;
            remaining -= size;
            turn += 1;
        }
        next
    }

    fn block_and_fail_next_send(&self) {
        self.control.block_next.store(true, Ordering::SeqCst);
        self.control.fail_next.store(true, Ordering::SeqCst);
    }

    async fn wait_blocked(&self) {
        timeout(Duration::from_secs(2), self.control.started.notified())
            .await
            .expect("send did not start");
    }

    async fn stop(&mut self) {
        timeout(Duration::from_secs(2), self.server.stop())
            .await
            .expect("stop blocked")
            .unwrap();
    }
}

#[tokio::test]
async fn request_byte_budget_routed_abort_removes_current_before_failed_send_and_stop_releases_rest(
) {
    let drops = Arc::new(AtomicUsize::new(0));
    let _probe = observe_payload_drops(&drops);
    let mut f = Fixture::start().await;
    let next = f.fill(4 * 1024 * 1024).await;
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    f.router = test_mac(31); // Same routed transaction, new immediate path.
    f.block_and_fail_next_send();
    f.segment(0, next[0], true, &[1]).await;
    f.wait_blocked().await;
    assert_eq!(
        drops.load(Ordering::SeqCst),
        237,
        "current saved owners released BEFORE Abort await"
    );
    assert_server_abort(f.reply().await, 0, AbortReason::BUFFER_OVERFLOW);
    f.control.release.notify_one();
    f.segment(0, next[0], true, &[1]).await;
    assert_server_abort(f.reply().await, 0, AbortReason::INVALID_APDU_IN_THIS_STATE);
    // Refill only the capacity released by request 0. Failed Abort must neither
    // restore it nor evict any of the other eleven requests.
    for seq in 0..237 {
        f.segment(12, seq, true, &[12; 1476]).await;
        assert_positive_ack(f.reply().await, 12, seq);
    }
    f.segment(13, 0, true, &[1]).await;
    assert_server_abort(f.reply().await, 13, AbortReason::BUFFER_OVERFLOW);
    assert_eq!(drops.load(Ordering::SeqCst), 237);
    assert_eq!(present_value(&f.server).await, "");
    f.stop().await;
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2842 + 237,
        "explicit stop joins dispatch and drops all remaining saved owners"
    );
}

#[tokio::test]
async fn request_byte_budget_final_ack_attempt_precedes_consuming_release_before_blocked_service() {
    let drops = Arc::new(AtomicUsize::new(0));
    let _probe = observe_payload_drops(&drops);
    let mut f = Fixture::start().await;
    let text = "release saved storage before async service work";
    let write = write_property_payload(text);
    let (first, last) = write.split_at(1);
    f.segment(1, 0, true, first).await;
    assert_positive_ack(f.reply().await, 1, 0);
    let db = Arc::clone(f.server.database());
    let guard = db.write().await; // Real service cannot proceed through this lock.
    f.block_and_fail_next_send();
    f.segment(1, 1, false, last).await;
    f.wait_blocked().await;
    assert_positive_ack(f.reply().await, 1, 1);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "do not complete before final ACK attempt finishes"
    );
    f.control.release.notify_one();
    // ACK failure still continues completion as before. This next input's ACK
    // proves the loop passed consuming completion; the service remains blocked.
    f.segment(2, 0, true, &[9]).await;
    assert_positive_ack(f.reply().await, 2, 0);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "first and final saved owners released before blocked service"
    );
    assert_eq!(
        sent_count(&f.sent),
        f.index,
        "no service response through locked DB"
    );
    drop(guard);
    assert!(matches!(f.reply().await, Apdu::SimpleAck(ack) if ack.invoke_id == 1));
    assert_eq!(present_value(&f.server).await, text);
    f.stop().await;
    assert_eq!(drops.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn request_byte_budget_validation_failures_do_not_charge_or_retain_input() {
    let drops = Arc::new(AtomicUsize::new(0));
    let _probe = observe_payload_drops(&drops);
    let mut f = Fixture::start().await;
    let next = f.fill(4 * 1024 * 1024 - 1477).await;
    // Budget fits this length exactly, so authoritative per-segment validation
    // (not the aggregate cap) rejects these initial and continuation inputs.
    f.segment(12, 0, true, &[7; 1477]).await;
    assert_server_abort(f.reply().await, 12, AbortReason::BUFFER_OVERFLOW);
    f.segment(12, 1, true, &[7]).await;
    assert_server_abort(f.reply().await, 12, AbortReason::INVALID_APDU_IN_THIS_STATE);
    f.segment(0, next[0], true, &[7; 1477]).await;
    assert_server_abort(f.reply().await, 0, AbortReason::BUFFER_OVERFLOW);
    assert_eq!(drops.load(Ordering::SeqCst), usize::from(next[0]));
    let write = write_property_payload("capacity after failed saves");
    f.segment(12, 0, false, &write).await;
    assert_positive_ack(f.reply().await, 12, 0);
    assert!(matches!(f.reply().await, Apdu::SimpleAck(ack) if ack.invoke_id == 12));
    assert_eq!(
        present_value(&f.server).await,
        "capacity after failed saves"
    );
    f.stop().await;
    assert_eq!(
        drops.load(Ordering::SeqCst),
        next.iter().map(|n| usize::from(*n)).sum::<usize>() + 1
    );
}

#[tokio::test]
async fn request_byte_budget_routed_initial_peer_and_global_capacity_precedence() {
    let mut f = Fixture::start().await;
    let next = f.fill(4 * 1024 * 1024).await;
    f.router = test_mac(31);
    f.segment(12, 0, true, &[1]).await;
    assert_server_abort(f.reply().await, 12, AbortReason::BUFFER_OVERFLOW);
    // Zero-size initial segments fill peer slots even when bytes are full.
    for invoke in 12..16 {
        f.segment(invoke, 0, true, &[]).await;
        assert_positive_ack(f.reply().await, invoke, 0);
    }
    f.segment(16, 0, true, &[1]).await;
    assert_server_abort(f.reply().await, 16, AbortReason::OUT_OF_RESOURCES);
    let original_remote = f.remote.clone();
    // Seven more canonical peers fill all 128 slots without adding payload.
    for peer in 41..48 {
        f.remote = routed_address(400, peer);
        for invoke in 0..16 {
            f.segment(invoke, 0, true, &[]).await;
            assert_positive_ack(f.reply().await, invoke, 0);
        }
    }
    f.remote = original_remote;
    f.segment(16, 0, true, &[1]).await;
    assert_server_abort(f.reply().await, 16, AbortReason::BUFFER_OVERFLOW);
    // A live request's zero-size growth ignores all NEW-request admission caps.
    f.segment(0, next[0], true, &[]).await;
    assert_positive_ack(f.reply().await, 0, next[0]);
    assert_eq!(present_value(&f.server).await, "");
    f.stop().await;
}
