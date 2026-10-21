//! The stack's `tracing` events, forwarded to Python's `logging` (#1724).
//!
//! Importing the module installs [`Bridge`] as the global `tracing`
//! subscriber. An event at target `bacnet_transport::bip` goes to the logger
//! `rusty_bacnet.bacnet_transport.bip`; ERROR, WARN, INFO and DEBUG keep
//! their names, and TRACE goes out at level 5, named `TRACE`. The
//! `rusty_bacnet` logger gets a `NullHandler`, as a library's should, so an
//! application that configures no logging prints nothing.
//!
//! **Cost when off.** Whether a logger takes a level is read from a snapshot
//! of the `logging` configuration ([`levels`]), never from Python, so a
//! binding thread decides without the GIL. Each callsite asks once, and
//! `tracing` caches the answer in the callsite as an atomic `Interest`; a
//! disabled event then costs one atomic load, or less when its level is above
//! every logger's (the subscriber's level hint). `refresh_logging()`, and
//! each client, server, hub or endpoint start, reads the configuration again
//! and rebuilds those caches.
//!
//! **Thread safety.** An enabled event is formatted on the thread that emits
//! it and queued; it never takes the GIL there. That thread may hold any of
//! the stack's locks, and a Python thread may be waiting on one of them with
//! the GIL held, so taking the GIL there could deadlock, and it would stall a
//! Tokio worker. One forwarder thread ([`deliver`]) drains the queue and
//! calls `logging` through [`attach`](crate::py_async::attach), holding no
//! lock of ours, so a handler that calls back into `rusty_bacnet` (and logs
//! again) only queues more records. A full queue drops records, and a
//! warning reports how many.
//!
//! **Exit.** `logging` is imported before the exit gate's `atexit` hook is
//! registered, and the flush hook after it; hooks run last-registered first,
//! so the flush delivers what's queued, then the gate closes, then
//! `logging.shutdown` flushes the handlers. Records emitted after the flush
//! are dropped. A forked child has no forwarder thread, so it forwards
//! nothing: the bindings aren't fork-safe.
//!
//! The stack's throttled warnings (#1624, #1712) throttle before they log, so
//! they reach Python as rarely as they reach any subscriber.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::OnceLock;

use pyo3::exceptions::{PyRuntimeError, PyRuntimeWarning};
use pyo3::intern;
use pyo3::prelude::*;
use tracing::level_filters::LevelFilter;
use tracing::span;
use tracing::subscriber::Interest;
use tracing::{Dispatch, Event, Metadata, Subscriber};

mod deliver;
mod levels;
mod record;
#[cfg(test)]
mod tests;

use deliver::Message;
use levels::{logger_name, python_level, Levels, PREFIX};
use record::Record;

/// Records waiting for the forwarder; past this, new ones are dropped.
const QUEUE: usize = 4096;

/// Records delivered per attach, so a flood doesn't hold the GIL for long.
const BATCH: usize = 64;

/// Records dropped, because the queue was full or their delivery panicked,
/// not yet reported.
static DROPPED: AtomicU64 = AtomicU64::new(0);

/// The queue, for flushing; set once the bridge is the global subscriber.
static FLUSH: OnceLock<SyncSender<Message>> = OnceLock::new();

/// Records queued, for the tests.
#[cfg(test)]
static QUEUED: AtomicU64 = AtomicU64::new(0);

/// Import `logging`, before the exit gate's hook is registered, so that its
/// `atexit` shutdown runs after the gate closes.
pub(crate) fn import_logging(py: Python<'_>) -> PyResult<()> {
    py.import(intern!(py, "logging")).map(drop)
}

/// Register `refresh_logging`, install the bridge, give `rusty_bacnet` its
/// `NullHandler`, and register the flush at exit. Called after
/// [`py_async::register`](crate::py_async::register).
pub(crate) fn register(m: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = m.py();
    m.add_function(wrap_pyfunction!(refresh_logging, m)?)?;
    install(py)?;
    let logging = py.import(intern!(py, "logging"))?;
    let null = logging.call_method0(intern!(py, "NullHandler"))?;
    logging
        .call_method1(intern!(py, "getLogger"), (PREFIX,))?
        .call_method1(intern!(py, "addHandler"), (null,))?;
    let flush = wrap_pyfunction!(deliver::flush_at_exit, m)?;
    py.import(intern!(py, "atexit"))?
        .call_method1(intern!(py, "register"), (flush,))?;
    Ok(())
}

