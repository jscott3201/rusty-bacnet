//! TUI-attached mode, the review round (#2185): the `[mcp]` limits only
//! while an agent is attached, one budget for every agent, subscriber ids
//! that can't clash with the TUI's own hub, the bridge's listener check,
//! refused credentials, and the initialize deadline.

use std::sync::Arc;
use std::time::Duration;

use bacnet_ops::engine::Engine;
use bacnet_ops::limits::RequestLimits;
use bacnet_ops::watch::{Hub, Point, WatchOptions, Watched};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::json;
use tokio::io::AsyncReadExt as _;
use tokio::time::Instant;

use super::attach::{bridged, events, lab, private_dir, workspace};
use super::{ALIASES, LAB};
use crate::Policy;
use crate::attach::{self, Listener, PeerId, socket_path};
use crate::policy::Rates;

const AI1: &str = "bacnet://device/1201/object/analog-input:1";

fn ai1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

/// How far apart three turns under `limits` go, in milliseconds, after
/// which the limits sit idle a second.
async fn spacing(limits: &RequestLimits) -> u128 {
    let start = Instant::now();
    for device in 7_000..7_003 {
        drop(limits.turn(device).await);
    }
    let spent = Instant::now() - start;
    tokio::time::sleep(Duration::from_secs(1)).await;
    spent.as_millis()
}

/// Wait until the listener counts `agents`.
pub(super) async fn agents_are(listener: &Listener, agents: usize) {
    let mut count = listener.agents();
    count.wait_for(|n| *n == agents).await.unwrap();
}

/// Acceptance (maintainer, 2026-10-31): the TUI runs at its own rate
/// alone, at the stricter `[mcp]` rate while an agent is attached, and at
/// its own again once the agent has gone.
#[tokio::test(start_paused = true)]
async fn the_mcp_limits_apply_only_while_an_agent_is_attached() {
    let lab = lab().await;
    let limits = Arc::new(RequestLimits::new(50));
    let engine = Arc::new(Engine::new(Arc::clone(&lab.client), Arc::clone(&limits)));
    let dir = private_dir();
    let policy = Policy {
        rates: Rates {
            global: 10,
            ..Rates::default()
        },
        ..Policy::default()
    };
    let listener = Listener::bind(Arc::clone(&engine), policy, "tester", workspace(dir.path()))
        .await
        .unwrap();
    assert_eq!(spacing(&limits).await, 40, "the TUI's own 50 a second");
    let (mut agent, bridge) = bridged(listener.path());
    agent.initialize().await;
    agents_are(&listener, 1).await;
    assert_eq!(spacing(&limits).await, 200, "the table's 10 a second");
    agent.close().await;
    bridge.await.unwrap().unwrap();
    agents_are(&listener, 0).await;
    assert_eq!(spacing(&limits).await, 40, "its own again");
    listener.shutdown().await;
    drop(engine);
    lab.stop().await;
}

/// Four agents flooding reads share one principal bucket (5 a second,
/// burst 20), so the TUI keeps most of the global 20 a second: its 20
/// reads need 20 slots while the agents take at most 20 + 5 per second of
/// them, so they finish within (20 + 20) / (20 - 5) s, about 2.7 s. With a
/// bucket each, the agents would take every slot they could queue for.
#[tokio::test(start_paused = true)]
async fn agents_share_one_budget_and_the_tui_keeps_its_share() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    let mut agents = Vec::new();
    for device in ALIASES {
        let (mut agent, bridge) = bridged(listener.path());
        agent.initialize().await;
        for _ in 0..60 {
            let read = json!({ "device": device,
                "reads": [{ "object": "ai:1", "properties": ["present-value"] }] });
            agent
                .post(
                    "tools/call",
                    json!({ "name": "bacnet_read", "arguments": read }),
                )
                .await;
        }
        agents.push((agent, bridge));
    }
    while lab.sends().len() < 8 {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let engine = lab.engine();
    let start = Instant::now();
    for _ in 0..20 {
        engine
            .read_property(LAB, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
            .unwrap();
    }
    let took = Instant::now() - start;
    assert!(
        took <= Duration::from_secs(3),
        "the TUI's reads took {took:?}"
    );
    for (mut agent, bridge) in agents {
        agent.close().await;
        bridge.await.unwrap().unwrap();
    }
    listener.shutdown().await;
    drop(engine);
    lab.stop().await;
}

/// Acceptance: the TUI's watch hub and the agents' hub on one engine use
/// different subscriber process ids, so an agent leaving a point the TUI
/// watches too cancels only its own subscription, and the TUI's keeps
/// its COV notifications.
#[tokio::test(start_paused = true)]
async fn the_tui_keeps_its_cov_when_an_agent_leaves_the_same_point() {
    let lab = lab().await;
    let engine = lab.engine();
    let tui = Hub::new(Arc::clone(&engine), WatchOptions::default()).unwrap();
    let dir = private_dir();
    let listener = Listener::bind(
        Arc::clone(&engine),
        Policy::default(),
        "tester",
        workspace(dir.path()),
    )
    .await
    .unwrap();
    let point = Point {
        device: LAB,
        object: ai1(),
        watched: Watched::Object,
    };
    let lease = tui.watch(point).unwrap();
    let (mut agent, bridge) = bridged(listener.path());
    let devices = agent
        .modern("resources/read", json!({ "uri": "bacnet://devices" }))
        .await;
    assert!(devices.get("result").is_some(), "{devices}");
    let (_, accepted) = agent.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(lab.cov_table().await.len(), 2, "one subscription each");

    agent.close().await;
    bridge.await.unwrap().unwrap();
    agents_are(&listener, 0).await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(lab.cov_table().await.len(), 1, "the TUI's stays");
    let before = lease.snapshot().notifications;
    lab.server
        .set_present_value_local(&ai1(), PropertyValue::Real(42.0))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        lease.snapshot().notifications > before,
        "and still notifies"
    );
    drop(lease);
    tui.shutdown(Duration::from_secs(5)).await;
    listener.shutdown().await;
    drop((tui, engine));
    lab.stop().await;
}

