//! Legacy subscriptions over the pipe (#2114): `resources/subscribe` and
//! `resources/unsubscribe` from a 2025-06-18 session, on the same watch hub
//! and caps as listen streams, checked against the lab device's own
//! subscription table (Active_COV_Subscriptions).

use std::sync::Arc;
use std::time::Duration;

use bacnet_ops::engine::Engine;
use bacnet_ops::watch::{Hub, WatchOptions};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;

use super::{Agent, Lab, Recording};
use crate::Policy;
use crate::follow::Follow;
use crate::policy::Caps;
use crate::resources::uri::{self, Uri};

const AI1: &str = "bacnet://device/1201/object/analog-input:1";
const AI3: &str = "bacnet://device/1201/object/ai:3";
const AO1: &str = "bacnet://device/1201/object/analog-output:1";
const UPDATED: &str = "notifications/resources/updated";

/// Let the lab's tasks run: the hub's requests are paced 50 ms apart.
async fn settle() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}

impl Agent {
    /// The 2025-06-18 handshake, as Codex opens; the answer's result.
    pub(super) async fn initialize_legacy(&mut self) -> Value {
        let reply = self
            .request(
                "initialize",
                json!({ "protocolVersion": "2025-06-18", "capabilities": {},
                        "clientInfo": { "name": "old-agent", "version": "1" } }),
            )
            .await;
        self.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        reply["result"].clone()
    }

    /// A legacy `resources/subscribe`: the result, or the error.
    pub(super) async fn subscribe(&mut self, uri: &str) -> Result<Value, Value> {
        let reply = self
            .request("resources/subscribe", json!({ "uri": uri }))
            .await;
        reply.get("result").cloned().ok_or(reply["error"].clone())
    }

    /// A legacy `resources/unsubscribe`.
    async fn unsubscribe(&mut self, uri: &str) {
        let reply = self
            .request("resources/unsubscribe", json!({ "uri": uri }))
            .await;
        assert_eq!(reply["result"], json!({}), "{reply}");
    }
}

/// The session's watch counts: points held and legacy URIs.
async fn watches(lab: &mut Lab) -> (Value, Value) {
    let session = lab.agent.read("bacnet://session").await.unwrap();
    (
        session["watches"]["held"].clone(),
        session["watches"]["legacy"].clone(),
    )
}

/// Acceptance: a 2025-06-18 client is offered subscriptions, subscribes to
/// a point (twice: one subscription), hears of a change through
/// `notifications/resources/updated`, and unsubscribing cancels the BACnet
/// subscription in the device's table.
#[tokio::test(start_paused = true)]
async fn a_legacy_client_subscribes_hears_and_unsubscribes() {
    let mut lab = Lab::start(Policy::default()).await;
    let hello = lab.agent.initialize_legacy().await;
    assert_eq!(hello["protocolVersion"], json!("2025-06-18"));
    assert_eq!(
        hello["capabilities"]["resources"]["subscribe"],
        json!(true),
        "{hello}"
    );
    lab.known().await;
    assert_eq!(lab.agent.subscribe(AI1).await, Ok(json!({})));
    assert_eq!(lab.agent.subscribe(AI1).await, Ok(json!({})));
    settle().await;
    let table = lab.cov_table().await;
    assert_eq!(table.len(), 1, "{table:?}");
    assert!(table[0].time_remaining > 0);
    assert_eq!(watches(&mut lab).await, (json!(1), json!(1)));
    let _ = lab.agent.drain(UPDATED, Duration::from_millis(10)).await;

    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    lab.server
        .set_present_value_local(&ai1, PropertyValue::Real(80.0))
        .await
        .unwrap();
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(updates[0]["params"]["uri"], json!(AI1));
    let content = lab.agent.read(AI1).await.unwrap();
    assert_eq!(super::listen::present_value(&content), json!(80.0));
    // Another subscription doesn't announce the first's change again.
    assert_eq!(lab.agent.subscribe(AI3).await, Ok(json!({})));
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert!(
        updates.iter().all(|u| u["params"]["uri"] != json!(AI1)),
        "{updates:?}"
    );
    lab.agent.unsubscribe(AI3).await;

    lab.agent.unsubscribe(AI1).await;
    // Unsubscribing what isn't subscribed is fine.
    lab.agent.unsubscribe(AI1).await;
    settle().await;
    assert!(
        lab.cov_table().await.is_empty(),
        "unsubscribing cancelled it"
    );
    assert_eq!(watches(&mut lab).await, (json!(0), json!(0)));
    lab.server
        .set_present_value_local(&ai1, PropertyValue::Real(81.0))
        .await
        .unwrap();
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert!(updates.is_empty(), "{updates:?}");
    lab.stop().await;
}

