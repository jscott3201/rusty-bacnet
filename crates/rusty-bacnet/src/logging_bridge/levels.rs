//! What the `logging` configuration lets through, read while attached and
//! kept so that a binding thread can decide whether an event is wanted
//! without the GIL (#1724).
//!
//! The snapshot holds the root logger's level, `logging.disable()`'s level,
//! and the level and `disabled` flag of every logger named `rusty_bacnet` or
//! `rusty_bacnet.*`. [`Levels::enabled`] answers as `Logger.isEnabledFor`
//! would for any such name, created yet or not: a logger's effective level is
//! the first level set on it or on an ancestor, else the root's.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, PoisonError, RwLock};

use pyo3::intern;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use tracing::level_filters::LevelFilter;
use tracing::Level;

/// The logger every forwarded event goes under.
pub(crate) const PREFIX: &str = "rusty_bacnet";

/// The `logging` level TRACE events go out at, below DEBUG's 10.
pub(crate) const TRACE_LEVEL: i32 = 5;

/// `logging.WARNING`, the root logger's level until something sets one.
const WARNING: i32 = 30;

/// The `logging` level for a tracing level.
pub(crate) const fn python_level(level: Level) -> i32 {
    match level {
        Level::ERROR => 40,
        Level::WARN => WARNING,
        Level::INFO => 20,
        Level::DEBUG => 10,
        Level::TRACE => TRACE_LEVEL,
    }
}

/// The logger name for a tracing target: `rusty_bacnet.` and the target with
/// each `::` turned into `.`, so that the module path becomes the logger
/// hierarchy (`bacnet_transport::bip` → `rusty_bacnet.bacnet_transport.bip`).
pub(crate) fn logger_name(target: &str) -> String {
    let mut name = String::with_capacity(PREFIX.len() + 1 + target.len());
    name.push_str(PREFIX);
    if !target.is_empty() {
        name.push('.');
        name.push_str(&target.replace("::", "."));
    }
    name
}

/// One logger's own settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LoggerLevel {
    /// `Logger.level`; 0 (NOTSET) defers to the parent.
    pub(crate) level: i32,
    /// `Logger.disabled`, which `logging.config` sets on loggers it turns off.
    pub(crate) disabled: bool,
}

/// The parts of the `logging` configuration that decide `isEnabledFor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Levels {
    /// `logging.root.manager.disable`: levels at or below it are off.
    pub(crate) disable: i32,
    /// The root logger's level.
    pub(crate) root: i32,
    /// The `rusty_bacnet` loggers that exist, by name.
    pub(crate) loggers: HashMap<String, LoggerLevel>,
}

impl Default for Levels {
    /// What `logging` starts with: nothing disabled, the root at WARNING.
    fn default() -> Self {
        Self {
            disable: 0,
            root: WARNING,
            loggers: HashMap::new(),
        }
    }
}

impl Levels {
    /// Whether the logger `name` takes records at `level`.
    pub(crate) fn enabled(&self, name: &str, level: i32) -> bool {
        if level <= self.disable || self.loggers.get(name).is_some_and(|l| l.disabled) {
            return false;
        }
        level >= self.effective(name)
    }

    /// The first level set on `name` or an ancestor, else the root's.
    fn effective(&self, name: &str) -> i32 {
        let mut current = name;
        loop {
            if let Some(logger) = self.loggers.get(current).filter(|l| l.level != 0) {
                return logger.level;
            }
            match current.rfind('.') {
                Some(dot) => current = &current[..dot],
                None => return self.root,
            }
        }
    }

    /// The most verbose tracing level any `rusty_bacnet` logger could take,
    /// as the subscriber's hint: the lowest level set anywhere on the path
    /// to the root bounds every effective level from below.
    pub(crate) fn max_level(&self) -> LevelFilter {
        let floor = self
            .loggers
            .values()
            .filter(|l| l.level != 0)
            .map(|l| l.level)
            .fold(self.root, i32::min);
        [
            Level::TRACE,
            Level::DEBUG,
            Level::INFO,
            Level::WARN,
            Level::ERROR,
        ]
        .into_iter()
        .find(|&level| {
            let python = python_level(level);
            python >= floor && python > self.disable
        })
        .map_or(LevelFilter::OFF, LevelFilter::from_level)
    }

    /// Read the configuration from `logging`.
    pub(crate) fn read(py: Python<'_>) -> PyResult<Self> {
        let logging = py.import(intern!(py, "logging"))?;
        let root = logging.getattr(intern!(py, "root"))?;
        let manager = root.getattr(intern!(py, "manager"))?;
        let logger_type = logging.getattr(intern!(py, "Logger"))?;
        // A copy, so that nothing another thread does changes it mid-walk.
        let registered = manager
            .getattr(intern!(py, "loggerDict"))?
            .call_method0(intern!(py, "copy"))?;
        let registered = registered.cast::<PyDict>()?;
        let mut loggers = HashMap::new();
        for (name, logger) in registered.iter() {
            let Ok(name) = name.extract::<String>() else {
                continue;
            };
            let ours = name == PREFIX
                || name
                    .strip_prefix(PREFIX)
                    .is_some_and(|rest| rest.starts_with('.'));
            // A PlaceHolder stands for a logger not yet created.
            if !ours || !logger.is_instance(&logger_type)? {
                continue;
            }
            let level = LoggerLevel {
                level: logger.getattr(intern!(py, "level"))?.extract()?,
                disabled: logger.getattr(intern!(py, "disabled"))?.is_truthy()?,
            };
            loggers.insert(name, level);
        }
        Ok(Self {
            disable: manager.getattr(intern!(py, "disable"))?.extract()?,
            root: root.getattr(intern!(py, "level"))?.extract()?,
            loggers,
        })
    }
}

/// The snapshot binding threads read, with the generation of the read that
/// made it. Nothing calls into Python, or waits for the GIL, while holding
/// the lock, in either direction.
static CURRENT: LazyLock<RwLock<(u64, Levels)>> = LazyLock::new(RwLock::default);

/// Numbers the reads, so that [`store`] keeps the latest.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// A number for a read about to start, above every earlier one.
pub(crate) fn next_generation() -> u64 {
    GENERATION.fetch_add(1, Ordering::Relaxed) + 1
}

/// Run `f` on the current snapshot.
pub(crate) fn with_current<R>(f: impl FnOnce(&Levels) -> R) -> R {
    f(&CURRENT.read().unwrap_or_else(PoisonError::into_inner).1)
}

/// Replace the snapshot with `levels`, read as `generation`, unless a read
/// that started later already stored its own: two refreshes that overlap
/// (on free-threaded Python, or when one releases the GIL mid-read) leave
/// the later read's levels. Callsites keep their cached answer until the
/// caller rebuilds the interest cache.
pub(crate) fn store(generation: u64, levels: Levels) {
    let mut current = CURRENT.write().unwrap_or_else(PoisonError::into_inner);
    if generation > current.0 {
        *current = (generation, levels);
    }
}

/// Name level 5 `TRACE` in `logging`, unless something named it already.
pub(crate) fn name_trace_level(py: Python<'_>) -> PyResult<()> {
    let logging = py.import(intern!(py, "logging"))?;
    let name = logging.call_method1(intern!(py, "getLevelName"), (TRACE_LEVEL,))?;
    if name
        .extract::<String>()
        .is_ok_and(|name| name.starts_with("Level "))
    {
        logging.call_method1(intern!(py, "addLevelName"), (TRACE_LEVEL, "TRACE"))?;
    }
    Ok(())
}
