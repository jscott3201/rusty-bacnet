//! A wait dropped before its slot gives the slot back (#2202): globally,
//! per network and under an overlay, and never once the slot's instant
//! has come; the meter doesn't count a slot given back.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Waker};

use super::tests::at;
use super::*;

/// A wait polled once, so it has booked its slot and sleeps until it.
type Wait<'a, T = ()> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Poll `wait` once and check it is waiting for its slot.
fn booked<T>(mut wait: Wait<'_, T>) -> Wait<'_, T> {
    let polled = wait.as_mut().poll(&mut Context::from_waker(Waker::noop()));
    assert!(polled.is_pending(), "the wait has a slot ahead");
    wait
}

/// `count` waits under the global cap alone, each booked.
fn global_waits(limits: &RequestLimits, count: usize) -> Vec<Wait<'_>> {
    (0..count)
        .map(|_| booked(Box::pin(limits.acquire())))
        .collect()
}

/// `count` waits for turns at distinct devices on `network`, each booked.
fn network_waits(limits: &RequestLimits, network: u16, count: u32) -> Vec<Wait<'_>> {
    (0..count)
        .map(|device| {
            booked(Box::pin(async move {
                drop(limits.turn(&at(network, device)).await)
            }))
        })
        .collect()
}

/// How long one more request under the global cap waits from now.
async fn next_global(limits: &RequestLimits) -> Duration {
    let start = Instant::now();
    limits.acquire().await;
    Instant::now() - start
}

/// How long one more request to `network` waits from now.
async fn next_on(limits: &RequestLimits, network: u16) -> Duration {
    let start = Instant::now();
    drop(limits.turn(&at(network, 99)).await);
    Instant::now() - start
}

/// Five waits booked and dropped, in any order, leave nothing booked: the
/// next request goes in the first slot after the one that went.
#[tokio::test(start_paused = true)]
async fn dropped_waits_give_their_slots_back() {
    let limits = RequestLimits::new(10);
    limits.acquire().await;
    let mut waits: Vec<Option<Wait<'_>>> = global_waits(&limits, 5).into_iter().map(Some).collect();
    for i in [2, 0, 4, 1, 3] {
        drop(waits[i].take());
    }
    assert_eq!(next_global(&limits).await, Duration::from_millis(100));
    assert_eq!(next_global(&limits).await, Duration::from_millis(100));
}

/// A slot given back between live waits goes to the next request, and
/// the live waits keep their own slots, in their order.
#[tokio::test(start_paused = true)]
async fn a_slot_given_back_between_live_waits_is_taken_first() {
    let limits = RequestLimits::new(10);
    let start = Instant::now();
    limits.acquire().await;
    let mut waits: Vec<Wait<'_, Duration>> = (0..3)
        .map(|_| {
            booked(Box::pin(async {
                limits.acquire().await;
                Instant::now() - start
            }) as Wait<'_, Duration>)
        })
        .collect();
    let last = waits.pop().unwrap();
    let middle = waits.pop().unwrap();
    drop(waits);
    assert_eq!(next_global(&limits).await, Duration::from_millis(100));
    assert_eq!(middle.await, Duration::from_millis(200));
    assert_eq!(last.await, Duration::from_millis(300));
}

/// The same for a network's rate: waits for its slots dropped, the next
/// request to it goes within one of its intervals.
#[tokio::test(start_paused = true)]
async fn dropped_waits_give_back_their_network_slots() {
    let limits = RequestLimits::new(0).with_network_rates(20, [(2001, 2)]);
    drop(limits.turn(&at(2001, 50)).await);
    drop(network_waits(&limits, 2001, 4));
    assert_eq!(next_on(&limits, 2001).await, Duration::from_millis(500));
}

/// Under an overlay, per network and globally; and slots booked at the
/// overlay's slower spacing, dropped after it is switched off, leave no
/// backlog for the limits' own requests.
#[tokio::test(start_paused = true)]
async fn waits_under_an_overlay_give_their_slots_back() {
    let limits = RequestLimits::new(50);
    let overlay = Overlay {
        global: 0,
        network: 0,
        rates: vec![(2001, 1)],
    };
    limits.set_overlay(Some(&overlay));
    drop(limits.turn(&at(2001, 50)).await);
    drop(network_waits(&limits, 2001, 3));
    assert_eq!(next_on(&limits, 2001).await, Duration::from_secs(1));

    limits.set_overlay(Some(&Overlay {
        global: 2,
        ..overlay
    }));
    tokio::time::sleep(Duration::from_secs(2)).await;
    limits.acquire().await;
    let waits = global_waits(&limits, 4);
    limits.set_overlay(None);
    drop(waits);
    assert_eq!(
        next_global(&limits).await,
        Duration::from_millis(500),
        "behind the one request that went at the overlay's spacing, not the four dropped"
    );
}

/// A wait dropped once its slot's instant has come frees nothing: its
/// request may have gone.
#[tokio::test(start_paused = true)]
async fn a_wait_dropped_after_its_instant_frees_nothing() {
    let limits = RequestLimits::new(10);
    limits.acquire().await;
    let wait = booked(Box::pin(limits.acquire()));
    tokio::time::advance(Duration::from_millis(150)).await;
    drop(wait);
    assert_eq!(next_global(&limits).await, Duration::from_millis(50));
}

/// The meter counts the requests that went, not slots given back.
#[tokio::test(start_paused = true)]
async fn the_meter_leaves_out_slots_given_back() {
    let limits = RequestLimits::new(10);
    limits.acquire().await;
    drop(global_waits(&limits, 3));
    limits.acquire().await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(limits.recent(), 2);
}

/// A charged request whose token came at once, withdrawn while it waits
/// for its device's gate or for a slot, gives the token back: the
/// bucket's next take goes at once.
#[tokio::test(start_paused = true)]
async fn a_request_withdrawn_after_its_token_gives_it_back() {
    let fresh = || {
        let bucket = Arc::new(TokenBucket::new(1, 1));
        (Arc::clone(&bucket), Arc::new(Account::new(Some(bucket))))
    };
    let at_once = |bucket: &TokenBucket| {
        let mut take = std::pin::pin!(bucket.take());
        take.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    };

    let limits = RequestLimits::new(0);
    let (bucket, account) = fresh();
    let busy = limits.turn(&at(0, 1)).await;
    let wait = booked(Box::pin(charged(Arc::clone(&account), async {
        drop(limits.turn(&at(0, 1)).await);
    })));
    drop(wait);
    drop(busy);
    assert!(at_once(&bucket), "given back while waiting for the gate");

    let limits = RequestLimits::new(1);
    let (bucket, account) = fresh();
    limits.acquire().await;
    let wait = booked(Box::pin(charged(account, limits.slot(None))));
    drop(wait);
    assert!(at_once(&bucket), "given back while waiting for a slot");
}

/// Waiting until a request could go counts a slot given back: on the
/// paused clock, which wakes exactly at the hole, it ends there, not after
/// the slots still booked.
#[tokio::test(start_paused = true)]
async fn waiting_until_free_counts_a_slot_given_back() {
    let limits = RequestLimits::new(10);
    let start = Instant::now();
    limits.acquire().await;
    let mut waits = global_waits(&limits, 3);
    drop(waits.remove(1));
    limits.wait_free().await;
    assert_eq!(Instant::now() - start, Duration::from_millis(200));
    drop(waits);
}
