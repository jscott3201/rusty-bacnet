//! TUI-attached mode, the second review round (#2185): the principal's
//! minute gates and point cap shared by every session of a listener,
//! shutdown not waiting out a silent handshake, a point two agents share,
//! an opened session left idle, and the eight seats.

use std::time::Duration;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::json;
use tokio::io::AsyncReadExt as _;
use tokio::time::Instant;

use super::Agent;
use super::attach::{audit, bridged, events, lab, private_dir, raw};
use super::attach_more::agents_are;
use crate::Policy;
use crate::attach::{Listener, MAX_AGENTS, socket_path};
use crate::policy::Caps;

const AI1: &str = "bacnet://device/1201/object/analog-input:1";
const AI3: &str = "bacnet://device/1201/object/ai:3";
const UPDATED: &str = "notifications/resources/updated";

/// An agent through the bridge, its handshake done and the lab known.
async fn agent(listener: &Listener) -> (Agent, tokio::task::JoinHandle<()>) {
    let (mut agent, bridge) = bridged(listener.path());
    agent.initialize().await;
    let devices = agent
        .modern("resources/read", json!({ "uri": "bacnet://devices" }))
        .await;
    assert!(devices.get("result").is_some(), "{devices}");
    let bridge = tokio::spawn(async move { bridge.await.unwrap().unwrap() });
    (agent, bridge)
}

/// Acceptance: shutting the listener down while a connection sits silent
/// before its handshake ends that session at once, well inside the 30 s
/// wait, and removes the socket; the close is on record without an error.
#[tokio::test(start_paused = true)]
async fn shutdown_does_not_wait_out_a_silent_handshake() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    let path = socket_path(dir.path()).unwrap();
    let mut idle = raw(&path).await;
    agents_are(&listener, 1).await;
    let start = Instant::now();
    listener.shutdown().await;
    let took = Instant::now() - start;
    assert!(took < Duration::from_secs(5), "shutdown took {took:?}");
    #[cfg(unix)]
    assert!(!path.exists());
    let mut answer = Vec::new();
    idle.read_to_end(&mut answer).await.unwrap();
    assert!(answer.is_empty());
    assert_eq!(events(dir.path()), ["mcp-connect", "mcp-disconnect"]);
    assert!(audit(dir.path())[1].get("error").is_none());
    lab.stop().await;
}

/// Acceptance: the agents are one principal, so a second session's broad
/// Who-Is within the minute is refused as the first session's would be,
/// and the principal's watch cap counts both sessions' points (a point
/// one already holds still fits the other).
#[tokio::test(start_paused = true)]
async fn the_minute_gates_and_the_watch_cap_span_the_sessions() {
    let lab = lab().await;
    let dir = private_dir();
    let policy = Policy {
        broad_discovery: true,
        caps: Caps {
            watches: 1,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let listener = lab.attach(dir.path(), policy).await.unwrap();
    let (mut first, first_bridge) = agent(&listener).await;
    let (mut second, second_bridge) = agent(&listener).await;
    let found = first.call("bacnet_discover", json!({ "wait_s": 1 })).await;
    assert_eq!(found["isError"], json!(false), "{found}");
    let refused = second.call("bacnet_discover", json!({ "wait_s": 1 })).await;
    let reason = &refused["structuredContent"]["reason"];
    assert_eq!(reason, &json!("rate-limited"), "{refused}");

    let (_, accepted) = first.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    let (_, accepted) = second.listen(&[AI3, AI1]).await;
    assert_eq!(accepted, [AI1], "AI3 is past the principal's cap");
    let session = second.read("bacnet://session").await.unwrap();
    assert_eq!(session["watches"]["held"], json!(1));
    for (mut agent, bridge) in [(first, first_bridge), (second, second_bridge)] {
        agent.close().await;
        bridge.await.unwrap();
    }
    listener.shutdown().await;
    lab.stop().await;
}

/// Two agents watching one point share its subscription; when one leaves,
/// the other keeps its COV updates. An opened session idle past the
/// initialize wait isn't closed.
#[tokio::test(start_paused = true)]
async fn an_agent_keeps_a_shared_point_when_the_other_leaves() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    let (mut first, first_bridge) = agent(&listener).await;
    let (mut second, second_bridge) = agent(&listener).await;
    tokio::time::sleep(Duration::from_secs(45)).await;
    assert_eq!(*listener.agents().borrow(), 2, "idle but open");

    assert_eq!(first.listen(&[AI1]).await.1, [AI1]);
    assert_eq!(second.listen(&[AI1]).await.1, [AI1]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(lab.cov_table().await.len(), 1, "one subscription");
    first.close().await;
    first_bridge.await.unwrap();
    agents_are(&listener, 1).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(lab.cov_table().await.len(), 1, "still subscribed");
    second.drain(UPDATED, Duration::ZERO).await;
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    lab.server
        .set_present_value_local(&ai1, PropertyValue::Real(42.0))
        .await
        .unwrap();
    let updated = tokio::time::timeout(Duration::from_secs(5), second.notification(UPDATED));
    let updated = updated.await.expect("an update after the other left");
    assert_eq!(updated["params"]["uri"], json!(AI1));
    second.close().await;
    second_bridge.await.unwrap();
    listener.shutdown().await;
    lab.stop().await;
}

/// Eight agents may attach at once: a ninth connection is closed with
/// nothing written and audited as `too-many-agents`, and once a seat is
/// given back another agent gets it.
#[tokio::test(start_paused = true)]
async fn a_ninth_agent_is_refused_until_a_seat_is_free() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    let mut agents = Vec::new();
    for _ in 0..MAX_AGENTS {
        let (mut agent, bridge) = bridged(listener.path());
        agent.initialize().await;
        agents.push((agent, bridge));
    }
    agents_are(&listener, MAX_AGENTS).await;
    let mut ninth = raw(listener.path()).await;
    let mut answer = Vec::new();
    // Closed, not left waiting; a reset counts as closed too.
    let read = tokio::time::timeout(Duration::from_secs(5), ninth.read_to_end(&mut answer));
    assert!(read.await.is_ok(), "the ninth is still open");
    assert!(answer.is_empty(), "{}", String::from_utf8_lossy(&answer));
    assert_eq!(*listener.agents().borrow(), MAX_AGENTS);

    let (mut leaving, bridge) = agents.pop().unwrap();
    leaving.close().await;
    let _ = bridge.await;
    agents_are(&listener, MAX_AGENTS - 1).await;
    let (mut next, bridge) = bridged(listener.path());
    let hello = next.initialize().await;
    assert!(
        hello.get("result").is_some(),
        "a freed seat is taken: {hello}"
    );
    agents_are(&listener, MAX_AGENTS).await;
    agents.push((next, bridge));

    listener.shutdown().await;
    // A listener going away may reset a bridge's connection: the records,
    // not each bridge's clean end, are the evidence.
    for (_agent, bridge) in agents {
        let _ = bridge.await;
    }
    let records = audit(dir.path());
    let refused: Vec<_> = records
        .iter()
        .filter(|record| record["event"] == json!("mcp-refused"))
        .collect();
    assert_eq!(refused.len(), 1, "{records:?}");
    assert_eq!(refused[0]["outcome"]["why"], json!("too-many-agents"));
    let connects = records
        .iter()
        .filter(|record| record["event"] == json!("mcp-connect"))
        .count();
    assert_eq!(connects, MAX_AGENTS + 1, "{records:?}");
    lab.stop().await;
}
