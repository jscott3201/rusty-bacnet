//! The point iterator's delivery (#2561): a point whose `__anext__` was
//! cancelled while on its way to Python comes back first, and concurrent
//! steps each get the next point. A source of records ready at once stands
//! in for the device, so the event loop's timing is the test's; the walk's
//! reads run over the wire in the Python suite (`tests/test_inventory.py`).

use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::sync::atomic::Ordering;

use bacnet_ops::DeviceAddress;
use bacnet_ops::inventory::PointRecord;
use bacnet_ops::target::AddressStyle;
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use tokio::sync::broadcast::error::RecvError;

use super::{Step, step};
use crate::event_stream::{EventSource, LossyReceiver};

/// The records of analog inputs 0, 1, ..., each named `AI n`, handed out
/// at once, then the end.
struct Ready(VecDeque<PointRecord>);

impl EventSource<Step> for Ready {
    async fn recv(&mut self) -> Result<Step, RecvError> {
        let record = self.0.pop_front().ok_or(RecvError::Closed)?;
        Ok(Step::Point(Box::new(record), AddressStyle::Bip))
    }
}

/// `step` over [`Ready`], with the delivery paths it took.
#[pyclass(frozen)]
struct Probe(LossyReceiver<Step, Ready>);

#[pymethods]
impl Probe {
    fn next<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        step(py, &self.0)
    }

    /// `set_result` callbacks queued on a loop so far.
    #[getter]
    fn queued(&self) -> u64 {
        self.0.trace().queued.load(Ordering::Relaxed)
    }

    /// Points put back by their `set_result` callback on the loop.
    #[getter]
    fn on_loop(&self) -> u64 {
        self.0.trace().on_loop.load(Ordering::Relaxed)
    }
}

/// Run `code` with `probe` over `count` points, and `block_until`, which
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
    let device = DeviceAddress::local(&[127, 0, 0, 1, 0xBA, 0xC0]).unwrap();
    let records = (0..count)
        .map(|n| {
            let object = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, n).unwrap();
            let mut record = PointRecord::new(device.clone(), 1234, object);
            record.name = Some(format!("AI {n}"));
            record
        })
        .collect();
    let code = CString::new(format!("{PRELUDE}{}", code.to_str().unwrap())).unwrap();
    Python::attach(|py| {
        let probe = Probe(LossyReceiver::from_source(Ready(records)));
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
/// callback finds the future done and puts the point back, and the next
/// step hands it out before the points after it.
#[test]
fn a_point_cancelled_on_its_way_comes_first() {
    run(
        3,
        cr#"
async def main():
    first = probe.next()
    # The loop stays blocked until first's set_result callback is queued.
    block_until(lambda: probe.queued == 1, "the set_result callback")
    first.cancel()
    point = await asyncio.wait_for(probe.next(), 5)
    assert point.name == "AI 0", point
    assert probe.on_loop == 1
    assert (await asyncio.wait_for(probe.next(), 5)).name == "AI 1"
    assert (await asyncio.wait_for(probe.next(), 5)).name == "AI 2"
    try:
        await asyncio.wait_for(probe.next(), 5)
    except StopAsyncIteration:
        pass
    else:
        raise AssertionError("the walk should have ended")
asyncio.run(main())
"#,
    );
}

/// Two steps at once complete one at a time, each with the next point.
#[test]
fn concurrent_steps_get_consecutive_points() {
    run(
        4,
        cr#"
async def main():
    pair = await asyncio.wait_for(asyncio.gather(probe.next(), probe.next()), 5)
    assert sorted(p.name for p in pair) == ["AI 0", "AI 1"], pair
    pair = await asyncio.wait_for(asyncio.gather(probe.next(), probe.next()), 5)
    assert sorted(p.name for p in pair) == ["AI 2", "AI 3"], pair
asyncio.run(main())
"#,
    );
}
