//! The logging bridge (#1724): the level snapshot alone, then the installed
//! subscriber against a real interpreter. nextest runs each test in its own
//! process, so each installs the bridge itself.

use std::collections::HashMap;
use std::hint::black_box;
use std::time::{Duration, Instant};

use pyo3::types::PyDict;

use super::levels::{Levels, LoggerLevel, TRACE_LEVEL, logger_name};
use super::record::{Value, message_text};
use super::*;

fn snapshot(root: i32, disable: i32, loggers: &[(&str, i32, bool)]) -> Levels {
    Levels {
        disable,
        root,
        loggers: loggers
            .iter()
            .map(|&(name, level, disabled)| (name.to_owned(), LoggerLevel { level, disabled }))
            .collect::<HashMap<_, _>>(),
    }
}

#[test]
fn logger_names_follow_the_module_path() {
    assert_eq!(
        logger_name("bacnet_transport::bip::ingress"),
        "rusty_bacnet.bacnet_transport.bip.ingress"
    );
    assert_eq!(logger_name("custom"), "rusty_bacnet.custom");
    assert_eq!(logger_name(""), "rusty_bacnet");
}

#[test]
fn levels_follow_the_logger_hierarchy_like_is_enabled_for() {
    let levels = snapshot(
        30,
        0,
        &[
            ("rusty_bacnet", 0, false),
            ("rusty_bacnet.bacnet_transport", 10, false),
            ("rusty_bacnet.bacnet_transport.sc", 40, false),
            ("rusty_bacnet.bacnet_network", 0, true),
        ],
    );
    // Inherited from the nearest ancestor with a level.
    assert!(levels.enabled("rusty_bacnet.bacnet_transport.bip.ingress", 10));
    assert!(!levels.enabled("rusty_bacnet.bacnet_transport.bip.ingress", TRACE_LEVEL));
    assert!(!levels.enabled("rusty_bacnet.bacnet_transport.sc.hub", 30));
    assert!(levels.enabled("rusty_bacnet.bacnet_transport.sc.hub", 40));
    // NOTSET all the way up: the root's level.
    assert!(levels.enabled("rusty_bacnet.bacnet_client", 30));
    assert!(!levels.enabled("rusty_bacnet.bacnet_client", 20));
    // A disabled logger takes nothing; its children are not disabled.
    assert!(!levels.enabled("rusty_bacnet.bacnet_network", 50));
    assert!(levels.enabled("rusty_bacnet.bacnet_network.layer", 30));
    // logging.disable(WARNING) turns WARNING off everywhere.
    let disabled = Levels {
        disable: 30,
        ..levels.clone()
    };
    assert!(!disabled.enabled("rusty_bacnet.bacnet_transport", 30));
    assert!(disabled.enabled("rusty_bacnet.bacnet_transport", 40));
    // A root at NOTSET takes everything.
    assert!(snapshot(0, 0, &[]).enabled("rusty_bacnet.x", TRACE_LEVEL));
}

#[test]
fn the_level_hint_is_the_most_verbose_any_logger_takes() {
    assert_eq!(snapshot(30, 0, &[]).max_level(), LevelFilter::WARN);
    assert_eq!(snapshot(0, 0, &[]).max_level(), LevelFilter::TRACE);
    let debug = snapshot(30, 0, &[("rusty_bacnet.a", 10, false)]);
    assert_eq!(debug.max_level(), LevelFilter::DEBUG);
    assert_eq!(
        Levels {
            disable: 10,
            ..debug
        }
        .max_level(),
        LevelFilter::INFO
    );
    assert_eq!(snapshot(50, 0, &[]).max_level(), LevelFilter::OFF);
}

#[test]
fn the_message_carries_the_fields_as_key_value_pairs() {
    let fields = vec![
        ("peer", Value::Str("10.0.0.1".into())),
        ("count", Value::U64(3)),
        ("ok", Value::Bool(true)),
    ];
    assert_eq!(
        message_text(Some("dropped"), &fields),
        "dropped peer=10.0.0.1 count=3 ok=true"
    );
    assert_eq!(message_text(None, &fields[1..2]), "count=3");
}

