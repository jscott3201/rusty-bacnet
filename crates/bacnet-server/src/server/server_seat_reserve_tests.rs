//! The server's share of the node's invoke IDs beside the node's client
//! (#2617): each keeps its reserve whatever the other holds, the server
//! borrows up to the client's, and a confirmed event notification waiting
//! for an ID is served once the server is under its reserve, whatever the
//! client does.
use super::super::notification_transactions::NotificationReserveError;
use super::tests::{PEER, node};
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_endpoint_core::coordinator::{
    CanonicalPeer, LeaseMetadata, LeaseToken, NOTIFICATION_RESERVE, OutboundTransactionCoordinator,
    REQUESTER_RESERVE, ReserveError, TerminalPolicy,
};

const COV: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;
const EVENT: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION;

/// The most the seated server may hold: the pool less the client's reserve.
const SERVER_CEILING: usize = 256 - REQUESTER_RESERVE;

fn peer() -> CanonicalPeer {
    canonical_direct_peer(&PEER)
}

/// A lease for a read the node's client sends.
fn client(coordinator: &OutboundTransactionCoordinator) -> Result<LeaseToken, ReserveError> {
    coordinator.reserve(LeaseMetadata::requester(
        canonical_direct_peer(&[0x77]),
        ConfirmedServiceChoice::READ_PROPERTY,
        TerminalPolicy::ComplexAck,
    ))
}

fn exhausted<T>(reserved: Result<T, NotificationReserveError>) -> bool {
    matches!(
        reserved,
        Err(NotificationReserveError::Coordinator(
            ReserveError::Exhausted
        ))
    )
}

/// A confirmed event notification's wait for an ID, as a task: the invoke ID
/// it got.
fn wait_for_id(
    pool: &Arc<NotificationTransactions>,
) -> tokio::task::JoinHandle<Result<u8, NotificationReserveError>> {
    let pool = Arc::clone(pool);
    tokio::spawn(async move {
        pool.reserve_waiting(peer(), EVENT, || {})
            .await
            .map(|(operation, _)| operation.invoke_id())
    })
}

/// A seated server holds up to all of the node's pool but the client's
/// reserve, whatever it sends; the client still finds its whole reserve,
/// and an event notification then waits for one of the server's own leases
/// to end. A server on its own link may take the whole pool.
#[tokio::test(start_paused = true)]
async fn a_seated_server_borrows_up_to_the_clients_reserve() {
    let (mut node, seat) = node().await;
    let mut server = BACnetServer::generic_builder()
        .seat(seat)
        .build()
        .await
        .unwrap();
    let pool = Arc::clone(&server.notification_transactions);
    let mut held: Vec<_> = (0..SERVER_CEILING)
        .map(|_| pool.reserve(peer(), COV).expect("within the ceiling"))
        .collect();
    // A read the server sends shares the ceiling.
    assert!(exhausted(
        pool.reserve_read(peer(), ConfirmedServiceChoice::READ_PROPERTY)
    ));
    assert!(exhausted(pool.reserve(peer(), COV)));
    let reads: Vec<_> = (0..REQUESTER_RESERVE)
        .map(|_| client(&node.coordinator).expect("the client's reserve"))
        .collect();
    assert_eq!(node.coordinator.active_count().unwrap(), 256);
    let waiting = wait_for_id(&pool);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(!waiting.is_finished(), "at the ceiling it waits");
    drop(held.pop());
    let invoke = tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("woken by the freed lease")
        .unwrap()
        .expect("an ID within the ceiling");
    assert!(reads.iter().all(|read| read.invoke_id() != invoke));
    for read in reads {
        node.coordinator.release(read).unwrap();
    }
    drop(held);
    server.stop().await.unwrap();
    node.ingress.stop().await.unwrap();

    let mut own = BACnetServer::generic_builder()
        .transport(TestTransport::new())
        .build()
        .await
        .unwrap();
    let whole: Vec<_> = (0..256)
        .map(|_| {
            own.notification_transactions
                .reserve(peer(), COV)
                .expect("the whole pool on its own link")
        })
        .collect();
    drop(whole);
    own.stop().await.unwrap();
}

