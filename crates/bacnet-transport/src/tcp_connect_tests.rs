//! Address ordering and racing, on paused time with scripted attempts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

use tokio::time::{sleep, Instant};

use super::{interleave, race, CONNECTION_ATTEMPT_DELAY};

const V6_A: &str = "[::1]:47808";
const V6_B: &str = "[2001:db8::1]:47808";
const V4_A: &str = "127.0.0.1:47808";
const V4_B: &str = "192.0.2.1:47808";

fn addr(text: &str) -> SocketAddr {
    text.parse().unwrap()
}

fn addrs(texts: &[&str]) -> Vec<SocketAddr> {
    texts.iter().map(|text| addr(text)).collect()
}

/// What a scripted attempt does.
#[derive(Clone, Copy)]
enum Plan {
    Connect(Duration),
    Fail(Duration, io::ErrorKind),
    Hang,
}

/// Scripted attempts that record when each one started and whether a pending
/// attempt was dropped.
#[derive(Clone, Default)]
struct Script {
    plans: HashMap<SocketAddr, Plan>,
    started: Rc<RefCell<Vec<(SocketAddr, Duration)>>>,
    dropped: Rc<RefCell<Vec<SocketAddr>>>,
}

struct DropFlag(SocketAddr, Rc<RefCell<Vec<SocketAddr>>>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

impl Script {
    fn new(plans: &[(&str, Plan)]) -> Self {
        Self {
            plans: plans.iter().map(|(a, plan)| (addr(a), *plan)).collect(),
            ..Self::default()
        }
    }

    async fn run(&self, order: &[&str]) -> io::Result<SocketAddr> {
        let base = Instant::now();
        race(&addrs(order), CONNECTION_ATTEMPT_DELAY, |addr| {
            let plan = self.plans[&addr];
            self.started.borrow_mut().push((addr, base.elapsed()));
            let flag = DropFlag(addr, self.dropped.clone());
            async move {
                let result = match plan {
                    Plan::Connect(after) => {
                        sleep(after).await;
                        Ok(addr)
                    }
                    Plan::Fail(after, kind) => {
                        sleep(after).await;
                        Err(io::Error::new(kind, format!("scripted {addr}")))
                    }
                    Plan::Hang => std::future::pending().await,
                };
                // Only an attempt that finishes disarms its flag.
                std::mem::forget(flag);
                result
            }
        })
        .await
    }

    fn started(&self) -> Vec<(SocketAddr, Duration)> {
        self.started.borrow().clone()
    }
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

#[test]
fn interleave_alternates_families_from_the_first_result() {
    assert_eq!(
        interleave(addrs(&[V6_A, V6_B, V4_A, V4_B])),
        addrs(&[V6_A, V4_A, V6_B, V4_B])
    );
    assert_eq!(
        interleave(addrs(&[V4_A, V6_A, V6_B, V4_A])),
        addrs(&[V4_A, V6_A, V6_B])
    );
    assert_eq!(interleave(addrs(&[V6_A, V6_B])), addrs(&[V6_A, V6_B]));
    assert!(interleave(Vec::new()).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_silent_attempt_gets_the_delay_before_the_next_family_starts() {
    let script = Script::new(&[(V6_A, Plan::Hang), (V4_A, Plan::Connect(ms(10)))]);
    let base = Instant::now();
    assert_eq!(script.run(&[V6_A, V4_A]).await.unwrap(), addr(V4_A));
    assert_eq!(
        script.started(),
        vec![(addr(V6_A), ms(0)), (addr(V4_A), CONNECTION_ATTEMPT_DELAY)]
    );
    assert_eq!(base.elapsed(), CONNECTION_ATTEMPT_DELAY + ms(10));
    // The winner aborts the attempt still in progress.
    assert_eq!(*script.dropped.borrow(), vec![addr(V6_A)]);
}

#[tokio::test(start_paused = true)]
async fn a_failed_attempt_starts_the_next_at_once() {
    let script = Script::new(&[
        (V6_A, Plan::Fail(ms(5), io::ErrorKind::ConnectionRefused)),
        (V4_A, Plan::Connect(ms(1))),
    ]);
    assert_eq!(script.run(&[V6_A, V4_A]).await.unwrap(), addr(V4_A));
    assert_eq!(
        script.started(),
        vec![(addr(V6_A), ms(0)), (addr(V4_A), ms(5))]
    );
}

#[tokio::test(start_paused = true)]
async fn each_start_restarts_the_delay() {
    let script = Script::new(&[
        (V6_A, Plan::Hang),
        (V4_A, Plan::Fail(ms(100), io::ErrorKind::ConnectionRefused)),
        (V6_B, Plan::Hang),
        (V4_B, Plan::Connect(ms(1))),
    ]);
    assert_eq!(
        script.run(&[V6_A, V4_A, V6_B, V4_B]).await.unwrap(),
        addr(V4_B)
    );
    assert_eq!(
        script.started(),
        vec![
            (addr(V6_A), ms(0)),
            (addr(V4_A), ms(250)),
            (addr(V6_B), ms(350)),
            (addr(V4_B), ms(600)),
        ]
    );
    let mut dropped = script.dropped.borrow().clone();
    dropped.sort();
    assert_eq!(dropped, addrs(&[V6_A, V6_B]));
}

#[tokio::test(start_paused = true)]
async fn an_answer_within_the_delay_starts_nothing_else() {
    let script = Script::new(&[(V6_A, Plan::Connect(ms(100))), (V4_A, Plan::Hang)]);
    assert_eq!(script.run(&[V6_A, V4_A]).await.unwrap(), addr(V6_A));
    assert_eq!(script.started(), vec![(addr(V6_A), ms(0))]);
}

#[tokio::test(start_paused = true)]
async fn a_slow_first_answer_still_wins_over_a_later_start() {
    let script = Script::new(&[(V6_A, Plan::Connect(ms(300))), (V4_A, Plan::Hang)]);
    assert_eq!(script.run(&[V6_A, V4_A]).await.unwrap(), addr(V6_A));
    assert_eq!(
        script.started(),
        vec![(addr(V6_A), ms(0)), (addr(V4_A), ms(250))]
    );
    assert_eq!(*script.dropped.borrow(), vec![addr(V4_A)]);
}

#[tokio::test(start_paused = true)]
async fn a_failure_of_an_earlier_attempt_also_starts_the_next() {
    let script = Script::new(&[
        (V6_A, Plan::Fail(ms(300), io::ErrorKind::ConnectionRefused)),
        (V4_A, Plan::Hang),
        (V6_B, Plan::Connect(ms(1))),
    ]);
    assert_eq!(script.run(&[V6_A, V4_A, V6_B]).await.unwrap(), addr(V6_B));
    // B started at the delay; A's failure at 300 ms starts C at once, not at
    // 500 ms.
    assert_eq!(
        script.started(),
        vec![
            (addr(V6_A), ms(0)),
            (addr(V4_A), ms(250)),
            (addr(V6_B), ms(300)),
        ]
    );
    assert_eq!(*script.dropped.borrow(), vec![addr(V4_A)]);
}

fn assert_names_both(error: &io::Error) {
    let message = error.to_string();
    for a in [V6_A, V4_A] {
        assert!(message.contains(&format!("{a}: scripted {a}")), "{message}");
    }
}

#[tokio::test(start_paused = true)]
async fn a_refusal_gives_the_kind_even_after_a_quicker_unreachable() {
    let script = Script::new(&[
        (V6_A, Plan::Fail(ms(5), io::ErrorKind::NetworkUnreachable)),
        (V4_A, Plan::Fail(ms(10), io::ErrorKind::ConnectionRefused)),
    ]);
    let error = script.run(&[V6_A, V4_A]).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
    assert_names_both(&error);
}

#[tokio::test(start_paused = true)]
async fn a_timeout_gives_the_kind_when_nothing_refused() {
    let script = Script::new(&[
        (V6_A, Plan::Fail(ms(5), io::ErrorKind::HostUnreachable)),
        (V4_A, Plan::Fail(ms(10), io::ErrorKind::TimedOut)),
    ]);
    let error = script.run(&[V6_A, V4_A]).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_names_both(&error);
}

#[tokio::test(start_paused = true)]
async fn otherwise_the_first_failure_gives_the_kind() {
    let script = Script::new(&[
        (V6_A, Plan::Fail(ms(5), io::ErrorKind::NetworkUnreachable)),
        (V4_A, Plan::Fail(ms(1), io::ErrorKind::AddrNotAvailable)),
    ]);
    let error = script.run(&[V6_A, V4_A]).await.unwrap_err();
    // V6_A fails at 5 ms and starts V4_A, which fails at 6 ms.
    assert_eq!(error.kind(), io::ErrorKind::NetworkUnreachable);
    assert_names_both(&error);
}

#[tokio::test(start_paused = true)]
async fn a_single_address_failure_is_returned_unchanged() {
    let script = Script::new(&[(V4_A, Plan::Fail(ms(1), io::ErrorKind::ConnectionRefused))]);
    let error = script.run(&[V4_A]).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionRefused);
    assert_eq!(error.to_string(), format!("scripted {V4_A}"));
}

#[tokio::test(start_paused = true)]
async fn no_address_is_invalid_input() {
    let script = Script::default();
    let error = script.run(&[]).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
}

#[tokio::test]
async fn connect_reaches_a_listener_through_resolution() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = listener.local_addr().unwrap();
    let host_port = target.to_string();
    let addrs = super::resolve(&host_port).await.unwrap();
    let (dialed, accepted) = tokio::join!(super::connect_resolved(&addrs), listener.accept());
    let (dialed, (accepted, peer)) = (dialed.unwrap(), accepted.unwrap());
    assert_eq!(dialed.peer_addr().unwrap(), target);
    assert_eq!(accepted.local_addr().unwrap(), target);
    assert_eq!(dialed.local_addr().unwrap(), peer);
}