/// Start an interpreter and install the bridge.
fn installed() {
    Python::initialize();
    Python::attach(|py| install(py).unwrap());
}

/// A handler that keeps every record, on the logger `rusty_bacnet.<target>`,
/// which stops propagation so the records don't reach stderr.
fn capture(py: Python<'_>, target: &str, level: &str) -> Py<PyAny> {
    let scope = PyDict::new(py);
    scope.set_item("NAME", logger_name(target)).unwrap();
    scope.set_item("LEVEL", level).unwrap();
    py.run(
        c"import logging
class Capture(logging.Handler):
    def __init__(self):
        super().__init__(logging.NOTSET)
        self.records = []
    def emit(self, record):
        self.records.append(record)
capture = Capture()
logger = logging.getLogger(NAME)
logger.addHandler(capture)
logger.setLevel(LEVEL)
logger.propagate = False
",
        Some(&scope),
        None,
    )
    .unwrap();
    scope.get_item("capture").unwrap().unwrap().unbind()
}

fn set_level(py: Python<'_>, target: &str, level: &str) {
    py.import("logging")
        .unwrap()
        .call_method1("getLogger", (logger_name(target),))
        .unwrap()
        .call_method1("setLevel", (level,))
        .unwrap();
}

/// The records `handler` holds once it holds `count`, waiting without the
/// GIL for the forwarder to deliver them.
fn records(handler: &Py<PyAny>, count: usize) -> Vec<Py<PyAny>> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let held: Vec<Py<PyAny>> = Python::attach(|py| {
            handler
                .bind(py)
                .getattr("records")
                .unwrap()
                .extract()
                .unwrap()
        });
        if held.len() >= count {
            return held;
        }
        assert!(
            Instant::now() < deadline,
            "{} of {count} records",
            held.len()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn attr<T: for<'a, 'py> FromPyObject<'a, 'py>>(record: &Py<PyAny>, name: &str) -> T
where
    for<'a, 'py> <T as FromPyObject<'a, 'py>>::Error: std::fmt::Debug,
{
    Python::attach(|py| record.bind(py).getattr(name).unwrap().extract().unwrap())
}

#[test]
fn an_event_reaches_its_logger_with_its_fields() {
    installed();
    let handler = Python::attach(|py| {
        // "TRACE" is a level name once the bridge is installed.
        let handler = capture(py, "bridge_test", "TRACE");
        refresh_logging(py).unwrap();
        handler
    });
    let line = line!() + 1;
    tracing::warn!(target: "bridge_test", count = 42u64, peer = "10.0.0.1", ok = true, "hello {}", 1);
    let record = records(&handler, 1).remove(0);
    assert_eq!(attr::<String>(&record, "name"), "rusty_bacnet.bridge_test");
    assert_eq!(attr::<i32>(&record, "levelno"), 30);
    assert_eq!(attr::<String>(&record, "levelname"), "WARNING");
    assert_eq!(
        attr::<String>(&record, "msg"),
        "hello 1 count=42 peer=10.0.0.1 ok=true"
    );
    assert_eq!(attr::<u32>(&record, "lineno"), line);
    assert!(attr::<String>(&record, "pathname").ends_with("tests.rs"));
    assert_eq!(attr::<String>(&record, "bacnet_target"), "bridge_test");
    Python::attach(|py| {
        let fields = record.bind(py).getattr("bacnet_fields").unwrap();
        assert_eq!(
            fields.get_item("count").unwrap().extract::<u64>().unwrap(),
            42
        );
        assert_eq!(
            fields
                .get_item("peer")
                .unwrap()
                .extract::<String>()
                .unwrap(),
            "10.0.0.1"
        );
        assert!(fields.get_item("ok").unwrap().extract::<bool>().unwrap());
    });

    // TRACE goes out at level 5, named TRACE.
    tracing::trace!(target: "bridge_test", "fine detail");
    let record = records(&handler, 2).remove(1);
    assert_eq!(attr::<i32>(&record, "levelno"), TRACE_LEVEL);
    assert_eq!(attr::<String>(&record, "levelname"), "TRACE");
}

#[test]
fn a_lower_level_takes_effect_on_refresh() {
    installed();
    let handler = Python::attach(|py| {
        let handler = capture(py, "refresh_test", "WARNING");
        refresh_logging(py).unwrap();
        handler
    });
    let queued = QUEUED.load(Ordering::Relaxed);
    tracing::debug!(target: "refresh_test", "below the level");
    // Lowered, but the callsites still hold the old answer.
    Python::attach(|py| set_level(py, "refresh_test", "DEBUG"));
    tracing::debug!(target: "refresh_test", "before the refresh");
    assert_eq!(QUEUED.load(Ordering::Relaxed), queued, "nothing queued");
    Python::attach(|py| refresh_logging(py).unwrap());
    tracing::debug!(target: "refresh_test", "after the refresh");
    let held = records(&handler, 1);
    assert_eq!(held.len(), 1);
    assert_eq!(attr::<String>(&held[0], "msg"), "after the refresh");

    // Raised again: the forwarder's own check filters until the refresh.
    Python::attach(|py| set_level(py, "refresh_test", "ERROR"));
    tracing::debug!(target: "refresh_test", "after raising");
    tracing::error!(target: "refresh_test", "an error");
    let held = records(&handler, 2);
    assert_eq!(attr::<String>(&held[1], "msg"), "an error");
}

/// Emits from inside a Python logging handler.
#[pyfunction]
fn emit_again() {
    tracing::warn!(target: "reentry_test", "from the handler");
}

#[test]
fn a_handler_that_calls_back_and_logs_again_does_not_deadlock() {
    installed();
    let handler = Python::attach(|py| {
        let scope = PyDict::new(py);
        scope
            .set_item("emit_again", wrap_pyfunction!(emit_again, py).unwrap())
            .unwrap();
        scope
            .set_item("refresh", wrap_pyfunction!(refresh_logging, py).unwrap())
            .unwrap();
        py.run(
            c"import logging
class Reenter(logging.Handler):
    def __init__(self):
        super().__init__(logging.NOTSET)
        self.records = []
    def emit(self, record):
        self.records.append(record)
        if len(self.records) == 1:
            refresh()
            emit_again()
handler = Reenter()
logger = logging.getLogger('rusty_bacnet.reentry_test')
logger.addHandler(handler)
logger.propagate = False
",
            Some(&scope),
            None,
        )
        .unwrap();
        refresh_logging(py).unwrap();
        scope.get_item("handler").unwrap().unwrap().unbind()
    });
    // Emitted by a thread while this one holds the GIL: an emitter that
    // waited for the GIL would never be joined.
    Python::attach(|_| {
        std::thread::spawn(|| tracing::warn!(target: "reentry_test", "first"))
            .join()
            .unwrap();
    });
    let held = records(&handler, 2);
    assert_eq!(attr::<String>(&held[0], "msg"), "first");
    assert_eq!(attr::<String>(&held[1], "msg"), "from the handler");
}

/// Average nanoseconds per call of `emit` over `n` calls.
fn nanos_per_call(n: u32, mut emit: impl FnMut(u32)) -> f64 {
    let start = Instant::now();
    for i in 0..n {
        emit(black_box(i));
    }
    start.elapsed().as_secs_f64() * 1e9 / f64::from(n)
}

/// The issue's bar: a disabled level costs nothing measurable. Run with
/// `--no-capture` to see the numbers.
#[test]
fn a_disabled_level_costs_a_load_not_the_gil() {
    installed();
    let handler = Python::attach(|py| {
        // The root stays at WARNING; one logger takes ERROR only.
        set_level(py, "quiet", "ERROR");
        let handler = capture(py, "loud", "WARNING");
        refresh_logging(py).unwrap();
        handler
    });
    let queued = QUEUED.load(Ordering::Relaxed);
    const N: u32 = 1_000_000;
    let baseline = nanos_per_call(N, |i| {
        black_box(i);
    });
    // Below every logger's level: the level hint stops it.
    let below_hint = nanos_per_call(N, |i| tracing::debug!(target: "quiet", i, "debug"));
    // At the hint, but not for this logger: the callsite's cached interest.
    let interest = nanos_per_call(N, |i| tracing::warn!(target: "quiet", i, "warn"));
    assert_eq!(QUEUED.load(Ordering::Relaxed), queued, "nothing was queued");
    // For contrast: enabled, so formatted and queued (and delivered).
    const ENABLED: u32 = 1_000;
    let enabled = nanos_per_call(ENABLED, |i| tracing::warn!(target: "loud", i, "warn"));
    records(&handler, ENABLED as usize);
    println!(
        "ns per call: empty loop {baseline:.1}, below the hint {below_hint:.1}, \
         disabled by interest {interest:.1}, enabled {enabled:.1}"
    );
    // Generous for debug builds on a loaded runner: an enabled event, or one
    // that took the GIL, costs far more.
    assert!(below_hint < 1_000.0, "{below_hint} ns");
    assert!(interest < 1_000.0, "{interest} ns");
}

#[test]
fn of_two_overlapping_refreshes_the_later_read_wins() {
    let (earlier, later) = (levels::next_generation(), levels::next_generation());
    levels::store(later, snapshot(10, 0, &[]));
    // The earlier read finishes last, and is not stored.
    levels::store(earlier, snapshot(40, 0, &[]));
    assert_eq!(levels::with_current(|levels| levels.root), 10);
}

fn held(handler: &Py<PyAny>) -> Vec<Py<PyAny>> {
    Python::attach(|py| {
        handler
            .bind(py)
            .getattr("records")
            .unwrap()
            .extract()
            .unwrap()
    })
}

#[test]
fn a_flush_delivers_everything_queued_before_it() {
    installed();
    let handler = Python::attach(|py| {
        let handler = capture(py, "flush_test", "DEBUG");
        refresh_logging(py).unwrap();
        handler
    });
    for i in 0..100 {
        tracing::warn!(target: "flush_test", i, "queued");
    }
    assert!(Python::attach(|py| deliver::flush(
        py,
        Duration::from_secs(10)
    )));
    // No waiting: the flush answered after the last of them.
    assert_eq!(held(&handler).len(), 100);
}

#[test]
fn records_dropped_on_a_full_queue_are_reported() {
    installed();
    let (_flood, package) = Python::attach(|py| {
        // The flood stays on its own logger; the report goes to the
        // package logger.
        let flood = capture(py, "flood_test", "WARNING");
        let package = capture(py, "", "WARNING");
        refresh_logging(py).unwrap();
        (flood, package)
    });
    // The forwarder can't take the GIL while this thread holds it, so the
    // queue fills and the rest are dropped.
    let flood = u32::try_from(QUEUE).unwrap() + 1_000;
    Python::attach(|_| {
        for i in 0..flood {
            tracing::warn!(target: "flood_test", i, "flood");
        }
    });
    assert!(Python::attach(|py| deliver::flush(
        py,
        Duration::from_secs(30)
    )));
    let report = held(&package)
        .into_iter()
        .find(|record| attr::<String>(record, "msg").contains("were dropped"))
        .expect("a drop report");
    let dropped: u64 = Python::attach(|py| {
        report
            .bind(py)
            .getattr("args")
            .unwrap()
            .get_item(0)
            .unwrap()
            .extract()
            .unwrap()
    });
    // The forwarder may have taken a batch before it blocked.
    assert!(
        (1_000 - BATCH as u64..=1_000).contains(&dropped),
        "{dropped}"
    );
}

#[test]
fn unreadable_levels_fall_back_to_the_defaults_with_a_warning() {
    Python::initialize();
    Python::attach(|py| {
        let scope = PyDict::new(py);
        let run = |code: &std::ffi::CStr| py.run(code, Some(&scope), None).unwrap();
        run(c"import logging, warnings
saved = logging.root.level
logging.root.level = 'not a level'
caught = warnings.catch_warnings(record=True)
seen = caught.__enter__()
warnings.simplefilter('always')
");
        let levels = read_or_default(py);
        run(c"caught.__exit__(None, None, None)
logging.root.level = saved
");
        assert_eq!(levels, Levels::default());
        let seen = scope.get_item("seen").unwrap().unwrap();
        assert_eq!(seen.len().unwrap(), 1);
        let message = seen.get_item(0).unwrap().getattr("message").unwrap();
        let message = message.str().unwrap().to_string();
        assert!(
            message.contains("reading the logging levels failed"),
            "{message}"
        );
    });
}
