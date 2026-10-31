//! Subscriptions over the pipe (#2067): `subscriptions/listen` streams on
//! the watch hub, checked against the lab device's own subscription table
//! (Active_COV_Subscriptions), not only the server's counters.

use std::time::Duration;

use bacnet_encoding::constructed::decode_cov_subscription;
use bacnet_types::constructed::BACnetCOVSubscription;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use serde_json::{Value, json};

use super::{LAB, Lab};
use crate::Policy;
use crate::policy::Caps;

const AI1: &str = "bacnet://device/1201/object/analog-input:1";
const AI3: &str = "bacnet://device/1201/object/ai:3";
const AO1: &str = "bacnet://device/1201/object/analog-output:1";
const UPDATED: &str = "notifications/resources/updated";

/// Let the lab's tasks run: the watch hub's requests are paced by the
/// limits, 50 ms apart at the global rate.
async fn settle() {
    tokio::time::sleep(Duration::from_secs(1)).await;
}

impl Lab {
    /// The lab device's subscription table.
    pub(crate) async fn cov_table(&self) -> Vec<BACnetCOVSubscription> {
        let device = ObjectIdentifier::new(ObjectType::DEVICE, LAB).unwrap();
        let value = self
            .server
            .read_local(&device, PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS, None)
            .await
            .unwrap();
        let PropertyValue::ApplicationData(data) = value else {
            panic!("an encoded list: {value:?}");
        };
        let mut entries = Vec::new();
        let mut at = 0;
        while at < data.len() {
            let (entry, next) = decode_cov_subscription(&data, at).unwrap();
            entries.push(entry);
            at = next;
        }
        entries
    }

    /// Make the lab's devices known to the server, as a discovery would.
    pub(crate) async fn known(&mut self) {
        let devices = self
            .agent
            .modern("resources/read", json!({ "uri": "bacnet://devices" }))
            .await;
        assert!(devices.get("result").is_some(), "{devices}");
    }
}

/// The present value in a point resource's content.
pub(crate) fn present_value(content: &Value) -> Value {
    content["values"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["property"] == json!("present-value"))
        .map(|row| row["value"]["typed"].clone())
        .unwrap_or(Value::Null)
}

/// Acceptance: two listen streams on one point make one BACnet
/// subscription, and the last stream leaving cancels it, as the lab
/// device's table shows; a watched point is read without sending.
#[tokio::test(start_paused = true)]
async fn two_streams_share_one_subscription_until_the_last_is_cancelled() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let (first, accepted) = lab.agent.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    let (second, accepted) = lab.agent.listen(&[AI1, "bacnet://session"]).await;
    assert_eq!(accepted, [AI1, "bacnet://session"]);
    settle().await;
    let table = lab.cov_table().await;
    assert_eq!(table.len(), 1, "{table:?}");
    assert!(
        table[0].time_remaining > 0,
        "never an indefinite subscription"
    );
    let before = lab.sends().len();
    let content = lab.agent.read(AI1).await.unwrap();
    assert_eq!(
        lab.sends().len(),
        before,
        "a watched point is read from the hub"
    );
    assert_eq!(content["watched"], json!(true));
    assert_eq!(content["source"], json!("cov"));
    assert_eq!(content["state"], json!("live"));
    assert_eq!(present_value(&content), json!(72.5));
    assert_eq!(content["cost"]["requests"], json!(0));

    lab.agent.cancel(first).await;
    settle().await;
    assert_eq!(lab.cov_table().await.len(), 1, "one stream still holds it");
    lab.agent.cancel(second).await;
    settle().await;
    assert!(
        lab.cov_table().await.is_empty(),
        "the last stream cancelled it"
    );
    lab.stop().await;
}