/// Acceptance: the host closing the session (stdin EOF) releases every
/// legacy subscription and cancels its BACnet subscription.
#[tokio::test(start_paused = true)]
async fn closing_the_session_releases_every_subscription() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize_legacy().await;
    lab.known().await;
    for uri in [AI1, AI3, "bacnet://devices", "bacnet://session"] {
        assert_eq!(lab.agent.subscribe(uri).await, Ok(json!({})), "{uri}");
    }
    settle().await;
    assert_eq!(lab.cov_table().await.len(), 2);
    lab.agent.write.shutdown().await.unwrap();
    (&mut lab.serving).await.unwrap();
    lab.shutdown = None;
    assert_eq!(lab.mcp.watches(), (0, 0));
    assert!(lab.cov_table().await.is_empty());
    // A subscribe that comes in after the end is refused, never kept.
    for uri in ["bacnet://session", AI1] {
        let error = lab.mcp.subscribe_now(uri).unwrap_err();
        let data = error.data.unwrap_or_default();
        assert_eq!(data["reason"], json!("shutting-down"), "{uri}: {data}");
    }
    assert_eq!(lab.mcp.watches(), (0, 0));
    lab.stop().await;
}

/// A subscription made before the session's notifier first runs (rmcp
/// doesn't wait for `notifications/initialized`) is still followed.
#[tokio::test(start_paused = true)]
async fn a_subscription_before_the_notifier_starts_is_followed() {
    let mut lab = Lab::start(Policy::default()).await;
    // The session's task hasn't run yet: nothing has awaited since it
    // was spawned.
    lab.mcp.subscribe_now("bacnet://devices").unwrap();
    lab.agent.initialize_legacy().await;
    settle().await;
    lab.client.clear_devices().await;
    lab.server.broadcast_i_am().await.unwrap();
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(updates[0]["params"]["uri"], json!("bacnet://devices"));
    lab.stop().await;
}

/// A point let go and watched again between two syncs (unsubscribed and
/// subscribed again while the notifier was busy) is followed on its new
/// watch, not the old one, which never fires again.
#[tokio::test(start_paused = true)]
async fn a_point_watched_again_is_followed_on_its_new_watch() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize_legacy().await;
    let limits = Arc::new(Policy::default().request_limits());
    let engine = Arc::new(Engine::new(Arc::clone(&lab.client), limits));
    let hub = Hub::new(Arc::clone(&engine), WatchOptions::default()).unwrap();
    let Ok(Uri::Point(point)) = uri::parse(AI1) else {
        panic!("a point URI");
    };
    let mut follow = Follow::new();
    let first = hub.watch(point).unwrap();
    let signals = crate::writes::notify::Signals::default();
    follow.set(
        Some(&*engine),
        &signals,
        vec![(AI1.to_string(), Some(first.changes()))],
    );
    assert!(first.changes().same(follow.signal(AI1).unwrap()));
    drop(first);
    let second = hub.watch(point).unwrap();
    assert!(!second.changes().same(&follow.signal(AI1).unwrap().clone()));
    follow.set(
        None::<&Engine<Recording>>,
        &signals,
        vec![(AI1.to_string(), Some(second.changes()))],
    );
    assert!(second.changes().same(follow.signal(AI1).unwrap()));
    drop(second);
    hub.shutdown(Duration::from_secs(10)).await;
    drop(hub);
    drop(engine);
    lab.stop().await;
}

/// The session's future dropped mid-session (its task aborted) still
/// releases the watches its legacy subscriptions held.
#[tokio::test(start_paused = true)]
async fn a_dropped_session_releases_its_watches() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize_legacy().await;
    lab.known().await;
    assert_eq!(lab.agent.subscribe(AI1).await, Ok(json!({})));
    assert_eq!(lab.mcp.watches(), (1, 1));
    lab.serving.abort();
    assert!((&mut lab.serving).await.unwrap_err().is_cancelled());
    assert_eq!(lab.mcp.watches(), (0, 0));
}

