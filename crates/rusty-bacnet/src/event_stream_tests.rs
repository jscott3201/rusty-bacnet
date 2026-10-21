use super::*;
use std::ffi::{CStr, CString};
use std::time::Duration;

use pyo3::types::PyDict;

/// The next item, which must come from the channel, its delivery settled
/// as a delivered one.
async fn fresh<T: Clone + Send + 'static>(stream: &LossyReceiver<T>) -> Option<T> {
    let (next, hook) = stream.next();
    let item = next.await.map(|next| match next {
        Next::Fresh(item) => item,
        Next::Again(_) => panic!("nothing was put back"),
    });
    hook.settled();
    item
}

fn in_flight<T>(stream: &LossyReceiver<T>) -> usize {
    stream.slot.in_flight.load(Ordering::Acquire)
}

#[tokio::test]
async fn a_lagging_receiver_counts_the_gap_and_never_holds_up_the_sender() {
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    // Nobody reads while ten items go out: each send returns at once.
    for item in 0..10u32 {
        tx.send(item).unwrap();
    }
    assert_eq!(stream.missed(), 0);
    // The six oldest are gone; the stream picks up at the oldest held.
    assert_eq!(fresh(&stream).await, Some(6));
    assert_eq!(stream.missed(), 6);
    for expected in 7..10 {
        assert_eq!(fresh(&stream).await, Some(expected));
    }
    assert_eq!(stream.missed(), 6);
}

#[tokio::test]
async fn a_cancelled_next_loses_nothing_and_the_end_follows_the_last_item() {
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    // A next() given up before anything came, as a cancelled __anext__.
    assert!(
        tokio::time::timeout(Duration::from_millis(20), stream.next().0)
            .await
            .is_err()
    );
    // Given up before it armed, it left nothing in flight and lost nothing.
    assert_eq!(in_flight(&stream), 0);
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    drop(tx);
    assert_eq!(fresh(&stream).await, Some(1));
    assert_eq!(fresh(&stream).await, Some(2));
    assert_eq!(fresh(&stream).await, None);
    assert_eq!(fresh(&stream).await, None);
    assert_eq!(stream.missed(), 0);
}

#[tokio::test]
async fn a_pending_next_ends_when_the_last_sender_drops() {
    let (tx, rx) = broadcast::channel::<u32>(4);
    let stream = LossyReceiver::new(rx);
    let pending = tokio::spawn(stream.next().0);
    tokio::task::yield_now().await;
    drop(tx);
    let ended = tokio::time::timeout(Duration::from_secs(5), pending).await;
    assert!(ended.unwrap().unwrap().is_none());
}

/// An item handed out and then dropped on its way, its delivery never
/// settled, counts as missed and still lets the next `next()` through.
#[tokio::test]
async fn a_delivery_dropped_unsettled_counts_its_item_and_leaves_nothing_in_flight() {
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    let (next, hook) = stream.next();
    assert!(matches!(next.await, Some(Next::Fresh(1))));
    assert_eq!(in_flight(&stream), 1);
    drop(hook);
    assert_eq!(in_flight(&stream), 0);
    assert_eq!(stream.missed(), 1);
    let second = tokio::time::timeout(Duration::from_secs(5), fresh(&stream)).await;
    assert_eq!(second.unwrap(), Some(2));
    assert_eq!(stream.missed(), 1);
}

