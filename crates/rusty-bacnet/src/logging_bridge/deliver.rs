//! The forwarder thread, which hands queued records to `logging`, and the
//! flush that delivers what's queued before the interpreter exits.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::{Duration, Instant, UNIX_EPOCH};

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple};

use super::levels::{PREFIX, logger_name, python_level};
use super::record::Record;
use super::{BATCH, DROPPED, FLUSH};
use crate::py_async;

/// What the queue carries.
pub(super) enum Message {
    /// An event for `logging`.
    Record(Record),
    /// A flush: answered once every message queued before it is handled.
    Flush(SyncSender<()>),
}

/// How long the forwarder waits for a record before it reports drops that
/// no record came to carry.
const REPORT_INTERVAL: Duration = Duration::from_secs(1);

/// How long the exit hook waits for the queue to drain.
const EXIT_FLUSH: Duration = Duration::from_secs(2);

/// The forwarder thread: hand queued records to `logging` until the sending
/// side is gone. A panic while delivering loses its batch, which is counted
/// with the drops, and the loop goes on.
pub(super) fn forward_all(queue: &Receiver<Message>) {
    loop {
        let first = match queue.recv_timeout(REPORT_INTERVAL) {
            Ok(message) => Some(message),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        let batch: Vec<Message> = first
            .into_iter()
            .chain(queue.try_iter().take(BATCH - 1))
            .collect();
        let dropped = DROPPED.swap(0, Ordering::Relaxed);
        if batch.is_empty() && dropped == 0 {
            continue;
        }
        let records = batch
            .iter()
            .filter(|message| matches!(message, Message::Record(_)))
            .count() as u64;
        if catch_unwind(AssertUnwindSafe(|| deliver(batch, dropped))).is_err() {
            DROPPED.fetch_add(records + dropped, Ordering::Relaxed);
        }
    }
}

/// Deliver one batch, after reporting `dropped`, through the exit gate.
fn deliver(batch: Vec<Message>, dropped: u64) {
    // Refused once the interpreter is exiting: the batch is dropped, and a
    // flush in it sees its answer's sender go.
    let _ = py_async::attach(|py| {
        if dropped > 0
            && let Err(err) = report_dropped(py, dropped)
        {
            err.write_unraisable(py, None);
        }
        for message in batch {
            match message {
                Message::Record(record) => {
                    if let Err(err) = forward(py, &record) {
                        err.write_unraisable(py, None);
                    }
                }
                Message::Flush(done) => {
                    let _ = done.try_send(());
                }
            }
        }
        Ok(())
    });
}

/// Hand one record to its logger, if the logger still takes its level.
fn forward(py: Python<'_>, record: &Record) -> PyResult<()> {
    let metadata = record.metadata;
    let name = logger_name(metadata.target());
    let level = python_level(*metadata.level());
    let logger = py
        .import(intern!(py, "logging"))?
        .call_method1(intern!(py, "getLogger"), (&name,))?;
    // A level raised since the last refresh still filters here.
    if !logger
        .call_method1(intern!(py, "isEnabledFor"), (level,))?
        .is_truthy()?
    {
        return Ok(());
    }
    let fields = PyDict::new(py);
    for (key, value) in &record.fields {
        fields.set_item(key, value.to_python(py)?)?;
    }
    let extra = PyDict::new(py);
    extra.set_item(intern!(py, "bacnet_fields"), fields)?;
    extra.set_item(intern!(py, "bacnet_target"), metadata.target())?;
    let made = logger.call_method1(
        intern!(py, "makeRecord"),
        (
            &name,
            level,
            metadata.file().unwrap_or("(unknown file)"),
            metadata.line().unwrap_or(0),
            record.text(),
            PyTuple::empty(py),
            py.None(),
            py.None(),
            extra,
        ),
    )?;
    // When the event happened, not when it was delivered; relativeCreated
    // moves back by as much.
    if let Ok(since) = record.created.duration_since(UNIX_EPOCH) {
        let created = since.as_secs_f64();
        let delivered: f64 = made.getattr(intern!(py, "created"))?.extract()?;
        let relative: f64 = made.getattr(intern!(py, "relativeCreated"))?.extract()?;
        made.setattr(intern!(py, "created"), created)?;
        made.setattr(intern!(py, "msecs"), f64::from(since.subsec_millis()))?;
        made.setattr(
            intern!(py, "relativeCreated"),
            relative - (delivered - created) * 1000.0,
        )?;
    }
    logger.call_method1(intern!(py, "handle"), (made,))?;
    Ok(())
}

/// Warn, on `rusty_bacnet`, that `dropped` records never reached `logging`.
fn report_dropped(py: Python<'_>, dropped: u64) -> PyResult<()> {
    py.import(intern!(py, "logging"))?
        .call_method1(intern!(py, "getLogger"), (PREFIX,))?
        .call_method1(
            intern!(py, "warning"),
            (
                "%d log records were dropped: the queue to logging was full, \
                 or delivering them failed",
                dropped,
            ),
        )?;
    Ok(())
}

/// Wait until every record queued before the call has been handed to
/// `logging`, at most `timeout`. The GIL is released while waiting, since
/// the forwarder needs it. False if the wait timed out, or nothing forwards.
pub(super) fn flush(py: Python<'_>, timeout: Duration) -> bool {
    let Some(queue) = FLUSH.get() else {
        return false;
    };
    py.detach(|| {
        let deadline = Instant::now() + timeout;
        let (done, answered) = sync_channel(1);
        let mut request = Message::Flush(done);
        loop {
            match queue.try_send(request) {
                Ok(()) => break,
                Err(TrySendError::Full(back)) if Instant::now() < deadline => {
                    request = back;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(_) => return false,
            }
        }
        answered
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
    })
}

/// `atexit` hook, registered after the exit gate's so that it runs first:
/// deliver what's queued while the forwarder may still attach.
#[pyfunction]
pub(super) fn flush_at_exit(py: Python<'_>) {
    flush(py, EXIT_FLUSH);
}
