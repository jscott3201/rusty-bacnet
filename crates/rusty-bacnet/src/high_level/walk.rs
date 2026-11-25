//! [`PyPointIterator`]: what `await device.points(...)` returns (#2561), a
//! device's point records handed out as an async iterator, one batch of
//! objects (one ReadPropertyMultiple while the device takes them) a refill,
//! each a call of
//! [`Device::point_records`](bacnet_ops::Device::point_records).
//!
//! The iterator keeps the device's `Device` state ([`Core`]) and takes the
//! client's lease for each refill, as each `Device` call does, so it holds
//! no client between reads and a stopped client's iterator raises what any
//! call raises then.
//!
//! The records go through the event iterators' receiver
//! ([`LossyReceiver`]), whose source here is [`Walk`]. Its `recv` is
//! cancel-safe: the walk moves on only on the poll that returns, so a
//! cancelled `__anext__` sends no further request, and the next one reads
//! the same batch again. A record whose `__anext__` was cancelled while it
//! was on its way goes back into the receiver's slot and comes out first,
//! so no record is lost or handed out twice. Nothing runs between
//! `__anext__` calls: an `async for` broken off reads nothing more.

use std::collections::VecDeque;
use std::sync::Arc;

use bacnet_ops::InventoryOptions;
use bacnet_ops::inventory::PointRecord;
use bacnet_ops::target::AddressStyle;
use bacnet_types::primitives::ObjectIdentifier;
use pyo3::exceptions::PyStopAsyncIteration;
use pyo3::prelude::*;
use pyo3::types::PyTuple;
use tokio::sync::broadcast::error::RecvError;

use super::device::Core;
use super::errors::ops_err;
use super::points::PyPoint;
use crate::event_stream::{EventSource, LossyReceiver, Next};
use crate::py_async::future_into_py_or_put_back;
use crate::types::PyObjectIdentifier;

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;

/// What one `__anext__` takes from the walk.
pub(super) enum Step {
    /// The next record, its device's address shown in the style.
    Point(Box<PointRecord>, AddressStyle),
    /// Why the walk can't go on now: raised, and the next `__anext__`
    /// tries the same objects again.
    Failed(PyErr),
}

/// The walk over a device's objects: the objects, how far the records
/// have been read, and the records read but not handed out yet.
pub(super) struct Walk {
    core: Arc<Core>,
    objects: Arc<[ObjectIdentifier]>,
    options: InventoryOptions,
    /// The first object whose record isn't read yet.
    next: usize,
    /// Records read and not handed out yet, in order.
    read: VecDeque<PointRecord>,
    /// The address style the records in `read` were read with.
    style: AddressStyle,
    /// Why the last batch stopped, raised once the records before it are
    /// out.
    stopped: Option<PyErr>,
}

impl Walk {
    /// Read the next batch: as many of the objects from `next` as one
    /// request reads ([`Device::point_batch`](bacnet_ops::Device::point_batch)).
    /// Only the poll that returns changes `self`, so a dropped read leaves
    /// the walk where it was.
    async fn refill(&mut self) -> Result<(), PyErr> {
        let objects = Arc::clone(&self.objects);
        let (from, options) = (self.next, self.options.clone());
        let read = self
            .core
            .run(move |device, style| async move {
                let rest = &objects[from..];
                let batch = device.point_batch(rest, &options);
                let read = device
                    .point_records(rest[..batch].iter().copied(), &options)
                    .await;
                let read = read.map_err(|error| ops_err(error, style))?;
                Ok((read, batch, style))
            })
            .await?;
        let (read, batch, style) = read;
        self.style = style;
        self.next = match read.stopped {
            // The records come in the order asked, up to where it stopped.
            Some(error) => {
                self.stopped = Some(ops_err(error, style));
                from + read.points.len()
            }
            None => from + batch,
        };
        self.read.extend(read.points);
        Ok(())
    }
}