/// While a delivery is in flight the next `next()` waits, even with the
/// next item already in the channel; the put-back item then comes first.
#[tokio::test]
async fn a_put_back_item_comes_before_a_later_one_already_queued() {
    Python::initialize();
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    let (first, first_hook) = stream.next();
    assert!(matches!(first.await, Some(Next::Fresh(1))));
    assert_eq!(in_flight(&stream), 1);
    let (second, second_hook) = stream.next();
    let second = tokio::spawn(second);
    // Every turn the second read gets, it finds the first still in flight.
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    assert!(!second.is_finished(), "read past an item still in flight");
    assert_eq!(in_flight(&stream), 1);
    Python::attach(|py| {
        first_hook.put_back(py, 1u32.into_py_any(py).unwrap(), Via::OnLoop);
        first_hook.settled();
    });
    let again = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .unwrap()
        .unwrap();
    match again {
        Some(Next::Again(object)) => {
            Python::attach(|py| assert_eq!(object.extract::<u32>(py).unwrap(), 1));
        }
        _ => panic!("expected the put-back item"),
    }
    // Delivered: until its delivery settles, the next read waits behind it.
    second_hook.settled();
    assert_eq!(fresh(&stream).await, Some(2));
    assert_eq!(stream.missed(), 0);
    let trace = &stream.slot.trace;
    assert_eq!(trace.on_loop.load(Ordering::Relaxed), 1);
    assert_eq!(trace.delivered.load(Ordering::Relaxed), 2);
}

/// A read held up behind an unsettled delivery past the limit warns once
/// and keeps waiting: the item ahead still comes first.
#[tokio::test]
async fn a_read_held_up_past_the_limit_warns_once_and_keeps_waiting() {
    Python::initialize();
    let (tx, rx) = broadcast::channel(4);
    let stream = LossyReceiver::new(rx);
    tx.send(1u32).unwrap();
    tx.send(2).unwrap();
    let (first, first_hook) = stream.next();
    assert!(matches!(first.await, Some(Next::Fresh(1))));
    let (second, second_hook) = stream.next();
    let second = tokio::spawn(second);
    tokio::time::sleep(STALL_WARNING * 3).await;
    assert!(stream.slot.warned.load(Ordering::Acquire));
    assert!(!second.is_finished(), "the warning must not end the wait");
    first_hook.settled();
    let next = tokio::time::timeout(Duration::from_secs(5), second)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(next, Some(Next::Fresh(2))));
    second_hook.settled();
    assert_eq!(stream.missed(), 0);
}

/// The slot keeps the older item and counts a newer one as missed.
#[test]
fn a_full_slot_keeps_the_older_item_and_counts_the_newer() {
    Python::initialize();
    Python::attach(|py| {
        let slot = Redelivery::default();
        slot.put_back(1u32.into_py_any(py).unwrap());
        slot.put_back(2u32.into_py_any(py).unwrap());
        assert_eq!(slot.missed.load(Ordering::Relaxed), 1);
        assert_eq!(slot.take().unwrap().extract::<u32>(py).unwrap(), 1);
        assert!(slot.take().is_none());
    });
}

/// A Python iterator over a `u32` channel the test feeds from Python.
#[pyclass(frozen)]
struct Probe {
    tx: StdMutex<Option<broadcast::Sender<u32>>>,
    stream: LossyReceiver<u32>,
}

#[pymethods]
impl Probe {
    fn send(&self, item: u32) {
        let tx = self.tx.lock().unwrap();
        tx.as_ref().expect("probe closed").send(item).unwrap();
    }

    /// Drop the sender, so the stream ends after what it holds.
    fn close(&self) {
        self.tx.lock().unwrap().take();
    }

    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let (next, put_back) = self.stream.next();
        crate::py_async::future_into_py_or_put_back(
            py,
            async move { next.await.ok_or_else(|| ended("probe")) },
            put_back,
        )
    }

    #[getter]
    fn missed(&self) -> u64 {
        self.stream.missed()
    }

    #[getter]
    fn in_flight(&self) -> usize {
        in_flight(&self.stream)
    }

    #[getter]
    fn warned(&self) -> bool {
        self.stream.slot.warned.load(Ordering::Acquire)
    }

    /// `set_result` callbacks queued on a loop so far.
    #[getter]
    fn queued(&self) -> u64 {
        self.stream.slot.trace.queued.load(Ordering::Relaxed)
    }

    /// The delivery paths taken so far, by name.
    #[getter]
    fn trace<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let trace = &self.stream.slot.trace;
        let paths = PyDict::new(py);
        for (name, count) in [
            ("before_delivery", &trace.before_delivery),
            ("on_loop", &trace.on_loop),
            ("closed_loop", &trace.closed_loop),
            ("delivered", &trace.delivered),
        ] {
            paths.set_item(name, count.load(Ordering::Relaxed))?;
        }
        Ok(paths)
    }
}

