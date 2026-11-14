//! The listener's own runtime (#2203): its sessions' requests run on
//! threads named `SESSION_THREADS`; a request that panics there ends only
//! its own session, audited with the reason, while another agent's goes
//! on; and the runtime's threads stop with the listener, dropped too. On
//! a real clock, since the listener's runtime doesn't share a paused one.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::LAB;
use super::attach::{audit, bridged, lab, private_dir, workspace};
use crate::Policy;
use crate::attach::{Listener, SESSION_THREADS, SessionRuntime, threads};

/// Set to make the next request a session thread sends panic.
static ARMED: AtomicBool = AtomicBool::new(false);

fn on_session_thread() -> bool {
    std::thread::current()
        .name()
        .is_some_and(|name| name.starts_with(SESSION_THREADS))
}

/// Panic, once armed, when called on a session thread: the lab's transport
/// calls it as a request leaves, standing in for a bug an agent reaches.
pub(super) fn panic_if_armed() {
    if on_session_thread() && ARMED.swap(false, Ordering::SeqCst) {
        panic!("an injected bug, reached by an agent's request");
    }
}

/// Wait on the real clock until `ok` holds, failing with `what` after
/// five seconds: the runtime's threads start and stop on their own time.
async fn eventually(what: &str, ok: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !ok() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

fn live() -> usize {
    threads::LIVE.load(Ordering::SeqCst)
}

fn read() -> Value {
    json!({ "device": LAB, "reads": [{ "object": "ai:1", "properties": ["present-value"] }] })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panicking_request_ends_only_its_session_on_the_listeners_threads() {
    // Which threads panicked, the hook otherwise as the process's.
    let panicked = Arc::new(Mutex::new(Vec::new()));
    let original = Arc::new(std::panic::take_hook());
    std::panic::set_hook({
        let (panicked, original) = (Arc::clone(&panicked), Arc::clone(&original));
        Box::new(move |info| {
            let name = std::thread::current().name().map(ToString::to_string);
            panicked.lock().unwrap().push(name);
            original(info);
        })
    });

    let lab = lab().await;
    let dir = private_dir();
    let own = SessionRuntime::Own;
    let listener = Listener::bind(
        lab.client(),
        Policy::default(),
        "tester",
        workspace(dir.path()),
        own,
        None,
    )
    .await
    .unwrap();
    eventually("its workers run", || live() >= 2).await;
    let (mut first, first_bridge) = bridged(listener.path());
    first.initialize().await;
    let (mut second, second_bridge) = bridged(listener.path());
    second.initialize().await;

    ARMED.store(true, Ordering::SeqCst);
    let id = first
        .post(
            "tools/call",
            json!({ "name": "bacnet_read", "arguments": read() }),
        )
        .await;
    // The SDK drains the answers of requests in flight as the session
    // ends, so the internal error gets out.
    let answer = first.answer(id).await;
    assert_eq!(answer["error"]["code"], json!(-32603), "{answer}");
    first_bridge.await.unwrap().unwrap();
    assert!(
        !ARMED.load(Ordering::SeqCst),
        "the request ran on a session thread"
    );
    let names = panicked.lock().unwrap().clone();
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(
        names[0]
            .as_deref()
            .is_some_and(|n| n.starts_with(SESSION_THREADS)),
        "{names:?}"
    );

    let other = second.call("bacnet_read", read()).await;
    assert_eq!(
        other["isError"],
        json!(false),
        "the other agent goes on: {other}"
    );

    listener.shutdown().await;
    let live = threads::LIVE.load(Ordering::SeqCst);
    assert_eq!(live, 0, "its threads stopped with it");
    second_bridge.await.unwrap().unwrap();
    drop(std::panic::take_hook());
    std::panic::set_hook(Box::new(move |info| original(info)));
    let records = audit(dir.path());
    let first_close = records
        .iter()
        .find(|r| r["event"] == json!("mcp-disconnect") && r["connection"] == json!(1))
        .unwrap_or_else(|| panic!("{records:?}"));
    let why = first_close["error"].as_str().unwrap_or_default();
    assert!(why.contains("panicked"), "{first_close}");
}

/// A listener dropped without its shutdown lets a session still ending
/// finish before its runtime stops, so the agent's close is on record.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_listener_lets_its_sessions_end_on_record() {
    // The session is still ending when the hub's shutdown is done.
    threads::END_DELAY_MS.store(300, Ordering::SeqCst);
    // Back to none however the test ends, for runs that share a process.
    struct NoDelay;
    impl Drop for NoDelay {
        fn drop(&mut self) {
            threads::END_DELAY_MS.store(0, Ordering::SeqCst);
        }
    }
    let _no_delay = NoDelay;
    let lab = lab().await;
    let dir = private_dir();
    let listener = Listener::bind(
        lab.client(),
        Policy::default(),
        "tester",
        workspace(dir.path()),
        SessionRuntime::Own,
        None,
    )
    .await
    .unwrap();
    let (mut agent, bridge) = bridged(listener.path());
    agent.initialize().await;
    drop(listener);
    eventually("its threads stopped", || live() == 0).await;
    // The runtime's stop drops the socket: the bridge may see it close or,
    // on Linux, be reset. The claim is the record below.
    if let Err(error) = bridge.await.unwrap() {
        let error = error.to_string();
        assert!(
            error.contains("the connection to the TUI failed"),
            "{error}"
        );
    }
    let records = audit(dir.path());
    assert!(
        records
            .iter()
            .any(|r| r["event"] == json!("mcp-disconnect") && r["connection"] == json!(1)),
        "{records:?}"
    );
}