/// The bridge refuses a listener the credentials name as another user
/// (a stand-in for the kernel's answer) and writes it nothing.
#[tokio::test]
async fn the_bridge_refuses_a_listener_of_another_user() {
    fn stranger(_: &tokio::net::UnixStream) -> std::io::Result<PeerId> {
        Ok(PeerId {
            uid: attach::own_uid().wrapping_add(1),
            pid: Some(4242),
        })
    }
    let dir = private_dir();
    let path = dir.path().join("other.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let connecting = tokio::spawn(async move { attach::connect_with(&path, stranger).await });
    let (mut accepted, _) = listener.accept().await.unwrap();
    let error = connecting.await.unwrap().unwrap_err().to_string();
    assert!(error.contains("not this user"), "{error}");
    let mut written = Vec::new();
    accepted.read_to_end(&mut written).await.unwrap();
    assert!(written.is_empty(), "{written:?}");
}

/// A peer whose credentials can't be read is refused and audited: as
/// gone when the OS says it has closed, else as unidentified.
#[tokio::test(start_paused = true)]
async fn unreadable_credentials_are_refused() {
    fn closed(_: &tokio::net::UnixStream) -> std::io::Result<PeerId> {
        Err(std::io::ErrorKind::NotConnected.into())
    }
    fn unreadable(_: &tokio::net::UnixStream) -> std::io::Result<PeerId> {
        Err(std::io::Error::other("no credentials"))
    }
    let lab = lab().await;
    for (credentials, why) in [
        (closed as fn(&_) -> _, "peer-closed"),
        (unreadable, "no-peer-credentials"),
    ] {
        let dir = private_dir();
        let listener = Listener::bind_with(
            lab.engine(),
            Policy::default(),
            "tester".into(),
            workspace(dir.path()),
            credentials,
        )
        .await
        .unwrap();
        let mut stream = tokio::net::UnixStream::connect(listener.path())
            .await
            .unwrap();
        let mut answer = Vec::new();
        let closed = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut answer));
        assert!(closed.await.is_ok() && answer.is_empty());
        listener.shutdown().await;
        let records = super::attach::audit(dir.path());
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0]["outcome"]["why"], json!(why));
    }
    lab.stop().await;
}

/// A connection that sends nothing is closed after 30 s, its seat given
/// back and its close on record.
#[tokio::test(start_paused = true)]
async fn an_idle_connection_is_closed_after_the_initialize_wait() {
    let lab = lab().await;
    let dir = private_dir();
    let listener = lab.attach(dir.path(), Policy::default()).await.unwrap();
    let mut idle = tokio::net::UnixStream::connect(socket_path(dir.path()))
        .await
        .unwrap();
    agents_are(&listener, 1).await;
    let mut answer = Vec::new();
    let closed = tokio::time::timeout(Duration::from_secs(60), idle.read_to_end(&mut answer));
    assert!(closed.await.is_ok(), "still open after a minute");
    agents_are(&listener, 0).await;
    listener.shutdown().await;
    assert_eq!(events(dir.path()), ["mcp-connect", "mcp-disconnect"]);
    let records = super::attach::audit(dir.path());
    let error = records[1]["error"].as_str().unwrap();
    assert!(error.contains("within 30 s"), "{error}");
    lab.stop().await;
}
