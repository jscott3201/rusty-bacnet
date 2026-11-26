//! A watch's steps (#2561): an update whose `__anext__` was cancelled while
//! on its way to Python comes back first, concurrent steps each get the
//! next update, and an ended watch raises StopAsyncIteration, with why, to
//! a step waiting and to every later one. A source of items ready at once,
//! or never, stands in for the hub (an `Update` is made only by the hub),
//! so the event loop's timing is the test's; the hub's updates run over
//! the wire in the Python suite (`tests/test_watch.py`).

use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};

use pyo3::prelude::*;
use pyo3::types::PyDict;
use tokio::sync::broadcast::error::RecvError;

use super::{Ended, step};
use crate::event_stream::{EventSource, LossyReceiver};

/// The numbers 0, 1, ..., handed out at once, then nothing ever again: a
/// watch never ends on its own.
struct Ready(VecDeque<u32>);

impl EventSource<u32> for Ready {
    async fn recv(&mut self) -> Result<u32, RecvError> {
        match self.0.pop_front() {
            Some(item) => Ok(item),
            None => std::future::pending().await,
        }
    }
}

/// `step` over [`Ready`], ending as a watch does, with the delivery paths
/// it took.
#[pyclass(frozen)]
struct Probe {
    stream: LossyReceiver<u32, Ready>,
    ended: Arc<OnceLock<Ended>>,
}

#[pymethods]
impl Probe {
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let ended = Arc::clone(&self.ended);
        let why = move || ended.get().copied().unwrap_or(Ended::Closed);
        step(py, &self.stream, why, |item| item, |put_back| put_back)
    }

    /// End the stream as the client's stop does.
    fn stop(&self) {
        let _ = self.ended.set(Ended::Stopped);
        self.stream.close();
    }

    /// `set_result` callbacks queued on a loop so far.
    #[getter]
    fn queued(&self) -> u64 {
        self.stream.trace().queued.load(Ordering::Relaxed)
    }

    /// Items put back by their `set_result` callback on the loop.
    #[getter]
    fn on_loop(&self) -> u64 {
        self.stream.trace().on_loop.load(Ordering::Relaxed)
    }
}

/// Run `code` with `probe` over `count` items, and `block_until`, which
/// blocks the loop with the GIL released until `done()` or a deadline.
fn run(count: u32, code: &CStr) {
    const PRELUDE: &str = r#"
import asyncio, time
def block_until(done, what):
    deadline = time.monotonic() + 10
    while not done():
        if time.monotonic() > deadline:
            raise AssertionError("timed out waiting for " + what)
        time.sleep(0.001)
"#;
    Python::initialize();
    let code = CString::new(format!("{PRELUDE}{}", code.to_str().unwrap())).unwrap();
    Python::attach(|py| {
        let probe = Probe {
            stream: LossyReceiver::from_source(Ready((0..count).collect())),
            ended: Arc::new(OnceLock::new()),
        };
        let scope = PyDict::new(py);
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
/// callback finds the future done and puts the update back, and the next
/// step hands it out before the ones after it.
#[test]
fn an_update_cancelled_on_its_way_comes_first() {
    run(
        3,
        cr#"
async def main():
    first = probe.next()
    # The loop stays blocked until first's set_result callback is queued.
    block_until(lambda: probe.queued == 1, "the set_result callback")
    first.cancel()
    assert await asyncio.wait_for(probe.next(), 5) == 0
    assert probe.on_loop == 1
    assert await asyncio.wait_for(probe.next(), 5) == 1
    assert await asyncio.wait_for(probe.next(), 5) == 2
asyncio.run(main())
"#,
    );
}

/// Two steps at once complete one at a time, each with the next update.
#[test]
fn concurrent_steps_get_consecutive_updates() {
    run(
        4,
        cr#"
async def main():
    pair = await asyncio.wait_for(asyncio.gather(probe.next(), probe.next()), 5)
    assert sorted(pair) == [0, 1], pair
    pair = await asyncio.wait_for(asyncio.gather(probe.next(), probe.next()), 5)
    assert sorted(pair) == [2, 3], pair
asyncio.run(main())
"#,
    );
}

/// A step waiting when the watch ends raises StopAsyncIteration saying
/// why, and so does every step after it.
#[test]
fn an_ended_watch_ends_a_waiting_step_and_stays_ended() {
    run(
        0,
        cr#"
async def main():
    waiting = asyncio.ensure_future(probe.next())
    await asyncio.sleep(0.05)
    assert not waiting.done()
    probe.stop()
    for step in (waiting, probe.next(), probe.next()):
        try:
            await asyncio.wait_for(step, 5)
        except StopAsyncIteration as ended:
            assert "the client stopped" in str(ended), ended
        else:
            raise AssertionError("the watch should have ended")
asyncio.run(main())
"#,
    );
}
