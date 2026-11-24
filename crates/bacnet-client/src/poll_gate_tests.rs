//! The gate's rules, polled by hand with wakers that record their wakes.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::Wake;

use super::*;

/// A waker that counts its wakes and logs its name to a shared log as it
/// is woken.
struct Recorder {
    name: char,
    wakes: AtomicUsize,
    log: Arc<Mutex<Vec<char>>>,
}

impl Wake for Recorder {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        self.log.lock().unwrap().push(self.name);
    }
}

/// A waiter, its waker, and the wake log it shares.
struct Caller<'g> {
    take: Pin<Box<Take<&'g PollGate>>>,
    waker: Arc<Recorder>,
}

impl<'g> Caller<'g> {
    fn new(gate: &'g PollGate, name: char, log: &Arc<Mutex<Vec<char>>>) -> Self {
        Self {
            take: Box::pin(gate.lock()),
            waker: Arc::new(Recorder {
                name,
                wakes: AtomicUsize::new(0),
                log: Arc::clone(log),
            }),
        }
    }

    fn poll(&mut self) -> Poll<PollGateGuard<&'g PollGate>> {
        let waker = Waker::from(Arc::clone(&self.waker));
        self.take.as_mut().poll(&mut Context::from_waker(&waker))
    }

    fn wakes(&self) -> usize {
        self.waker.wakes.load(Ordering::SeqCst)
    }
}

fn waiting(gate: &PollGate) -> usize {
    gate.state().waiters.len()
}

/// A free gate is taken at once and released on drop; `try_lock` fails
/// while it is held.
#[test]
fn a_free_gate_is_taken_at_once() {
    let gate = PollGate::default();
    let log = Arc::default();
    let mut a = Caller::new(&gate, 'a', &log);
    let Poll::Ready(guard) = a.poll() else {
        panic!("a free gate is taken")
    };
    assert!(gate.try_lock().is_none());
    drop(guard);
    assert!(gate.try_lock().is_some());
    assert_eq!(waiting(&gate), 0);
}

/// A waiter polled once and never again holds the gate for nobody: when
/// the holder lets go, a polled waiter steps aside once for it, then takes
/// the gate. The parked one, polled again, sleeps until that one lets go,
/// and then takes it.
#[test]
fn a_waiter_never_polled_again_holds_the_gate_for_nobody() {
    let gate = PollGate::default();
    let log = Arc::default();
    let holder = gate.try_lock().unwrap();
    let mut parked = Caller::new(&gate, 'p', &log);
    let mut polled = Caller::new(&gate, 'q', &log);
    assert!(parked.poll().is_pending());
    assert!(polled.poll().is_pending());
    drop(holder);
    assert_eq!(*log.lock().unwrap(), ['p', 'q'], "woken oldest first");
    assert!(polled.poll().is_pending(), "steps aside for the older");
    assert_eq!(polled.wakes(), 2, "and wakes itself");
    let Poll::Ready(guard) = polled.poll() else {
        panic!("then takes the gate")
    };
    assert!(parked.poll().is_pending());
    drop(guard);
    assert!(matches!(parked.poll(), Poll::Ready(_)));
    assert_eq!(waiting(&gate), 0);
}

/// Waiters polled as they are woken go in the order they first waited,
/// and a newcomer polled while the gate is free and woken waiters are
/// still to poll steps aside for them.
#[test]
fn woken_waiters_go_in_turn_and_a_newcomer_steps_aside() {
    let gate = PollGate::default();
    let log = Arc::default();
    let holder = gate.try_lock().unwrap();
    let mut a = Caller::new(&gate, 'a', &log);
    let mut b = Caller::new(&gate, 'b', &log);
    assert!(a.poll().is_pending());
    assert!(b.poll().is_pending());
    drop(holder);
    let mut newcomer = Caller::new(&gate, 'n', &log);
    assert!(newcomer.poll().is_pending(), "steps aside");
    let Poll::Ready(first) = a.poll() else {
        panic!("the oldest goes first")
    };
    assert!(b.poll().is_pending());
    assert!(newcomer.poll().is_pending());
    log.lock().unwrap().clear();
    drop(first);
    assert_eq!(*log.lock().unwrap(), ['b', 'n']);
    assert!(newcomer.poll().is_pending(), "b waited first");
    let Poll::Ready(second) = b.poll() else {
        panic!("b goes next")
    };
    assert!(newcomer.poll().is_pending());
    drop(second);
    assert!(matches!(newcomer.poll(), Poll::Ready(_)));
}

