//! The head of the event notifications waiting for an invoke ID against the
//! node's client (#2617): the client leases from the node's pool apart from
//! the queue, and passes the head over only while the server holds its
//! reserve or more. Once the head is owed, the server's other work leaves
//! it an ID the client can't take. Polled by hand, the client's leases made
//! on the coordinator between polls.

use super::*;

/// A lease for a read the node's client sends.
fn client(coordinator: &OutboundTransactionCoordinator) -> Result<LeaseToken, ReserveError> {
    coordinator.reserve(client_read(canonical_direct_peer(&[0x99])))
}

/// The client's leases, taken until the pool refuses it.
fn client_fills(coordinator: &OutboundTransactionCoordinator) -> Vec<LeaseToken> {
    std::iter::from_fn(|| client(coordinator).ok()).collect()
}

/// A client that keeps the pool full, leasing again at every release before
/// the head is polled, passes it over only while the server holds its
/// reserve or more: the release that takes the server under it is the
/// head's, the client then at its ceiling.
#[test]
fn a_client_keeping_the_pool_full_passes_the_head_only_while_the_server_holds_its_reserve() {
    let (coordinator, transactions) = seated();
    let borrowed = 16;
    let mut server: Vec<_> = (0..NOTIFICATION_RESERVE + borrowed)
        .map(|_| fresh(&transactions).expect("within the ceiling"))
        .collect();
    let mut held = client_fills(&coordinator);
    assert_eq!(held.len(), 256 - NOTIFICATION_RESERVE - borrowed, "full");
    let mut head = waiter(&transactions, 0);
    assert!(head.poll().is_pending());
    // The client's requests end, and it leases again each time.
    for churn in 0..10 {
        coordinator.release(held.pop().unwrap()).unwrap();
        held.push(client(&coordinator).expect("its own ID back"));
        assert!(!head.served(), "churn {churn}: passed over by the client");
    }
    // The server's leases end; the client takes each while the server is
    // over its reserve.
    for ended in 1..=borrowed {
        drop(server.pop());
        held.push(client(&coordinator).expect("the shared part"));
        assert!(!head.served(), "lease {ended} ended over the reserve");
    }
    assert_eq!(held.len(), 256 - NOTIFICATION_RESERVE, "its ceiling");
    drop(server.pop());
    assert!(
        matches!(client(&coordinator), Err(ReserveError::Exhausted)),
        "the server's reserve"
    );
    assert!(head.served(), "the first ID under the reserve");
    assert_eq!(waiting(&transactions), 0);
    for token in held {
        coordinator.release(token).unwrap();
    }
}

/// On a node two free IDs are not enough for other work to go first while
/// the head is owed one, as they are in a pool of the server's own: the
/// client may take either. With the server over its reserve, other work
/// is refused, and the head takes the ID the client leaves.
#[test]
fn on_a_node_two_free_ids_are_not_enough_to_go_before_an_owed_head() {
    let (coordinator, transactions) = seated();
    let _server: Vec<_> = (0..NOTIFICATION_RESERVE)
        .map(|_| fresh(&transactions).expect("its reserve"))
        .collect();
    let mut held = client_fills(&coordinator);
    let mut head = waiter(&transactions, 0);
    assert!(head.poll().is_pending());
    coordinator.release(held.pop().unwrap()).unwrap();
    coordinator.release(held.pop().unwrap()).unwrap();
    let _passing = fresh(&transactions).expect("may go first once");
    held.push(client(&coordinator).unwrap());
    assert!(!head.served(), "passed over");
    coordinator.release(held.pop().unwrap()).unwrap();
    coordinator.release(held.pop().unwrap()).unwrap();
    assert_eq!(
        transactions.free_invoke_ids(),
        0,
        "none sure to be the server's"
    );
    assert!(exhausted(&fresh(&transactions)), "held back for the head");
    held.push(client(&coordinator).expect("the client takes one"));
    assert!(head.served(), "and the head the other");
}

/// While the head is owed, other work goes once the server is two under its
/// reserve, and the head still finds the last of it.
#[test]
fn with_two_of_the_reserve_left_other_work_goes_and_the_head_takes_the_last() {
    let (coordinator, transactions) = seated();
    let mut server: Vec<_> = (0..NOTIFICATION_RESERVE)
        .map(|_| fresh(&transactions).expect("its reserve"))
        .collect();
    let mut held = client_fills(&coordinator);
    let mut head = waiter(&transactions, 0);
    assert!(head.poll().is_pending());
    coordinator.release(held.pop().unwrap()).unwrap();
    held.push(client(&coordinator).unwrap());
    assert!(!head.served(), "passed over");
    drop(server.pop());
    drop(server.pop());
    assert_eq!(transactions.free_invoke_ids(), 1, "two left, one owed");
    let _other = fresh(&transactions).expect("one to spare");
    assert!(client(&coordinator).is_err(), "the client at its ceiling");
    assert!(head.served());
}

/// The head passed over, the client lets three IDs go, then one of the
/// server's leases ends, waking the head and a COV report's owed look. The look runs first: its wait made, it reads no room (the head is
/// owed the last of the reserve) and waits, never trying a lease. As the
/// head takes its ID and leaves, the look is woken, and finds the three.
#[test]
fn a_look_that_read_no_room_while_the_head_was_owed_is_woken_as_it_leaves() {
    let (coordinator, transactions) = seated();
    let mut server: Vec<_> = (0..NOTIFICATION_RESERVE)
        .map(|_| fresh(&transactions).expect("its reserve"))
        .collect();
    let mut held = client_fills(&coordinator);
    let mut head = waiter(&transactions, 0);
    assert!(head.poll().is_pending());
    coordinator.release(held.pop().unwrap()).unwrap();
    held.push(client(&coordinator).unwrap());
    assert!(!head.served(), "passed over");
    for _ in 0..3 {
        coordinator.release(held.pop().unwrap()).unwrap();
    }
    drop(server.pop());
    let mut look = std::pin::pin!(transactions.released());
    look.as_mut().enable();
    assert_eq!(transactions.free_invoke_ids(), 0, "the head is owed");
    let noop = &mut Context::from_waker(Waker::noop());
    assert!(look.as_mut().poll(noop).is_pending());
    assert!(head.served());
    assert!(look.as_mut().poll(noop).is_ready(), "woken to look again");
    assert_eq!(transactions.free_invoke_ids(), 3);
    let _now = fresh(&transactions).expect("no longer held back");
}

/// A head that was never owed an ID wakes no wait as it leaves: only the
/// lease ends do.
#[test]
fn a_head_never_owed_wakes_no_wait_as_it_leaves() {
    let (coordinator, transactions) = seated();
    let mut server: Vec<_> = (0..NOTIFICATION_RESERVE)
        .map(|_| fresh(&transactions).expect("its reserve"))
        .collect();
    let _held = client_fills(&coordinator);
    let mut head = waiter(&transactions, 0);
    assert!(head.poll().is_pending());
    drop(server.pop());
    let mut wait = std::pin::pin!(transactions.released());
    wait.as_mut().enable();
    assert!(head.served(), "the server's own ID back");
    let noop = &mut Context::from_waker(Waker::noop());
    assert!(wait.as_mut().poll(noop).is_pending());
}
