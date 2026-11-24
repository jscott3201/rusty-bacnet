//! The event notifications waiting for an invoke ID go first-come (#2563):
//! only the oldest takes a released ID, whichever is polled first; one that
//! leaves, served, dropped or cancelled after its wake, hands the turn on
//! and holds no ID; a head that other work passed over takes the next ID;
//! closing ends every wait. The waits are polled by hand, in the order each
//! test names, so no scheduler decides who goes first; the last test runs
//! them as tasks on four worker threads.

use super::*;
use bacnet_endpoint_core::coordinator::{NOTIFICATION_RESERVE, REQUESTER_RESERVE, TerminalPolicy};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::Wake;

#[path = "notification_reserve_tests.rs"]
mod reserve;

const EVENT: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION;

/// What a node's client leases for a read it sends to `peer`.
fn client_read(peer: CanonicalPeer) -> LeaseMetadata {
    LeaseMetadata::requester(
        peer,
        ConfirmedServiceChoice::READ_PROPERTY,
        TerminalPolicy::ComplexAck,
    )
}

type Reserved = Result<NotificationReservation, NotificationReserveError>;

/// Counts the wakes a wait asked for.
#[derive(Default)]
struct Wakes(AtomicUsize);

impl Wake for Wakes {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// One notification's wait for an invoke ID, polled by hand.
struct Waiter<'a> {
    wait: Pin<Box<dyn Future<Output = Reserved> + 'a>>,
    wakes: Arc<Wakes>,
    /// What it reserved, kept leased.
    kept: Option<NotificationReservation>,
}

impl Waiter<'_> {
    fn poll(&mut self) -> Poll<Reserved> {
        let waker = Waker::from(Arc::clone(&self.wakes));
        self.wait.as_mut().poll(&mut Context::from_waker(&waker))
    }

    /// Poll once more: whether it now has an ID.
    fn served(&mut self) -> bool {
        match self.poll() {
            Poll::Ready(reserved) => {
                self.kept = Some(reserved.expect("an invoke ID"));
                true
            }
            Poll::Pending => false,
        }
    }

    fn woken(&self) -> usize {
        self.wakes.0.load(Ordering::SeqCst)
    }
}

/// A wait for an ID to `peer`, started by its first poll.
fn waiter(transactions: &NotificationTransactions, peer: u8) -> Waiter<'_> {
    let wait = transactions.reserve_waiting(canonical_direct_peer(&[peer]), EVENT, || {});
    Waiter {
        wait: Box::pin(wait),
        wakes: Arc::default(),
        kept: None,
    }
}

/// `count` waits, each polled once in turn, every ID leased.
fn queued(transactions: &NotificationTransactions, count: u8) -> Vec<Waiter<'_>> {
    let mut waiters: Vec<_> = (0..count).map(|at| waiter(transactions, at)).collect();
    for waiter in &mut waiters {
        assert!(waiter.poll().is_pending());
    }
    assert_eq!(waiting(transactions), usize::from(count));
    waiters
}

fn waiting(transactions: &NotificationTransactions) -> usize {
    transactions.core.state.lock().unwrap().waiters.len()
}

fn fresh(transactions: &NotificationTransactions) -> Reserved {
    transactions.reserve(canonical_direct_peer(&[0xEE]), EVENT)
}

fn exhausted(reserved: &Reserved) -> bool {
    matches!(
        reserved,
        Err(NotificationReserveError::Coordinator(
            ReserveError::Exhausted
        ))
    )
}

/// Each released ID goes to the oldest waiter, though the newest is polled
/// first, and the next is woken for its turn.
#[test]
fn waiters_are_served_in_the_order_they_began_to_wait() {
    let transactions = NotificationTransactions::new();
    let mut leases = transactions.lease_every_invoke_id();
    let mut waiters = queued(&transactions, 3);
    for served in 0..3 {
        drop(leases.pop());
        for at in (served..3).rev() {
            assert_eq!(
                waiters[at].served(),
                at == served,
                "ID {served}, waiter {at}"
            );
        }
        if served < 2 {
            assert!(waiters[served + 1].woken() > 0, "its turn, woken");
        }
    }
    assert_eq!(waiting(&transactions), 0);
}