/// A waiter that leaves is forgotten and passes nothing on: everyone
/// asleep is woken at each release anyway. A gate let go with nobody
/// waiting wakes nobody.
#[test]
fn a_waiter_that_leaves_is_forgotten() {
    let gate = PollGate::default();
    let log = Arc::default();
    let holder = gate.try_lock().unwrap();
    let mut a = Caller::new(&gate, 'a', &log);
    let mut b = Caller::new(&gate, 'b', &log);
    assert!(a.poll().is_pending());
    assert!(b.poll().is_pending());
    drop(a);
    assert_eq!(waiting(&gate), 1);
    drop(holder);
    assert_eq!(*log.lock().unwrap(), ['b']);
    assert!(matches!(b.poll(), Poll::Ready(_)), "the oldest left");
    assert_eq!(waiting(&gate), 0);
    let quiet = gate.try_lock().unwrap();
    drop(quiet);
    assert_eq!(log.lock().unwrap().len(), 1, "nobody woken");
}

/// A waiter that sleeps again keeps its place: polled with a new waker,
/// it is woken with that one, and still goes before a later waiter.
#[test]
fn a_waiter_keeps_its_place_and_its_latest_waker() {
    let gate = PollGate::default();
    let log = Arc::default();
    let holder = gate.try_lock().unwrap();
    let mut a = Caller::new(&gate, 'a', &log);
    assert!(a.poll().is_pending());
    let mut b = Caller::new(&gate, 'b', &log);
    assert!(b.poll().is_pending());
    a.waker = Arc::new(Recorder {
        name: 'A',
        wakes: AtomicUsize::new(0),
        log: Arc::clone(&log),
    });
    assert!(a.poll().is_pending());
    drop(holder);
    assert_eq!(*log.lock().unwrap(), ['A', 'b']);
    assert!(b.poll().is_pending());
    assert!(matches!(a.poll(), Poll::Ready(_)));
}

/// The oldest waiter dropped after a release, before it polled, holds
/// the next one up not at all: that one is now the oldest and takes the
/// gate on its first poll, without stepping aside.
#[test]
fn a_woken_oldest_waiter_that_leaves_holds_nobody_up() {
    let gate = PollGate::default();
    let log = Arc::default();
    let holder = gate.try_lock().unwrap();
    let mut a = Caller::new(&gate, 'a', &log);
    let mut b = Caller::new(&gate, 'b', &log);
    assert!(a.poll().is_pending());
    assert!(b.poll().is_pending());
    drop(holder);
    drop(a);
    assert!(matches!(b.poll(), Poll::Ready(_)));
    assert_eq!(b.wakes(), 1, "woken by the release alone");
    assert_eq!(waiting(&gate), 0);
}

/// A waiter dropped while it stepped aside is forgotten: the parked waiter
/// it stepped aside for is still the oldest, and a later newcomer steps
/// aside once for that one alone, then takes the gate.
#[test]
fn a_waiter_dropped_while_it_stepped_aside_is_forgotten() {
    let gate = PollGate::default();
    let log = Arc::default();
    let holder = gate.try_lock().unwrap();
    let mut parked = Caller::new(&gate, 'p', &log);
    assert!(parked.poll().is_pending());
    drop(holder);
    let mut gone = Caller::new(&gate, 'g', &log);
    assert!(gone.poll().is_pending(), "steps aside for the parked one");
    assert_eq!(waiting(&gate), 2);
    drop(gone);
    assert_eq!(waiting(&gate), 1);
    let mut next = Caller::new(&gate, 'n', &log);
    assert!(next.poll().is_pending(), "steps aside once");
    let Poll::Ready(guard) = next.poll() else {
        panic!("then takes the gate")
    };
    drop(guard);
    assert!(matches!(parked.poll(), Poll::Ready(_)));
    assert_eq!(waiting(&gate), 0);
}