/// The node's client keeps the pool full: it leases again at every release,
/// and every 10 ms one of its requests ends and it starts another. The
/// server holds 16 past its reserve, and one of its leases ends each second
/// (a slow recipient's notification running out of retries). The waiting
/// event notification is served no later than the end that takes the
/// server under its reserve, and the client never holds past its ceiling.
#[tokio::test(start_paused = true)]
async fn a_waiting_head_is_served_under_a_client_keeping_the_pool_full() {
    let (mut node, seat) = node().await;
    let mut server = BACnetServer::generic_builder()
        .seat(seat)
        .build()
        .await
        .unwrap();
    let pool = Arc::clone(&server.notification_transactions);
    let borrowed = 16;
    let mut held: Vec<_> = (0..NOTIFICATION_RESERVE + borrowed)
        .map(|_| pool.reserve(peer(), COV).expect("within the ceiling"))
        .collect();
    let (stop, mut stopped) = oneshot::channel::<()>();
    let coordinator = Arc::clone(&node.coordinator);
    let churn = tokio::spawn(async move {
        let (mut leases, mut most) = (Vec::new(), 0);
        loop {
            // Made before the pool is read, so no release is missed.
            let released = coordinator.released();
            tokio::pin!(released);
            released.as_mut().enable();
            while let Ok(token) = client(&coordinator) {
                leases.push(token);
            }
            most = most.max(leases.len());
            tokio::select! {
                _ = &mut stopped => break,
                () = released => {}
                () = tokio::time::sleep(Duration::from_millis(10)) => {
                    if let Some(token) = leases.pop() {
                        coordinator.release(token).unwrap();
                    }
                }
            }
        }
        (leases, most)
    });
    tokio::task::yield_now().await;
    assert_eq!(node.coordinator.active_count().unwrap(), 256, "full");
    let started = tokio::time::Instant::now();
    let waiting = wait_for_id(&pool);
    let mut ended = 0;
    while !waiting.is_finished() {
        assert!(ended <= borrowed, "past the end under the reserve");
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(
            !waiting.is_finished(),
            "served before a lease of its own ended"
        );
        drop(held.pop());
        ended += 1;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    // Each end is a second and a millisecond after the last.
    let ends = u64::try_from(borrowed + 1).unwrap();
    assert!(started.elapsed() <= Duration::from_millis(ends * 1_001));
    waiting.await.unwrap().expect("an ID");
    stop.send(()).unwrap();
    let (leases, most) = churn.await.unwrap();
    assert!(
        most <= 256 - NOTIFICATION_RESERVE,
        "the client's ceiling: {most}"
    );
    for token in leases {
        node.coordinator.release(token).unwrap();
    }
    drop(held);
    server.stop().await.unwrap();
    node.ingress.stop().await.unwrap();
}

/// A head the client keeps waiting ends when the server stops or is
/// dropped, finding it closed; the client's leases stay.
#[tokio::test(start_paused = true)]
async fn stop_or_drop_ends_a_head_the_client_keeps_waiting() {
    for stop in [true, false] {
        let (mut node, seat) = node().await;
        let mut server = BACnetServer::generic_builder()
            .seat(seat)
            .build()
            .await
            .unwrap();
        let pool = Arc::clone(&server.notification_transactions);
        let _held: Vec<_> = (0..NOTIFICATION_RESERVE)
            .map(|_| pool.reserve(peer(), COV).expect("its reserve"))
            .collect();
        let reads: Vec<_> = std::iter::from_fn(|| client(&node.coordinator).ok()).collect();
        assert_eq!(reads.len(), 256 - NOTIFICATION_RESERVE);
        let waiting = wait_for_id(&pool);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!waiting.is_finished());
        if stop {
            server.stop().await.unwrap();
        } else {
            drop(server);
        }
        let ended = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("woken by the close")
            .unwrap();
        assert!(
            matches!(ended, Err(NotificationReserveError::Closed)),
            "{ended:?}"
        );
        assert_eq!(
            node.coordinator.active_count().unwrap(),
            reads.len(),
            "the client's leases stay; the server's are cancelled"
        );
        for read in reads {
            node.coordinator.release(read).unwrap();
        }
        node.ingress.stop().await.unwrap();
    }
}