/// One that arrives while others wait queues behind them, though an ID is
/// free, and counts its wait.
#[test]
fn a_later_arrival_waits_behind_those_waiting() {
    let transactions = NotificationTransactions::new();
    let mut leases = transactions.lease_every_invoke_id();
    let mut first = waiter(&transactions, 0);
    assert!(first.poll().is_pending());
    drop(leases.pop());
    let waits = std::cell::Cell::new(0);
    let later = transactions.reserve_waiting(canonical_direct_peer(&[1]), EVENT, || {
        waits.set(waits.get() + 1);
    });
    let mut later = Waiter {
        wait: Box::pin(later),
        wakes: Arc::default(),
        kept: None,
    };
    assert!(!later.served(), "behind the first");
    assert_eq!(waits.get(), 1, "its wait counted");
    assert!(first.served());
    drop(leases.pop());
    assert!(later.served());
}

/// A head dropped after a release woke it, before it took the ID, hands its
/// turn on; the next takes the ID.
#[test]
fn a_head_dropped_after_its_wake_leaves_the_id_to_the_next() {
    let transactions = NotificationTransactions::new();
    let mut leases = transactions.lease_every_invoke_id();
    let mut waiters = queued(&transactions, 2);
    drop(leases.pop());
    assert!(waiters[0].woken() > 0);
    let mut second = waiters.pop().unwrap();
    drop(waiters);
    assert_eq!(second.woken(), 1, "its turn");
    assert!(second.served());
    assert_eq!(transactions.active_count(), 256);
}

/// A waiter dropped from the middle of the queue holds up nobody.
#[test]
fn a_waiter_dropped_from_the_middle_holds_up_nobody() {
    let transactions = NotificationTransactions::new();
    let mut leases = transactions.lease_every_invoke_id();
    let mut waiters = queued(&transactions, 3);
    drop(waiters.remove(1));
    assert_eq!(waiting(&transactions), 2);
    drop(leases.pop());
    assert!(!waiters[1].served());
    assert!(waiters[0].served());
    drop(leases.pop());
    assert!(waiters[1].served());
}

/// No ID is set aside for a head: while it isn't polled, the one released
/// stays free, and work that doesn't wait can take it.
#[test]
fn an_unpolled_head_holds_no_id() {
    let transactions = NotificationTransactions::new();
    let mut leases = transactions.lease_every_invoke_id();
    let _waiters = queued(&transactions, 2);
    drop(leases.pop());
    assert_eq!(transactions.active_count(), 255);
    assert!(fresh(&transactions).is_ok());
}

/// Work that doesn't wait may take an ID first, once: the head that a
/// release woke and that found none is owed the next, which no other
/// reservation then takes, unless two are free.
#[test]
fn a_head_passed_over_takes_the_next_id() {
    let transactions = NotificationTransactions::new();
    let mut leases = transactions.lease_every_invoke_id();
    let mut head = waiter(&transactions, 0);
    assert!(head.poll().is_pending());
    drop(leases.pop());
    let _first = fresh(&transactions).expect("may go first once");
    assert!(!head.served(), "passed over");
    drop(leases.pop());
    assert_eq!(transactions.free_invoke_ids(), 0, "the free one is owed");
    assert!(exhausted(&fresh(&transactions)), "held back");
    assert!(head.served());
    drop(leases.pop());
    let _after = fresh(&transactions).expect("no longer held back");

    let mut head = waiter(&transactions, 1);
    assert!(head.poll().is_pending());
    drop(leases.pop());
    let _first = fresh(&transactions).expect("may go first once");
    assert!(!head.served(), "passed over");
    drop(leases.pop());
    drop(leases.pop());
    assert_eq!(transactions.free_invoke_ids(), 1, "two free, one owed");
    let _spare = fresh(&transactions).expect("one to spare");
    assert!(head.served());
}

/// The most a server on a node's seat may hold: the node's pool less the
/// client's reserve (#2617).
const SEAT_CEILING: usize = 256 - REQUESTER_RESERVE;

/// A server on a node's seat: the node's pool, which keeps the client
/// [`REQUESTER_RESERVE`] and the server [`NOTIFICATION_RESERVE`] (#2617).
fn seated() -> (
    Arc<OutboundTransactionCoordinator>,
    Arc<NotificationTransactions>,
) {
    let coordinator = Arc::new(OutboundTransactionCoordinator::with_reserves(
        REQUESTER_RESERVE,
        NOTIFICATION_RESERVE,
    ));
    let transactions = NotificationTransactions::with_coordinator(Arc::clone(&coordinator));
    (coordinator, transactions)
}