/// The acknowledgment lists only what is honoured: past the principal's
/// cap, unknown devices, devices out of reach, malformed URIs and a device
/// summary (which has no updates) are left out.
#[tokio::test(start_paused = true)]
async fn the_acknowledgment_leaves_out_what_is_not_honoured() {
    let policy = Policy {
        devices: Some(vec![(1201, 1201), (300, 304)]),
        caps: Caps {
            watches: 2,
            ..Caps::default()
        },
        ..Policy::default()
    };
    let mut lab = Lab::start(policy).await;
    lab.known().await;
    let asked = [
        "bacnet://device/1202/object/ai:1",
        AI1,
        AI3,
        AO1,
        "bacnet://device/4242/object/ai:1",
        "bacnet://device/1201",
        "bacnet://nonsense",
        "bacnet://ledger",
        AI1,
    ];
    let (_, accepted) = lab.agent.listen(&asked).await;
    // The SDK keeps a URI asked for twice twice; it is honoured once.
    assert_eq!(accepted, [AI1, AI3, "bacnet://ledger", AI1]);
    settle().await;
    assert_eq!(lab.cov_table().await.len(), 2);
    // The same point again counts once against the cap; a new one doesn't fit.
    let (_, accepted) = lab.agent.listen(&[AI3, AO1]).await;
    assert_eq!(accepted, [AI3]);
    let session = lab.agent.read("bacnet://session").await.unwrap();
    assert_eq!(session["watches"]["held"], json!(2));
    assert_eq!(session["watches"]["max"], json!(2));
    assert_eq!(session["cov"]["subscribed"], json!(2));
    // Out of reach is refused when read as well, with nothing sent.
    let error = lab
        .agent
        .read("bacnet://device/1202/object/ai:1")
        .await
        .unwrap_err();
    assert_eq!(error["code"], json!(-32602));
    assert_eq!(error["data"]["reason"], json!("device-not-allowed"));
    lab.stop().await;
}

/// Acceptance: a device that refuses COV for an object is polled, and the
/// content says `source: poll`.
#[tokio::test(start_paused = true)]
async fn a_point_the_device_wont_subscribe_is_polled() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let device = "bacnet://device/1201/object/device:1201";
    let (_, accepted) = lab.agent.listen(&[device]).await;
    assert_eq!(accepted, [device]);
    settle().await;
    let content = lab.agent.read(device).await.unwrap();
    assert_eq!(content["source"], json!("poll"), "{content}");
    assert!(
        content["note"].as_str().unwrap().starts_with("COV refused"),
        "{content}"
    );
    assert!(lab.cov_table().await.is_empty());
    tokio::time::sleep(Duration::from_secs(11)).await;
    let content = lab.agent.read(device).await.unwrap();
    assert_eq!(content["updates"]["polls"], json!(1));
    lab.stop().await;
}

/// Acceptance: a burst of changes makes few notifications, each naming the
/// URI only; the folded changes are counted, and a read afterwards has the
/// latest value.
#[tokio::test(start_paused = true)]
async fn a_notification_storm_is_coalesced() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let (_, accepted) = lab.agent.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    settle().await;
    let start = lab.agent.drain(UPDATED, Duration::from_millis(10)).await;
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    for step in 0..30u8 {
        lab.server
            .set_present_value_local(&ai1, PropertyValue::Real(f32::from(step) + 100.0))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert!(
        (1..=3).contains(&updates.len()),
        "{} notifications after {} at start",
        updates.len(),
        start.len()
    );
    for update in &updates {
        assert_eq!(update["params"]["uri"], json!(AI1));
        assert!(update["params"].get("contents").is_none(), "the URI only");
    }
    let content = lab.agent.read(AI1).await.unwrap();
    assert_eq!(present_value(&content), json!(129.0));
    assert!(
        content["updates"]["coalesced"].as_u64().unwrap() >= 25,
        "{content}"
    );
    lab.stop().await;
}

