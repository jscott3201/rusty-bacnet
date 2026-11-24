//! Routed byte refusal, failure and the release of saved bytes at real async
//! boundaries. The shared receiver (#2742) releases a transfer's bytes inside
//! the call that ends it, before the server sends anything; these tests see
//! the release as capacity the next transfer can use.

use super::*;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_transport::port::ReceivedNpdu;
use request_peer_quota::{assert_positive_ack, assert_server_abort, next_routed_apdu};
use request_reassembly::{
    inject_routed_segment, present_value, routed_injection_transport, write_property_payload,
};
use tokio::time::timeout;

struct Fixture {
    server: BACnetServer<TestTransport>,
    incoming: mpsc::Sender<ReceivedNpdu>,
    sent: SendLog,
    control: TestTransportHandle,
    router: MacAddr,
    remote: NpduAddress,
    index: usize,
}

impl Fixture {
    async fn start() -> Self {
        let (transport, incoming, sent) = routed_injection_transport();
        let control = transport.handle();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            CharacterStringValueObject::new(1, "CSV-1").unwrap(),
        ))
        .unwrap();
        let config = ServerConfig {
            device: bacnet_objects::DeviceIdentity::default()
                .with_segmentation(Segmentation::SEGMENTED_BOTH),
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
        self.control.block_next_send();
        self.control.fail_next_send();
    }

    async fn wait_blocked(&self) {
        timeout(Duration::from_secs(2), self.control.wait_blocked())
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
async fn request_byte_budget_routed_abort_frees_exactly_its_bytes_despite_a_failed_send() {
    let mut f = Fixture::start().await;
    let next = f.fill(4 * 1024 * 1024).await;
    f.router = test_mac(31); // Same routed transaction, new immediate path.
    f.block_and_fail_next_send();
    f.segment(0, next[0], true, &[1]).await;
    f.wait_blocked().await;
    assert_server_abort(f.reply().await, 0, AbortReason::BUFFER_OVERFLOW);
    f.control.release_sends(1);
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
    assert_eq!(present_value(&f.server).await, "");
    f.stop().await;
}

#[tokio::test]
async fn request_byte_budget_failed_final_ack_still_hands_on_the_request_before_blocked_service() {
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
    assert_eq!(sent_count(&f.sent), f.index, "the final ACK goes first");
    f.control.release_sends(1);
    // ACK failure still continues completion as before. This next input's ACK
    // proves the loop handed the request on; the service remains blocked.
    f.segment(2, 0, true, &[9]).await;
    assert_positive_ack(f.reply().await, 2, 0);
    assert_eq!(
        sent_count(&f.sent),
        f.index,
        "no service response through locked DB"
    );
    drop(guard);
    assert!(matches!(f.reply().await, Apdu::SimpleAck(ack) if ack.invoke_id == 1));
    assert_eq!(present_value(&f.server).await, text);
    f.stop().await;
}

#[tokio::test]
async fn request_byte_budget_validation_failures_do_not_charge_or_retain_input() {
    let mut f = Fixture::start().await;
    let next = f.fill(4 * 1024 * 1024 - 1477).await;
    // Budget fits this length exactly, so per-segment validation (not the
    // aggregate cap) refuses these initial and continuation inputs.
    f.segment(12, 0, true, &[7; 1477]).await;
    assert_server_abort(f.reply().await, 12, AbortReason::BUFFER_OVERFLOW);
    f.segment(12, 1, true, &[7]).await;
    assert_server_abort(f.reply().await, 12, AbortReason::INVALID_APDU_IN_THIS_STATE);
    f.segment(0, next[0], true, &[7; 1477]).await;
    assert_server_abort(f.reply().await, 0, AbortReason::BUFFER_OVERFLOW);
    let write = write_property_payload("capacity after failed saves");
    f.segment(12, 0, false, &write).await;
    assert_positive_ack(f.reply().await, 12, 0);
    assert!(matches!(f.reply().await, Apdu::SimpleAck(ack) if ack.invoke_id == 12));
    assert_eq!(
        present_value(&f.server).await,
        "capacity after failed saves"
    );
    // Free now: the 1477 bytes the refused segments never took, and request
    // 0's full segments, which its Abort gave back. Exactly that fits again.
    assert_eq!(next[0], 237);
    let mut left = 1477 + usize::from(next[0]) * 1476;
    let mut seq = 0;
    while left != 0 {
        let size = left.min(1476);
        f.segment(20, seq, true, &vec![20; size]).await;
        assert_positive_ack(f.reply().await, 20, seq);
        left -= size;
        seq += 1;
    }
    f.segment(20, seq, true, &[1]).await;
    assert_server_abort(f.reply().await, 20, AbortReason::BUFFER_OVERFLOW);
    f.stop().await;
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