/// On a seat at its ceiling, the node's pool with IDs to spare, the
/// server's own work keeps the order: the oldest waiter takes the first
/// lease to end, and a head that other work passed over takes the next.
#[test]
fn on_a_seat_the_order_holds_among_the_servers_own_work() {
    let (_coordinator, transactions) = seated();
    let mut leases: Vec<_> = (0..SEAT_CEILING)
        .map(|_| fresh(&transactions).expect("within the ceiling"))
        .collect();
    assert!(exhausted(&fresh(&transactions)), "at the ceiling");
    assert_eq!(
        transactions.active_count(),
        SEAT_CEILING,
        "the pool has more"
    );
    let mut waiters = queued(&transactions, 2);
    drop(leases.pop());
    assert!(!waiters[1].served());
    assert!(waiters[0].served(), "the oldest");
    drop(leases.pop());
    let _first = fresh(&transactions).expect("may go first as the turn passes");
    assert!(!waiters[1].served());
    drop(leases.pop());
    let _second = fresh(&transactions).expect("may go first once more");
    assert!(!waiters[1].served(), "passed over");
    drop(leases.pop());
    assert_eq!(transactions.free_invoke_ids(), 0, "the free one is owed");
    assert!(exhausted(&fresh(&transactions)), "held back");
    assert!(waiters[1].served());
}

/// On a seat holding no lease of its own, other leases filling the server's
/// share of the node's pool and the node's client the rest, closing ends no
/// lease, and still wakes the head as well as the rest, each to find it
/// closed.
#[test]
fn closing_wakes_a_head_with_no_lease_to_end() {
    let (coordinator, transactions) = seated();
    for at in 0..=u8::MAX {
        let peer = canonical_direct_peer(&[at]);
        let metadata = if usize::from(at) < SEAT_CEILING {
            LeaseMetadata::notification(peer, EVENT)
        } else {
            client_read(peer)
        };
        coordinator.reserve(metadata).expect("a free ID");
    }
    let mut waiters = queued(&transactions, 2);
    transactions.close();
    for waiter in &mut waiters {
        assert!(waiter.woken() > 0, "woken by the close");
        assert!(matches!(
            waiter.poll(),
            Poll::Ready(Err(NotificationReserveError::Closed))
        ));
    }
}

/// Closing ends every wait, the queued ones woken to find it closed.
#[test]
fn closing_ends_every_wait() {
    let transactions = NotificationTransactions::new();
    let _leases = transactions.lease_every_invoke_id();
    let mut waiters = queued(&transactions, 2);
    transactions.close();
    assert!(waiters[1].woken() > 0);
    for waiter in &mut waiters {
        assert!(matches!(
            waiter.poll(),
            Poll::Ready(Err(NotificationReserveError::Closed))
        ));
    }
    assert_eq!(waiting(&transactions), 0);
}

/// On four worker threads, five waits started in turn take the IDs released
/// one at a time in the order they began to wait, every round.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn on_worker_threads_the_waiters_go_in_order() {
    let transactions = NotificationTransactions::new();
    for round in 0..50 {
        let mut leases = transactions.lease_every_invoke_id();
        let (served, mut order) = tokio::sync::mpsc::unbounded_channel();
        let mut tasks = Vec::new();
        for at in 0..5_u8 {
            let (owner, served) = (Arc::clone(&transactions), served.clone());
            tasks.push(tokio::spawn(async move {
                let peer = canonical_direct_peer(&[at]);
                let reserved = owner.reserve_waiting(peer, EVENT, || {}).await;
                served.send(at).unwrap();
                reserved.expect("an invoke ID")
            }));
            let queued = async {
                while waiting(&transactions) <= usize::from(at) {
                    tokio::task::yield_now().await;
                }
            };
            tokio::time::timeout(std::time::Duration::from_secs(5), queued)
                .await
                .expect("queued");
        }
        let mut went = Vec::new();
        for _ in 0..5 {
            drop(leases.pop());
            went.push(order.recv().await.unwrap());
        }
        assert_eq!(went, [0, 1, 2, 3, 4], "round {round}");
        for task in tasks {
            drop(task.await.unwrap());
        }
        drop(leases);
    }
}