/// Acceptance: a graceful shutdown completes each listen stream with its
/// final result and cancels every subscription.
#[tokio::test(start_paused = true)]
async fn shutdown_completes_streams_and_cancels_subscriptions() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let (first, accepted) = lab.agent.listen(&[AI1, AI3]).await;
    assert_eq!(accepted.len(), 2);
    let (second, accepted) = lab.agent.listen(&[AO1]).await;
    assert_eq!(accepted.len(), 1);
    settle().await;
    assert_eq!(lab.cov_table().await.len(), 3);
    let asked = tokio::time::Instant::now();
    lab.shut_down().await;
    // The streams complete at once, not when the session's cancellation
    // reaches them.
    assert!(
        asked.elapsed() < Duration::from_secs(2),
        "{:?}",
        asked.elapsed()
    );
    for id in [first, second] {
        let done = lab.agent.answer(id).await;
        assert_eq!(done["result"]["resultType"], json!("complete"), "{done}");
    }
    assert!(lab.cov_table().await.is_empty());
    lab.stop().await;
}

/// A device that goes silent: its subscription's renewal fails and the
/// point says it is unreachable, polled.
#[tokio::test(start_paused = true)]
async fn a_silent_device_is_reported_on_its_point() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let (_, _) = lab.agent.listen(&[AI1]).await;
    settle().await;
    lab.go_silent_after(0);
    tokio::time::sleep(Duration::from_secs(250)).await;
    // The hub's own state, read without sending.
    let content = lab.agent.read(AI1).await.unwrap();
    assert_eq!(content["state"], json!("unreachable"), "{content}");
    assert_eq!(content["source"], json!("poll"));
    lab.stop().await;
}

