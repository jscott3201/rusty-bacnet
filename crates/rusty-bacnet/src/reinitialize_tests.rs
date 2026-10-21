//! The Python handler adapter in isolation: the notification is registered
//! on the after-reply hook, never scheduled from the handler itself (#1566).
use super::*;
use bacnet_transport::port::TransportProvenance;
use pyo3::types::PyDict;

/// A fake event loop whose `call_soon_threadsafe` only records its calls,
/// and the two callables: one that accepts, one for the notification.
fn fixture(py: Python<'_>) -> (Arc<PyReinitialize>, Bound<'_, PyAny>) {
    let scope = PyDict::new(py);
    py.run(
        c"class Loop:\n    def __init__(self):\n        self.calls = []\n    def call_soon_threadsafe(self, *args):\n        self.calls.append(args)\nloop = Loop()\naccept = lambda request: None\nnotify = lambda request: None\n",
        Some(&scope),
        None,
    )
    .unwrap();
    let get = |name: &str| scope.get_item(name).unwrap().unwrap();
    let callables = parse(Some(&get("accept")), Some(&get("notify")))
        .unwrap()
        .unwrap();
    let event_loop = get("loop");
    callables.set_event_loop(py, event_loop.clone());
    (callables, event_loop)
}

fn scheduled(event_loop: &Bound<'_, PyAny>) -> usize {
    event_loop.getattr("calls").unwrap().len().unwrap()
}

#[test]
fn an_accepted_request_schedules_its_notification_only_from_the_hook() {
    Python::initialize();
    let (callables, event_loop) = Python::attach(|py| {
        let (callables, event_loop) = fixture(py);
        (callables, event_loop.unbind())
    });
    let context = ReinitializeContext::new(
        ReinitializedState::WARMSTART,
        MacAddr::from_slice(&[7]),
        None,
        TransportProvenance::unverified(),
        3,
    );
    // No Tokio runtime here: the hook schedules inline, so the count below
    // is exact.
    assert!(callables.decide(&context).is_ok());
    Python::attach(|py| assert_eq!(scheduled(event_loop.bind(py)), 0, "not before the reply"));
    assert_eq!(context.run_after_reply_for_test(), 1, "one hook registered");
    Python::attach(|py| assert_eq!(scheduled(event_loop.bind(py)), 1));
}

#[test]
fn a_coroutine_answer_is_refused_and_closed() {
    Python::initialize();
    Python::attach(|py| {
        let scope = PyDict::new(py);
        py.run(
            c"async def handler(request):\n    return None\ncoroutine = handler(None)\n",
            Some(&scope),
            None,
        )
        .unwrap();
        let coroutine = scope.get_item("coroutine").unwrap().unwrap();
        let error = answer(&coroutine).err().expect("refused");
        assert!(error.to_string().contains("plain function"), "{error}");
        // Closed: its frame is gone, so no never-awaited warning follows.
        assert!(coroutine.getattr("cr_frame").unwrap().is_none());
    });
}