impl EventSource<Step> for Walk {
    /// The next record; the error that stopped the walk, once the records
    /// before it are out; or `Closed` once every object is read.
    async fn recv(&mut self) -> Result<Step, RecvError> {
        loop {
            if let Some(record) = self.read.pop_front() {
                return Ok(Step::Point(Box::new(record), self.style));
            }
            if let Some(error) = self.stopped.take() {
                return Ok(Step::Failed(error));
            }
            if self.next >= self.objects.len() {
                return Err(RecvError::Closed);
            }
            if let Err(error) = self.refill().await {
                return Ok(Step::Failed(error));
            }
        }
    }
}

/// One `__anext__` over `stream`, as an asyncio future: the next point,
/// the error that stopped the walk, or StopAsyncIteration at the end. A
/// point whose future was cancelled on its way goes back into the
/// receiver's slot, and the next step hands it out first.
fn step<'py, S: EventSource<Step> + 'static>(
    py: Python<'py>,
    stream: &LossyReceiver<Step, S>,
) -> PyResult<Bound<'py, PyAny>> {
    let (next, put_back) = stream.next();
    future_into_py_or_put_back(
        py,
        async move {
            match next.await {
                Some(Next::Fresh(Step::Point(record, style))) => {
                    Ok(Next::Fresh(Fresh(record, style)))
                }
                Some(Next::Again(point)) => Ok(Next::Again(point)),
                Some(Next::Fresh(Step::Failed(error))) => Err(error),
                None => Err(PyStopAsyncIteration::new_err("every point was read")),
            }
        },
        put_back,
    )
}

/// A fresh record, made a `Point` when it reaches Python.
struct Fresh(Box<PointRecord>, AddressStyle);

impl<'py> IntoPyObject<'py> for Fresh {
    type Target = PyPoint;
    type Output = Bound<'py, PyPoint>;
    type Error = PyErr;

    fn into_pyobject(self, py: Python<'py>) -> PyResult<Bound<'py, PyPoint>> {
        Bound::new(py, PyPoint::new(py, &self.0, self.1)?)
    }
}

/// A device's points, read as they are iterated: what `await
/// device.points(...)` returns. Each `__anext__` hands out the next
/// `Point`, reading the next batch of objects (one request while the device
/// takes ReadPropertyMultiple) when the last is out. A device that stops
/// answering raises from `__anext__` once the points read before it are
/// out; iterating again tries the first object not read again. Frozen;
/// compares by identity and refuses `copy` and `pickle`, as a `Point`
/// does.
#[pyclass(name = "PointIterator", module = "rusty_bacnet", frozen)]
pub(crate) struct PyPointIterator {
    objects: Arc<[ObjectIdentifier]>,
    stream: LossyReceiver<Step, Walk>,
}

impl PyPointIterator {
    /// The walk over `objects` of the device `core` names, read under
    /// `options`.
    pub(super) fn new(
        core: Arc<Core>,
        objects: Vec<ObjectIdentifier>,
        options: InventoryOptions,
    ) -> Self {
        let objects: Arc<[ObjectIdentifier]> = objects.into();
        let walk = Walk {
            core,
            objects: Arc::clone(&objects),
            options,
            next: 0,
            read: VecDeque::new(),
            style: AddressStyle::Bip,
            stopped: None,
        };
        Self {
            objects,
            stream: LossyReceiver::from_source(walk),
        }
    }
}

#[pymethods]
impl PyPointIterator {
    /// The objects it reads, in order, as `ObjectIdentifier`s: the
    /// Object_List's of the types asked for, or those named, each once.
    #[getter]
    fn objects<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(
            py,
            self.objects
                .iter()
                .map(|o| PyObjectIdentifier::from_rust(*o)),
        )
    }

    fn __len__(&self) -> usize {
        self.objects.len()
    }

    fn __aiter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __anext__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        step(py, &self.stream)
    }

    fn __repr__(&self) -> String {
        format!("PointIterator({} objects)", self.objects.len())
    }

    /// Refuses `copy` and `pickle`: a walk over a live client. See
    /// [`crate::types::not_picklable`].
    fn __reduce__(slf: &Bound<'_, Self>) -> PyResult<()> {
        Err(crate::types::not_picklable(slf.as_any()))
    }
}