/// A holder that panics lets the gate go as it unwinds, and wakes the
/// waiter, which then takes it.
#[test]
fn a_holder_that_panics_lets_the_gate_go() {
    let gate = PollGate::default();
    let log = Arc::default();
    let mut waiter = Caller::new(&gate, 'w', &log);
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = gate.try_lock().unwrap();
        assert!(waiter.poll().is_pending());
        panic!("the holder panics");
    }));
    assert!(unwound.is_err());
    assert_eq!(waiter.wakes(), 1);
    assert!(matches!(waiter.poll(), Poll::Ready(_)));
}

/// `try_lock` takes a free gate ahead of a woken waiter still to poll: it
/// never waits. The waiter then sleeps until it lets go.
#[test]
fn try_lock_takes_a_free_gate_ahead_of_a_woken_waiter() {
    let gate = PollGate::default();
    let log = Arc::default();
    let holder = gate.try_lock().unwrap();
    let mut waiter = Caller::new(&gate, 'w', &log);
    assert!(waiter.poll().is_pending());
    drop(holder);
    assert_eq!(waiter.wakes(), 1);
    let taken = gate.try_lock().expect("the gate is free");
    assert!(waiter.poll().is_pending());
    drop(taken);
    assert_eq!(waiter.wakes(), 2);
    assert!(matches!(waiter.poll(), Poll::Ready(_)));
}

/// On worker threads, tasks taking the gate in turn never overlap, though
/// some give up their waits by a timeout and one waiter is parked, never
/// polled again, for the whole run. Every task gets its turns, and at the
/// end only the parked waiter is left waiting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tasks_on_threads_hold_the_gate_one_at_a_time() {
    let gate = Arc::new(PollGate::default());
    let holder = gate.try_lock().unwrap();
    let mut parked = Box::pin(Arc::clone(&gate).lock_owned());
    assert!(
        parked
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(holder);
    let inside = Arc::new(AtomicBool::new(false));
    let passes = Arc::new(AtomicUsize::new(0));
    let given_up = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<_> = (0..8)
        .map(|task| {
            let (gate, inside) = (Arc::clone(&gate), Arc::clone(&inside));
            let (passes, given_up) = (Arc::clone(&passes), Arc::clone(&given_up));
            tokio::spawn(async move {
                for _ in 0..200 {
                    let wait = Arc::clone(&gate).lock_owned();
                    let guard = if task % 2 == 0 {
                        wait.await
                    } else {
                        let limit = std::time::Duration::from_micros(50);
                        match tokio::time::timeout(limit, wait).await {
                            Ok(guard) => guard,
                            Err(_) => {
                                given_up.fetch_add(1, Ordering::SeqCst);
                                continue;
                            }
                        }
                    };
                    assert!(!inside.swap(true, Ordering::SeqCst), "one holder");
                    tokio::task::yield_now().await;
                    inside.store(false, Ordering::SeqCst);
                    passes.fetch_add(1, Ordering::SeqCst);
                    drop(guard);
                }
            })
        })
        .collect();
    for task in tasks {
        tokio::time::timeout(std::time::Duration::from_secs(30), task)
            .await
            .expect("every task gets its turns")
            .unwrap();
    }
    let (passes, given_up) = (
        passes.load(Ordering::SeqCst),
        given_up.load(Ordering::SeqCst),
    );
    assert_eq!(passes + given_up, 1600);
    assert!(
        passes >= 800,
        "the tasks that never give up pass every turn"
    );
    assert_eq!(waiting(&gate), 1, "only the parked waiter");
    drop(parked);
    assert_eq!(waiting(&gate), 0);
    assert!(gate.try_lock().is_some());
}
