//! The router an address recipient's notifications go through (#2831,
//! decision of 2026-11-26): learned or replaced only through the server's
//! own router query. No answer teaches one, silence forgets none, and while
//! none is known the notifications go by local broadcast.

use super::*;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_network::layer::{QueueAdmissionCounters, ReceivedNetworkControl};
use bacnet_network::router_learning::RouterReports;
use bacnet_npdu::NetworkMessageType;

/// Another station on the link.
const STRANGER: &[u8] = &[10, 0, 0, 9, 0xBA, 0xC0];
/// The learner's wait for router answers in these tests.
const LEARN_WAIT: Duration = Duration::from_millis(20);

/// A router announcement (I-Am-Router-To-Network) from `router` on this
/// link, listing `networks`.
pub(super) fn announcement(router: &[u8], networks: &[u16]) -> ReceivedNetworkControl {
    let payload: Vec<u8> = networks.iter().flat_map(|n| n.to_be_bytes()).collect();
    let mut buf = BytesMut::new();
    encode_npdu(
        &mut buf,
        &Npdu {
            message_type: Some(NetworkMessageType::I_AM_ROUTER_TO_NETWORK),
            payload: Bytes::from(payload),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNetworkControl {
        npdu: decode_npdu(buf.freeze()).unwrap(),
        source_mac: MacAddr::from_slice(router),
        link_layer_group: true,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        ingress_sequence: 1,
    }
}

/// Whether `frame` is the server's router query for `network`.
fn is_router_query(frame: &crate::server::test_transport::SentFrame, network: u16) -> bool {
    let npdu = frame.decode_npdu();
    frame.broadcast
        && npdu.message_type == Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
        && npdu.payload.as_ref() == network.to_be_bytes()
}

async fn until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..2_000 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    panic!("{what}");
}

impl Harness {
    /// Learn `router` for `network` the one way the server does: its own
    /// router query, which `router` alone answers. The learner runs for
    /// this only, and the send log is cleared after.
    pub(super) async fn learn_router(&self, network: u16, router: &[u8]) {
        let reports = RouterReports::new(Some(QueueAdmissionCounters::default()));
        let task = {
            let (learning, link) = (Arc::clone(&self.learned_routers), Arc::clone(&self.network));
            let reports = reports.clone();
            tokio::spawn(async move { learning.run(&link, &reports, || true, LEARN_WAIT).await })
        };
        self.learned_routers.unrouted(network);
        until("the router query", || {
            self.sent
                .broadcasts()
                .iter()
                .any(|frame| is_router_query(frame, network))
        })
        .await;
        reports.offer(&announcement(router, &[network]));
        until("the router learned", || {
            self.learned_routers.router(network).is_some()
        })
        .await;
        task.abort();
        self.sent.clear();
        assert_eq!(
            self.learned_routers.router(network).as_deref(),
            Some(router)
        );
    }
}

/// The first notification goes by local broadcast; the router carrying its
/// answer is not learned, so the next goes by broadcast too.
#[tokio::test]
async fn an_answer_to_a_broadcast_attempt_teaches_no_router() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    for round in 0..3 {
        harness.distribute().await;
        let broadcasts = harness.broadcast_frames();
        assert_eq!(broadcasts.len(), round + 1);
        assert!(harness.unicast_frames().is_empty(), "round {round}");
        let (_, request) = decode_confirmed(&broadcasts[round]);
        assert!(
            harness
                .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
                .await,
            "any router may carry the answer to a broadcast attempt"
        );
        assert_eq!(harness.learned_routers.router(1000), None);
    }
}

/// A router the server's own query learned carries the next notification,
/// with the same DNET and DADR, by unicast to it.
#[tokio::test]
async fn a_router_the_query_learned_carries_the_next_notification() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    harness.learn_router(1000, ROUTER_A).await;
    harness.distribute().await;
    assert!(harness.broadcast_frames().is_empty());
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0.as_slice(), ROUTER_A);
    let (npdu, request) = decode_confirmed(&unicasts[0].1);
    assert!(npdu.expecting_reply);
    let destination = npdu.destination.unwrap();
    assert_eq!(destination.network, 1000);
    assert_eq!(destination.mac_address.as_slice(), RECIPIENT);
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
}