/// Helpers every check gets: `block_until` blocks the calling thread, and
/// so the loop, with the GIL released, until `done()` or a deadline;
/// `state()` is the probe's counters, for assertion messages.
const PRELUDE: &str = r#"
import asyncio, sys, threading, time
def state():
    return dict(trace=probe.trace, queued=probe.queued, in_flight=probe.in_flight,
                missed=probe.missed)
def block_until(done, what):
    deadline = time.monotonic() + 10
    while not done():
        if time.monotonic() > deadline:
            raise AssertionError("timed out waiting for " + what + ": " + repr(state()))
        time.sleep(0.001)
def trace(**expected):
    paths = dict(before_delivery=0, on_loop=0, closed_loop=0, delivered=0)
    paths.update(expected)
    return paths
"#;

fn run(code: &CStr) {
    Python::initialize();
    let code = CString::new(format!("{PRELUDE}{}", code.to_str().unwrap())).unwrap();
    Python::attach(|py| {
        let scope = PyDict::new(py);
        let (tx, rx) = broadcast::channel(16);
        let probe = Probe {
            tx: StdMutex::new(Some(tx)),
            stream: LossyReceiver::new(rx),
        };
        scope
            .set_item("probe", Py::new(py, probe).unwrap())
            .unwrap();
        if let Err(err) = py.run(&code, Some(&scope), None) {
            err.print(py);
            panic!("Python check failed: {err}");
        }
    });
}