/// Read the `logging` configuration again, so that level changes made since
/// import (or the last refresh) decide which events are forwarded.
#[pyfunction]
pub(crate) fn refresh_logging(py: Python<'_>) -> PyResult<()> {
    // Numbered before reading, so that of two refreshes the later read wins.
    let generation = levels::next_generation();
    levels::store(generation, Levels::read(py)?);
    // Rebuilding locks tracing's callsite registry, which a thread holding
    // the GIL may be waiting on; so release it.
    py.detach(tracing::callsite::rebuild_interest_cache);
    Ok(())
}

/// [`refresh_logging`] as a client, server, hub or endpoint starts, so that
/// levels configured after import take effect. A failure only goes to
/// `sys.unraisablehook`: it must not stop the start.
pub(crate) fn refresh_at_start(py: Python<'_>) {
    if let Err(err) = refresh_logging(py) {
        err.write_unraisable(py, None);
    }
}

/// Install the bridge as the global subscriber, once.
///
/// This module's copy of `tracing` is its own: no other extension module,
/// and no Python code, can set its global subscriber, so this only finds one
/// set when a Rust embedder (or a test) set it first. Then that subscriber
/// stays, and nothing is forwarded.
pub(crate) fn install(py: Python<'_>) -> PyResult<()> {
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    if INSTALLED.load(Ordering::Acquire) {
        return Ok(());
    }
    levels::name_trace_level(py)?;
    levels::store(levels::next_generation(), read_or_default(py));
    let (queue, records) = sync_channel(QUEUE);
    std::thread::Builder::new()
        .name("rusty-bacnet-logging".into())
        .spawn(move || deliver::forward_all(&records))
        .map_err(|e| PyRuntimeError::new_err(format!("start the logging thread: {e}")))?;
    let flush = queue.clone();
    // If a subscriber is set already, the dispatch and both senders are
    // dropped, and the forwarder thread ends.
    if py
        .detach(|| tracing::dispatcher::set_global_default(Dispatch::new(Bridge { queue })))
        .is_ok()
    {
        let _ = FLUSH.set(flush);
    }
    INSTALLED.store(true, Ordering::Release);
    Ok(())
}

/// The `logging` levels, or `logging`'s defaults with a `RuntimeWarning` if
/// they can't be read: a strange configuration mustn't fail the import.
fn read_or_default(py: Python<'_>) -> Levels {
    Levels::read(py).unwrap_or_else(|err| {
        let message =
            format!("rusty_bacnet: reading the logging levels failed ({err}); using WARNING");
        let warned = py.import(intern!(py, "warnings")).and_then(|warnings| {
            warnings.call_method1(
                intern!(py, "warn"),
                (message, py.get_type::<PyRuntimeWarning>()),
            )
        });
        if let Err(err) = warned {
            err.write_unraisable(py, None);
        }
        Levels::default()
    })
}

/// The subscriber: decides from [`levels`], queues what's enabled.
struct Bridge {
    queue: SyncSender<Message>,
}

/// Whether the logger for `metadata` takes it, by the current snapshot.
fn wanted(metadata: &Metadata<'_>) -> bool {
    metadata.is_event()
        && levels::with_current(|levels| {
            levels.enabled(
                &logger_name(metadata.target()),
                python_level(*metadata.level()),
            )
        })
}

impl Subscriber for Bridge {
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if wanted(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }

    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        wanted(metadata)
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(levels::with_current(Levels::max_level))
    }

    // Spans are never enabled, so these are never called with one.
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }

    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}

    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}

    fn event(&self, event: &Event<'_>) {
        match self.queue.try_send(Message::Record(Record::capture(event))) {
            Ok(()) => {
                #[cfg(test)]
                QUEUED.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) => {
                DROPPED.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }

    fn enter(&self, _: &span::Id) {}

    fn exit(&self, _: &span::Id) {}
}