/// The router stays silent: the retry goes by local broadcast, its answer
/// from another station completes the notification, and the router is
/// neither forgotten nor replaced. The next notification goes to it again.
#[tokio::test(start_paused = true)]
async fn silence_forgets_nothing_and_the_retry_goes_by_broadcast() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        1_000,
    )
    .await;
    harness.learn_router(1000, ROUTER_A).await;
    harness.distribute().await;
    let (_, request) = decode_confirmed(&harness.unicast_frames()[0].1);
    tokio::time::sleep(Duration::from_millis(1_001)).await;
    assert_eq!(harness.broadcast_frames().len(), 1, "the retry broadcasts");
    assert!(
        harness
            .ack_routed(STRANGER, 1000, RECIPIENT, request.invoke_id)
            .await,
        "any router may carry the answer to a broadcast retry"
    );
    assert_eq!(
        harness.learned_routers.router(1000).as_deref(),
        Some(ROUTER_A)
    );
    harness.distribute().await;
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 2);
    assert_eq!(unicasts[1].0.as_slice(), ROUTER_A);
}

/// With no retries, the one attempt through a silent router ends the
/// notification unanswered and forgets nothing.
#[tokio::test(start_paused = true)]
async fn with_no_retries_a_silent_router_is_kept() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        1_000,
    )
    .await;
    harness.notification_transactions.set_retries(0);
    harness.learn_router(1000, ROUTER_A).await;
    harness.distribute().await;
    assert_eq!(harness.unicast_frames().len(), 1);
    tokio::time::sleep(Duration::from_millis(1_001)).await;
    assert_eq!(harness.notification_transactions.active_count(), 0);
    assert_eq!(
        harness.learned_routers.router(1000).as_deref(),
        Some(ROUTER_A)
    );
    harness.distribute().await;
    assert!(harness.broadcast_frames().is_empty());
    assert_eq!(harness.unicast_frames().len(), 2);
}

/// An answer from the router itself only keeps it: two recipients on one
/// network, both answered, one through ROUTER_A and one through a stranger,
/// teach nothing while no router is known.
#[tokio::test]
async fn answers_through_two_stations_teach_nothing() {
    let harness = Harness::new(
        vec![
            destination_for(address_recipient(1000, RECIPIENT), true),
            destination_for(address_recipient(1000, &[0x0B; 6]), true),
        ],
        60_000,
    )
    .await;
    harness.distribute().await;
    let broadcasts = harness.broadcast_frames();
    assert_eq!(broadcasts.len(), 2);
    for (frame, router) in broadcasts.iter().zip([ROUTER_A, STRANGER]) {
        let (npdu, request) = decode_confirmed(frame);
        let dadr = npdu.destination.unwrap().mac_address;
        assert!(
            harness
                .ack_routed(router, 1000, &dadr, request.invoke_id)
                .await
        );
    }
    assert_eq!(harness.learned_routers.router(1000), None);
}

/// A stranger's answer under the right invoke ID neither completes a
/// notification sent through the known router nor replaces that router.
#[tokio::test]
async fn a_strangers_answer_does_not_replace_a_known_router() {
    let harness = Harness::new(
        vec![destination_for(address_recipient(1000, RECIPIENT), true)],
        60_000,
    )
    .await;
    harness.learn_router(1000, ROUTER_A).await;
    harness.distribute().await;
    let (_, request) = decode_confirmed(&harness.unicast_frames()[0].1);
    assert!(
        !harness
            .ack_routed(STRANGER, 1000, RECIPIENT, request.invoke_id)
            .await
    );
    assert_eq!(
        harness.learned_routers.router(1000).as_deref(),
        Some(ROUTER_A)
    );
    assert!(
        harness
            .ack_routed(ROUTER_A, 1000, RECIPIENT, request.invoke_id)
            .await
    );
}