/// Listen streams and legacy subscriptions count against one cap; a
/// refusal is a -32602 with the tool error body, and sends nothing.
#[tokio::test(start_paused = true)]
async fn listen_streams_and_legacy_subscriptions_share_the_caps() {
    let policy = Policy {
        caps: Caps {
            watches: 2,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize_legacy().await;
    lab.known().await;
    let (_, accepted) = lab.agent.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    assert_eq!(lab.agent.subscribe(AI3).await, Ok(json!({})));
    settle().await;
    let before = lab.sends().len();
    let error = lab.agent.subscribe(AO1).await.unwrap_err();
    assert_eq!(error["code"], json!(-32602), "{error}");
    assert_eq!(error["data"]["error"], json!("refused"));
    assert_eq!(error["data"]["reason"], json!("principal-cap"));
    // The same point a stream holds counts once.
    assert_eq!(
        lab.agent
            .subscribe("bacnet://device/1201/object/analog-input:1")
            .await,
        Ok(json!({}))
    );
    let (_, accepted) = lab.agent.listen(&[AO1]).await;
    assert!(accepted.is_empty(), "{accepted:?}");
    assert_eq!(lab.sends().len(), before, "refusals send nothing");
    assert_eq!(watches(&mut lab).await, (json!(2), json!(2)));

    lab.agent.unsubscribe(AI3).await;
    let (_, accepted) = lab.agent.listen(&[AO1]).await;
    assert_eq!(accepted, [AO1]);
    settle().await;
    assert_eq!(lab.cov_table().await.len(), 2);
    lab.stop().await;
}

/// URIs that can't be honoured are refused as a resource read refuses
/// them, with nothing sent: unknown and out-of-reach devices, malformed
/// URIs, and a device summary (which has no updates).
#[tokio::test(start_paused = true)]
async fn refused_uris_are_errors_and_send_nothing() {
    let policy = Policy {
        devices: Some(vec![(1201, 1201), (300, 304)]),
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize_legacy().await;
    lab.known().await;
    let before = lab.sends().len();
    let cases = [
        ("bacnet://device/4242/object/ai:1", "unknown-device", None),
        (
            "bacnet://device/1202/object/ai:1",
            "refused",
            Some("device-not-allowed"),
        ),
        (
            "bacnet://device/1202/alarms",
            "refused",
            Some("device-not-allowed"),
        ),
        ("bacnet://nonsense", "invalid-argument", None),
        ("bacnet://device/1201", "invalid-argument", None),
    ];
    for (uri, kind, reason) in cases {
        let error = lab.agent.subscribe(uri).await.unwrap_err();
        assert_eq!(error["code"], json!(-32602), "{uri}: {error}");
        assert_eq!(error["data"]["error"], json!(kind), "{uri}: {error}");
        if let Some(reason) = reason {
            assert_eq!(error["data"]["reason"], json!(reason), "{uri}: {error}");
        }
    }
    assert_eq!(lab.sends().len(), before, "refusals send nothing");
    assert_eq!(watches(&mut lab).await, (json!(0), json!(0)));
    lab.stop().await;
}

/// A session holds at most twice the point cap in URIs; one past it is
/// refused before any watch starts, so nothing is sent.
#[tokio::test(start_paused = true)]
async fn a_session_past_its_uri_bound_is_refused_without_sending() {
    let policy = Policy {
        caps: Caps {
            watches: 1,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize_legacy().await;
    lab.known().await;
    for uri in ["bacnet://session", "bacnet://ledger"] {
        assert_eq!(lab.agent.subscribe(uri).await, Ok(json!({})), "{uri}");
    }
    let before = lab.sends().len();
    let error = lab.agent.subscribe(AI1).await.unwrap_err();
    assert_eq!(error["data"]["reason"], json!("principal-cap"), "{error}");
    settle().await;
    assert_eq!(lab.sends().len(), before, "nothing sent");
    assert_eq!(watches(&mut lab).await, (json!(0), json!(2)));
    lab.stop().await;
}

/// A passive server takes legacy subscriptions to the session and ledger
/// only.
#[tokio::test(start_paused = true)]
async fn a_passive_server_takes_only_session_and_ledger() {
    use bacnet_transport::bip::BipTransport;
    let policy = Policy {
        passive: true,
        ..Policy::default()
    };
    let (mut agent, serving) =
        Agent::connect(crate::McpServer::<BipTransport>::new(None, policy, "tester").unwrap());
    agent.initialize_legacy().await;
    for uri in ["bacnet://session", "bacnet://ledger"] {
        assert_eq!(agent.subscribe(uri).await, Ok(json!({})), "{uri}");
    }
    for uri in [
        AI1,
        "bacnet://devices",
        "bacnet://devices/after/5",
        "bacnet://alarms",
    ] {
        let error = agent.subscribe(uri).await.unwrap_err();
        assert_eq!(error["code"], json!(-32602), "{uri}: {error}");
        assert_eq!(error["data"]["reason"], json!("passive"), "{uri}: {error}");
    }
    drop(agent);
    serving.await.unwrap();
}

/// The device list is announced to a legacy subscriber as to a stream,
/// at most once a second.
#[tokio::test(start_paused = true)]
async fn the_device_list_is_announced_to_a_legacy_subscriber() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize_legacy().await;
    assert_eq!(lab.agent.subscribe("bacnet://devices").await, Ok(json!({})));
    settle().await;
    lab.client.clear_devices().await;
    for _ in 0..3 {
        lab.server.broadcast_i_am().await.unwrap();
    }
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(updates[0]["params"]["uri"], json!("bacnet://devices"));
    lab.stop().await;
}

/// A later page of the device list (#2116) is subscribed as the first is,
/// and each subscribed page is announced when the list changes; an
/// unsubscribed one isn't.
#[tokio::test(start_paused = true)]
async fn a_device_list_page_is_announced_to_a_legacy_subscriber() {
    const PAGE: &str = "bacnet://devices/after/302";
    let mut lab = Lab::start(Policy::default()).await;
    lab.agent.initialize_legacy().await;
    for uri in ["bacnet://devices", PAGE] {
        assert_eq!(lab.agent.subscribe(uri).await, Ok(json!({})), "{uri}");
    }
    settle().await;
    lab.client.clear_devices().await;
    lab.server.broadcast_i_am().await.unwrap();
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    let mut told: Vec<&str> = updates
        .iter()
        .map(|u| u["params"]["uri"].as_str().unwrap())
        .collect();
    told.sort_unstable();
    assert_eq!(told, ["bacnet://devices", PAGE], "{updates:?}");
    lab.agent.unsubscribe("bacnet://devices").await;
    settle().await;
    lab.client.clear_devices().await;
    lab.server.broadcast_i_am().await.unwrap();
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(updates[0]["params"]["uri"], json!(PAGE));
    lab.stop().await;
}

/// Device-list pages count against the session's URI bound (twice the
/// point cap) like any other resource.
#[tokio::test(start_paused = true)]
async fn device_list_pages_count_against_the_uri_bound() {
    let policy = Policy {
        caps: Caps {
            watches: 2,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.agent.initialize_legacy().await;
    let pages: Vec<String> = (0..5)
        .map(|n| format!("bacnet://devices/after/{n}"))
        .collect();
    for page in &pages[..4] {
        assert_eq!(lab.agent.subscribe(page).await, Ok(json!({})), "{page}");
    }
    let error = lab.agent.subscribe(&pages[4]).await.unwrap_err();
    assert_eq!(error["code"], json!(-32602), "{error}");
    assert_eq!(error["data"]["reason"], json!("principal-cap"), "{error}");
    // Subscribing again to a page held is still one subscription.
    assert_eq!(lab.agent.subscribe(&pages[0]).await, Ok(json!({})));
    assert_eq!(watches(&mut lab).await, (json!(0), json!(4)));
    lab.stop().await;
}

/// Per-device alarm URIs on listen streams share the session's URI bound
/// (twice the point cap) with legacy subscriptions (#2150): a stream asking
/// for more gets exactly the room left, the alarm poll reads only those
/// devices, and the room comes back when the stream goes.
#[tokio::test(start_paused = true)]
async fn listen_alarm_uris_share_the_session_uri_bound() {
    let policy = Policy {
        caps: Caps {
            watches: 2,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start_site(policy, Duration::ZERO).await;
    lab.agent.initialize_legacy().await;
    lab.known().await;
    assert_eq!(lab.agent.subscribe("bacnet://session").await, Ok(json!({})));
    let asked: Vec<String> = (1201..1206)
        .map(|device| format!("bacnet://device/{device}/alarms"))
        .collect();
    let asked: Vec<&str> = asked.iter().map(String::as_str).collect();
    let (stream, accepted) = lab.agent.listen(&asked).await;
    assert_eq!(accepted, asked[..3], "the bound of 4 less the legacy URI");
    assert_eq!(lab.mcp.alarm_state(), (4, vec![]));
    let error = lab.agent.subscribe("bacnet://ledger").await.unwrap_err();
    assert_eq!(error["data"]["reason"], json!("principal-cap"), "{error}");
    // The alarm poll reads the accepted devices and no other.
    tokio::time::sleep(Duration::from_secs(61)).await;
    assert_eq!(lab.mcp.alarm_state(), (4, vec![1201, 1202, 1203]));

    lab.agent.cancel(stream).await;
    settle().await;
    assert_eq!(lab.mcp.alarm_state().0, 1);
    assert_eq!(lab.agent.subscribe("bacnet://ledger").await, Ok(json!({})));
    lab.stop().await;
}