/// Cancelled while its `set_result` callback waits on the event loop: the
/// callback finds the future done and puts the item back. A `next()` that
/// was already waiting wakes for it.
#[test]
fn an_item_cancelled_on_the_loop_comes_back_first() {
    run(cr#"
async def main():
    first = probe.next()
    await asyncio.sleep(0)
    probe.send(7)
    # The loop stays blocked until first's set_result callback is queued.
    block_until(lambda: probe.queued == 1, "the set_result callback")
    first.cancel()
    second = probe.next()
    assert await asyncio.wait_for(second, 5) == 7
    assert probe.trace == trace(on_loop=1, delivered=1), probe.trace
    assert probe.missed == 0
    probe.send(8)
    assert await asyncio.wait_for(probe.next(), 5) == 8
asyncio.run(main())
"#);
}

/// A burst of two with the first read cancelled after its task returned,
/// while its `set_result` callback waits on the loop: the second read,
/// already waiting, gets the first item back, then the second.
#[test]
fn a_cancelled_read_in_a_burst_keeps_the_order() {
    run(cr#"
async def main():
    first = probe.next()
    await asyncio.sleep(0)
    probe.send(1)
    block_until(lambda: probe.queued == 1, "the set_result callback")
    # With 1's callback still queued, the second read starts and 2 is
    # already in the channel: the read must wait behind 1, not take 2.
    second = probe.next()
    probe.send(2)
    assert probe.in_flight == 1 and probe.queued == 1
    first.cancel()
    assert await asyncio.wait_for(second, 5) == 1
    assert await asyncio.wait_for(probe.next(), 5) == 2
    assert probe.trace == trace(on_loop=1, delivered=2), probe.trace
    assert probe.missed == 0
asyncio.run(main())
"#);
}

/// The same, with the sender gone while the first item is on its way back:
/// the item still comes before the end of the stream.
#[test]
fn a_cancelled_read_comes_back_before_the_end() {
    run(cr#"
async def main():
    first = probe.next()
    await asyncio.sleep(0)
    probe.send(1)
    block_until(lambda: probe.queued == 1, "the set_result callback")
    first.cancel()
    probe.close()
    second = probe.next()
    assert await asyncio.wait_for(second, 5) == 1
    try:
        await asyncio.wait_for(probe.next(), 5)
    except StopAsyncIteration:
        pass
    else:
        raise AssertionError("the stream should have ended")
    assert probe.trace == trace(on_loop=1, delivered=1), probe.trace
asyncio.run(main())
"#);
}

/// Cancelled before the binding thread could take the GIL to deliver: the
/// first read finds its future done and puts its item back. The second
/// read waited behind it, so nothing is lost and the order holds.
///
/// The binding thread counts a `set_result` callback as queued only once
/// `call_soon_threadsafe` returns, and that call writes the loop's
/// self-pipe with the GIL released: the loop can run the callback, and the
/// awaiting coroutine read the count, before the binding thread counts it.
/// Windows' Proactor loop did (a flake on `assert probe.queued == 2`), so
/// this check forces that order on every platform and waits for the count.
#[test]
fn reads_cancelled_before_delivery_lose_nothing() {
    run(cr#"
async def main():
    loop = asyncio.get_running_loop()
    call_soon_threadsafe = loop.call_soon_threadsafe
    def callback_runs_first(callback, future, *args):
        # Return to the binding thread only once the loop has run the
        # callback and set the result.
        call_soon_threadsafe(callback, future, *args)
        if threading.current_thread() is not threading.main_thread():
            block_until(future.done, "the set_result callback to run")
    loop.call_soon_threadsafe = callback_runs_first
    interval = sys.getswitchinterval()
    first, second = probe.next(), probe.next()
    await asyncio.sleep(0)
    sys.setswitchinterval(30)
    try:
        probe.send(1)
        probe.send(2)
        # Holds the GIL until the first read's task has returned its item,
        # so its delivery is left waiting for the GIL.
        deadline = time.monotonic() + 10
        while probe.in_flight != 1:
            assert time.monotonic() < deadline, ("the first read never armed", state())
        first.cancel()
        second.cancel()
    finally:
        sys.setswitchinterval(interval)
    got = await asyncio.wait_for(probe.next(), 5)
    assert got == 1, (got, state())
    got = await asyncio.wait_for(probe.next(), 5)
    assert got == 2, (got, state())
    # Both results are set; the binding threads count their callbacks after.
    block_until(lambda: probe.queued == 2, "both set_result callbacks to be counted")
    # The first read's delivery took this path. The second read, cancelled
    # but not yet aborted when the item came back, may take it too and put
    # it back again; either way it is the only path besides the deliveries.
    paths = probe.trace
    assert paths["before_delivery"] in (1, 2), state()
    assert paths == trace(before_delivery=paths["before_delivery"], delivered=2), state()
    assert probe.queued == 2 and probe.in_flight == 0, state()
    assert probe.missed == 0, state()
    probe.send(3)
    got = await asyncio.wait_for(probe.next(), 5)
    assert got == 3, (got, state())
asyncio.run(main())
"#);
}

/// A read whose `set_result` callback is queued on a loop that stopped and
/// is never run again holds up reads from another loop: they warn once and
/// keep waiting. Closing the stalled loop drops the callback, which counts
/// the item as missed and lets the next read through.
#[test]
fn a_stalled_loop_warns_and_closing_it_counts_the_item() {
    run(cr#"
stalled = asyncio.new_event_loop()
async def start():
    pending = probe.next()
    await asyncio.sleep(0)
    return pending
pending = stalled.run_until_complete(start())
# The loop has stopped; the callback for this item is queued on it.
probe.send(1)
block_until(lambda: probe.queued == 1, "the set_result callback")
probe.send(2)
async def main():
    later = asyncio.ensure_future(probe.next())
    deadline = time.monotonic() + 10
    while not probe.warned:
        assert time.monotonic() < deadline, "no stall warning"
        await asyncio.sleep(0.05)
    assert not later.done()
    stalled.close()
    assert await asyncio.wait_for(later, 5) == 2
    assert probe.missed == 1
    assert probe.trace == trace(delivered=1), probe.trace
asyncio.run(main())
"#);
}
