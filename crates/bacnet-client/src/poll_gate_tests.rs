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

/// On worker threads, tasks taking the gate in turn never overlap, and
/// none is left waiting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tasks_on_threads_hold_the_gate_one_at_a_time() {
    let gate = Arc::new(PollGate::default());
    let inside = Arc::new(AtomicBool::new(false));
    let passes = Arc::new(AtomicUsize::new(0));
    let tasks: Vec<_> = (0..8)
        .map(|_| {
            let (gate, inside, passes) =
                (Arc::clone(&gate), Arc::clone(&inside), Arc::clone(&passes));
            tokio::spawn(async move {
                for _ in 0..200 {
                    let guard = Arc::clone(&gate).lock_owned().await;
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
    assert_eq!(passes.load(Ordering::SeqCst), 1600);
    assert_eq!(waiting(&gate), 0);
}
