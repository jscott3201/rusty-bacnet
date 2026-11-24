//! The one-request-at-a-time gate of a routed path is taken only by a
//! request as it is polled (#2759). Tokio's fair mutex, which it was, hands
//! its lock to the first queued waiter whether or not anything polls that
//! waiter again, so a request parked in a `buffer_unordered` stream or a
//! `select!` arm held the path for every request behind it. These requests
//! are polled by hand, or parked in a stream whose loop body sends through
//! the same router; requests that are all polled still go in the order
//! they first waited.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

use futures_util::StreamExt;
use tokio::time::timeout;

use super::*;
use crate::client::routed_path_limit_tests::{
    DNET, confirmed_request, harness, inject_simple_ack, routed_request,
};

const ROUTER: [u8; 1] = [2];
const DADR: [u8; 1] = [3];
/// How long a test waits for what it expects, in paused time.
const WAIT: Duration = Duration::from_secs(60);

/// Poll `future` once, with a waker that wakes nothing: its caller then
/// stops polling it.
fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

/// The fill octet of the routed write a frame carries.
fn fill(frame: &bacnet_encoding::apdu::ConfirmedRequest) -> u8 {
    frame.service_request[0]
}

/// A waiter polled once and never again, while another request holds the
/// path, holds it for nobody once that request lets go: a waiter that is
/// polled takes the path, and the parked one, polled again later, takes it
/// after that one.
#[tokio::test(start_paused = true)]
async fn a_waiter_whose_caller_stopped_polling_holds_the_path_for_nobody() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(
        4,
        Duration::from_millis(10),
    ));
    let held = limits.acquire(&ROUTER, DNET).await.unwrap();
    let mut parked = Box::pin(limits.acquire(&ROUTER, DNET));
    assert!(poll_once(parked.as_mut()).is_pending());
    let polled = tokio::spawn({
        let limits = Arc::clone(&limits);
        async move { limits.acquire(&ROUTER, DNET).await.map(drop) }
    });
    tokio::task::yield_now().await;
    drop(held);
    timeout(WAIT, polled)
        .await
        .expect("the polled waiter takes the path")
        .unwrap()
        .unwrap();
    assert!(
        timeout(WAIT, parked)
            .await
            .expect("then the parked one")
            .is_ok()
    );
}

/// A loop that drains a `buffer_unordered` stream of routed writes through
/// one router, and writes through it again in its body: the body's write
/// goes ahead of the buffered write nothing polls while the body runs, and
/// the loop ends. It never ended when the path went to that buffered write.
#[tokio::test(start_paused = true)]
async fn a_loop_body_sends_through_the_path_its_buffered_requests_wait_for() {
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap(),
    );
    let drain = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            let write = |fill: u8| {
                let client = Arc::clone(&client);
                async move {
                    client
                        .confirmed_request(
                            &Station::routed(&ROUTER, DNET, &DADR),
                            ConfirmedServiceChoice::WRITE_PROPERTY,
                            &[fill],
                        )
                        .await
                }
            };
            let mut writes = futures_util::stream::iter([1, 2])
                .map(write)
                .buffer_unordered(2);
            let mut bodies = 0;
            while let Some(written) = writes.next().await {
                written.unwrap();
                write(0x10 + bodies).await.unwrap();
                bodies += 1;
            }
            bodies
        }
    });
    let mut sent = Vec::new();
    for _ in 0..4 {
        let frame = timeout(WAIT, outbound.recv())
            .await
            .expect("the next write goes")
            .unwrap();
        let request = confirmed_request(frame, &ROUTER);
        sent.push(fill(&request));
        inject_simple_ack(&inbound, &ROUTER, DNET, &DADR, request.invoke_id).await;
    }
    assert_eq!(sent, [1, 0x10, 2, 0x11]);
    assert_eq!(timeout(WAIT, drain).await.unwrap().unwrap(), 2);
}

/// Requests to one path whose callers all poll them go one at a time, in
/// the order they first waited, as with the fair mutex.
#[tokio::test(start_paused = true)]
async fn polled_requests_go_in_the_order_they_waited() {
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap(),
    );
    let request = |fill: u8| {
        routed_request(
            Arc::clone(&client),
            ROUTER.to_vec(),
            DNET,
            DADR.to_vec(),
            vec![fill],
        )
    };
    let mut tasks = vec![request(1)];
    let first = confirmed_request(outbound.recv().await.unwrap(), &ROUTER);
    for fill in 2..=5 {
        tasks.push(request(fill));
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let mut next = first;
    let mut sent = vec![fill(&next)];
    for _ in 2..=5 {
        inject_simple_ack(&inbound, &ROUTER, DNET, &DADR, next.invoke_id).await;
        next = confirmed_request(
            timeout(WAIT, outbound.recv()).await.unwrap().unwrap(),
            &ROUTER,
        );
        sent.push(fill(&next));
    }
    inject_simple_ack(&inbound, &ROUTER, DNET, &DADR, next.invoke_id).await;
    assert_eq!(sent, [1, 2, 3, 4, 5]);
    for task in tasks {
        assert!(timeout(WAIT, task).await.unwrap().unwrap().is_ok());
    }
}

/// A task that sends through the path back to back doesn't keep out a
/// request that waited first: when its request ends, its next one finds
/// the path free and the other request woken, and steps aside for it.
#[tokio::test(start_paused = true)]
async fn back_to_back_requests_let_a_waiting_one_go_first() {
    let (transport, inbound, mut outbound) = harness(&[1], 1486);
    let client = Arc::new(
        BACnetClient::start(ClientConfig::default(), transport)
            .await
            .unwrap(),
    );
    let looping = tokio::spawn({
        let client = Arc::clone(&client);
        async move {
            for fill in [1, 3] {
                client
                    .confirmed_request(
                        &Station::routed(&ROUTER, DNET, &DADR),
                        ConfirmedServiceChoice::WRITE_PROPERTY,
                        &[fill],
                    )
                    .await
                    .unwrap();
            }
        }
    });
    let mut next = confirmed_request(outbound.recv().await.unwrap(), &ROUTER);
    let waiting = routed_request(
        Arc::clone(&client),
        ROUTER.to_vec(),
        DNET,
        DADR.to_vec(),
        vec![2],
    );
    tokio::time::sleep(Duration::from_millis(1)).await;
    let mut sent = vec![fill(&next)];
    for _ in 0..2 {
        inject_simple_ack(&inbound, &ROUTER, DNET, &DADR, next.invoke_id).await;
        next = confirmed_request(
            timeout(WAIT, outbound.recv()).await.unwrap().unwrap(),
            &ROUTER,
        );
        sent.push(fill(&next));
    }
    inject_simple_ack(&inbound, &ROUTER, DNET, &DADR, next.invoke_id).await;
    assert_eq!(sent, [1, 2, 3]);
    timeout(WAIT, looping).await.unwrap().unwrap();
    assert!(timeout(WAIT, waiting).await.unwrap().unwrap().is_ok());
}