/// Passive mode lists only the resources that send nothing, refuses a
/// point, and honours only the session and ledger.
#[tokio::test(start_paused = true)]
async fn a_passive_server_offers_only_what_sends_nothing() {
    use bacnet_transport::bip::BipTransport;
    let policy = Policy {
        passive: true,
        ..Policy::default()
    };
    let (mut agent, serving) = super::Agent::connect(
        crate::McpServer::<BipTransport>::new(None, policy, "tester").unwrap(),
    );
    let listed = agent.modern("resources/list", json!({})).await;
    let uris: Vec<&str> = listed["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert_eq!(uris, ["bacnet://session", "bacnet://ledger"]);
    let templates = agent.modern("resources/templates/list", json!({})).await;
    assert_eq!(templates["result"]["resourceTemplates"], json!([]));
    let error = agent.read(AI1).await.unwrap_err();
    assert_eq!(error["data"]["reason"], json!("passive"), "{error}");
    let (_, accepted) = agent
        .listen(&[AI1, "bacnet://devices", "bacnet://session"])
        .await;
    assert_eq!(accepted, ["bacnet://session"]);
    let session = agent.read("bacnet://session").await.unwrap();
    assert_eq!(session["level"], json!("passive"));
    drop(agent);
    serving.await.unwrap();
}

/// The device list is announced when a device arrives, at most once a
/// second.
#[tokio::test(start_paused = true)]
async fn the_device_list_is_announced_when_a_device_arrives() {
    let mut lab = Lab::start(Policy::default()).await;
    let (_, accepted) = lab.agent.listen(&["bacnet://devices"]).await;
    assert_eq!(accepted, ["bacnet://devices"]);
    lab.client.clear_devices().await;
    for _ in 0..3 {
        lab.server.broadcast_i_am().await.unwrap();
    }
    let updates = lab.agent.drain(UPDATED, Duration::from_secs(3)).await;
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(updates[0]["params"]["uri"], json!("bacnet://devices"));
    let devices = lab.agent.read("bacnet://devices").await.unwrap();
    assert_eq!(devices["devices"][0]["instance"], json!(LAB));
    lab.stop().await;
}

/// A reservation no stream claims (its acknowledgment failed, or the
/// client cancelled first) is released by the server's sweep, with no
/// later subscription to clear it: its subscription is cancelled and the
/// principal's cap freed.
#[tokio::test(start_paused = true)]
async fn a_reservation_never_claimed_is_released() {
    use rmcp::ServerHandler;
    use rmcp::model::SubscriptionFilter;
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let filter = SubscriptionFilter::builder()
        .resource_subscriptions([AI1])
        .build();
    let accepted = lab.mcp.accepted_subscription_filter(&filter).unwrap();
    assert_eq!(accepted.resource_subscriptions, Some(vec![AI1.to_string()]));
    settle().await;
    assert_eq!(lab.cov_table().await.len(), 1);
    tokio::time::sleep(Duration::from_secs(95)).await;
    assert!(lab.cov_table().await.is_empty());
    let session = lab.agent.read("bacnet://session").await.unwrap();
    assert_eq!(session["watches"]["held"], json!(0));
    lab.stop().await;
}

/// A session runs at most four listen streams (#2150): a fifth is refused
/// with a `stream-cap` error and takes no watch, and cancelling one makes
/// room again.
#[tokio::test(start_paused = true)]
async fn a_session_runs_at_most_four_listen_streams() {
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let mut streams = Vec::new();
    for _ in 0..crate::listen::MAX_STREAMS {
        let (id, accepted) = lab.agent.listen(&["bacnet://session"]).await;
        assert_eq!(accepted, ["bacnet://session"]);
        streams.push(id);
    }
    settle().await;
    let before = lab.sends().len();
    let (extra, accepted) = lab.agent.listen(&[AI1]).await;
    assert!(
        accepted.is_empty(),
        "the acknowledgment accepts nothing: {accepted:?}"
    );
    let refused = lab.agent.answer(extra).await;
    assert_eq!(refused["error"]["code"], json!(-32602), "{refused}");
    assert_eq!(refused["error"]["data"]["error"], json!("refused"));
    assert_eq!(refused["error"]["data"]["reason"], json!("stream-cap"));
    settle().await;
    assert_eq!(lab.sends().len(), before, "a refused stream sends nothing");
    let session = lab.agent.read("bacnet://session").await.unwrap();
    assert_eq!(session["watches"]["held"], json!(0), "{session}");
    assert_eq!(session["watches"]["streams"], json!(4), "{session}");

    lab.agent.cancel(streams.pop().unwrap()).await;
    settle().await;
    let (_, accepted) = lab.agent.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    settle().await;
    assert_eq!(lab.cov_table().await.len(), 1);
    lab.stop().await;
}

/// A stream whose watches were set aside under the cap, but which finds
/// the cap reached when it begins, is refused and lets them go at once:
/// its point's subscription is cancelled and its alarm URI's room freed,
/// with no wait for the reservation sweep (#2150).
#[tokio::test(start_paused = true)]
async fn a_stream_refused_at_its_start_frees_what_was_set_aside() {
    use rmcp::ServerHandler;
    use rmcp::model::SubscriptionFilter;
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.known().await;
    let uris = [AI1.to_string(), "bacnet://device/1201/alarms".to_string()];
    let filter = SubscriptionFilter::builder()
        .resource_subscriptions(uris.clone())
        .build();
    let accepted = lab.mcp.accepted_subscription_filter(&filter).unwrap();
    assert_eq!(accepted.resource_subscriptions.as_deref(), Some(&uris[..]));
    settle().await;
    let ctx = lab.mcp.ctx();
    assert_eq!((ctx.subs.held(), ctx.subs.uris()), (1, 1));
    assert_eq!(lab.cov_table().await.len(), 1);
    // Four streams begin meanwhile.
    let running: Vec<_> = (0..crate::listen::MAX_STREAMS)
        .map(|_| ctx.listening(crate::listen::MAX_STREAMS).unwrap())
        .collect();
    let refused = crate::listen::open(ctx, crate::listen::Ticket::current(), &uris)
        .err()
        .expect("refused");
    assert_eq!(refused.data.unwrap()["reason"], json!("stream-cap"));
    assert_eq!((ctx.subs.held(), ctx.subs.uris()), (0, 0), "freed at once");
    settle().await;
    assert!(
        lab.cov_table().await.is_empty(),
        "cancelled before the sweep"
    );
    drop(running);
    lab.stop().await;
}

/// Two streams acknowledged with the same URIs wait to begin. The first
/// one's reservation was swept before it began, and it is refused at its
/// start: it frees nothing of the second's, which still claims its own
/// watches, and its point keeps updating (#2157).
#[tokio::test(start_paused = true)]
async fn a_refused_stream_frees_only_its_own_reservation() {
    use crate::listen::{MAX_STREAMS, Ticket, open};
    let mut lab = Lab::start_site(Policy::default(), Duration::ZERO).await;
    lab.known().await;
    let ctx = lab.mcp.ctx();
    let uris = [AI1.to_string()];
    // Each request is handled in a task of its own.
    let ticket = || async { tokio::spawn(async { Ticket::current() }).await.unwrap() };
    let (first, second) = (ticket().await, ticket().await);
    assert_ne!(first, second);
    assert_eq!(ctx.subs.reserve(ctx, first, &uris), uris);
    tokio::time::sleep(Duration::from_secs(20)).await;
    assert_eq!(ctx.subs.reserve(ctx, second, &uris), uris);
    tokio::time::sleep(Duration::from_secs(11)).await;
    ctx.subs.prune();
    assert_eq!(
        ctx.subs.pending_tickets(),
        [second],
        "the first's was swept"
    );
    assert_eq!(ctx.subs.held(), 1);

    let running: Vec<_> = (0..MAX_STREAMS)
        .map(|_| ctx.listening(MAX_STREAMS).unwrap())
        .collect();
    let refused = open(ctx, first, &uris).err().expect("refused");
    assert_eq!(refused.data.unwrap()["reason"], json!("stream-cap"));
    drop(running);
    settle().await;
    assert_eq!(ctx.subs.held(), 1, "the second's reservation stays");
    assert_eq!(lab.cov_table().await.len(), 1);

    let (listening, claim) = open(ctx, second, &uris).expect("room again");
    let claim = claim.expect("the second claims its own reservation");
    let lease = claim.lease(AI1).expect("its point's lease");
    let before = lease.snapshot().notifications;
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    lab.server
        .set_present_value_local(&ai1, PropertyValue::Real(42.0))
        .await
        .unwrap();
    settle().await;
    assert!(lease.snapshot().notifications > before, "its point updates");
    drop((claim, listening));
    lab.stop().await;
}

/// Over the pipe, a stream claims the reservation its own request made,
/// not an older one with the same URIs (#2157). This rests on the SDK
/// asking for the filter and running the stream in the request's task: if
/// that changes, the stream claims nothing and both reservations wait.
#[tokio::test(start_paused = true)]
async fn a_stream_claims_the_reservation_its_request_made() {
    use crate::listen::Ticket;
    let mut lab = Lab::start(Policy::default()).await;
    lab.known().await;
    let other = tokio::spawn(async { Ticket::current() }).await.unwrap();
    let ctx = lab.mcp.ctx();
    let uris = [AI1.to_string()];
    assert_eq!(ctx.subs.reserve(ctx, other, &uris), uris);
    let (_, accepted) = lab.agent.listen(&[AI1]).await;
    assert_eq!(accepted, [AI1]);
    settle().await;
    assert_eq!(ctx.subs.pending_tickets(), [other], "only the other waits");
    assert_eq!(lab.cov_table().await.len(), 1);

    lab.agent.drain(UPDATED, Duration::from_millis(10)).await;
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    lab.server
        .set_present_value_local(&ai1, PropertyValue::Real(42.0))
        .await
        .unwrap();
    let updated = lab.agent.notification(UPDATED).await;
    assert_eq!(updated["params"]["uri"], json!(AI1), "the stream follows");
    lab.stop().await;
}
