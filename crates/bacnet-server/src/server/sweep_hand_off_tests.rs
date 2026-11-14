//! The sweep's COV tail hands an object on to a later pass at most once,
//! and only from a pass whose event notifications went in time (#2537).

use super::*;
use bacnet_objects::event::{
    EventStateChange, EventTransitionCommit, EventTransitionCommitError, TransitionOutcome,
};
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::{CovReporting, EventReporting, LocalInputValue};
use bacnet_types::enums::{EventType, LifeSafetyState, SilencedState};
use std::borrow::Cow;
use std::collections::VecDeque;

/// Set AI-1's Present_Value as its application would, unseen by the write
/// path: above its High_Limit of 80 on even steps, below it on odd ones, and
/// a new value at every step, so each COV report differs from the last.
async fn flap(server: &BACnetServer<TestTransport>, oid: &ObjectIdentifier, step: u16) {
    let value = if step % 2 == 0 { 85 + step } else { 50 + step };
    let value = PropertyValue::Real(f32::from(value));
    let mut db = server.database().write().await;
    db.get_mut(oid)
        .unwrap()
        .local_input()
        .unwrap()
        .apply_local_input(LocalInputValue::present_value(value))
        .unwrap();
}

/// AI-1, subscribed, changes every 2 s from 0.2 s to 20.2 s, so its passes
/// commit at 2, 4, ... 22 s (each sweep that sees a change seeds its 1 s
/// delay), behind event notifications held until the instants (ms) in
/// `releases`, one each, or for good. Run to 30 s in 50 ms steps: the
/// instants (ms) at which AI-1's COV reports went out.
async fn flapping(releases: Option<&[u64]>) -> Vec<u64> {
    let (transport, sent, permits) = sweep_transport(true);
    let (db, oids) = database(&[1]);
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    subscribe(&server, &oids).await;
    let start = tokio::time::Instant::now();
    let mut reported = Vec::new();
    let mut seen = 0;
    for millis in (0..30_000).step_by(50) {
        tokio::time::sleep_until(start + Duration::from_millis(millis)).await;
        if millis % 2_000 == 200 && millis < 22_000 {
            let step = u16::try_from(millis / 2_000).unwrap();
            flap(&server, &oids[0], step).await;
        }
        if releases.is_some_and(|releases| releases.contains(&millis)) {
            permits.add_permits(1);
        }
        let frames = sent.frames();
        reported.extend(reports(&frames[seen..]).iter().map(|_| millis));
        seen = frames.len();
    }
    reported
}

/// The reviewer's probe: AI-1 changes every 2 s for 22 s behind event
/// notifications that never go out. A pass whose wait ran out hands nothing
/// on, so each pass reports at its own deadline, 3 s after its commit, well
/// inside the bound of two caps; before the fix every report was handed on
/// until the changes stopped. Handing on from such a pass, even once, leaves
/// every other deadline without a report.
#[tokio::test(start_paused = true)]
async fn a_flapping_object_behind_held_notifications_reports_at_each_deadline() {
    let reported = flapping(None).await;
    // The last pass's report would tell what the one before it told.
    let deadlines: Vec<u64> = (0..10).map(|k| 2_000 + 2_000 * k + 3_000).collect();
    assert_eq!(reported.len(), deadlines.len(), "{reported:?}");
    for (at, deadline) in reported.iter().zip(&deadlines) {
        assert!((*deadline..=deadline + 100).contains(at), "{reported:?}");
    }
}

/// Each pass's notification goes out 2.5 s after its commit, in time but
/// after the next pass committed, so every pass would hand AI-1 on to the
/// next. An object is handed on once at most, so every other pass reports
/// it, after its own notification, and each commit is reported within two
/// caps (6 s) of it; handing it on again would hold every report back until
/// the changes stop.
#[tokio::test(start_paused = true)]
async fn an_object_is_handed_on_once_at_most_so_reports_go_within_two_caps() {
    let releases: Vec<u64> = (0..11).map(|k| 2_000 + 2_000 * k + 2_500).collect();
    let reported = flapping(Some(&releases)).await;
    for k in 0..11 {
        let committed = 2_000 + 2_000 * k;
        assert!(
            reported
                .iter()
                .any(|at| (committed..=committed + 6_100).contains(at)),
            "the commit at {committed} ms unreported within two caps: {reported:?}"
        );
    }
    // Passes 1, 3, 5, 7 and 9 report, each after its own notification; the
    // last pass's report would tell what pass 9's told.
    let expected = [6_500, 10_500, 14_500, 18_500, 22_500];
    assert_eq!(reported.len(), expected.len(), "{reported:?}");
    for (at, release) in reported.iter().zip(expected) {
        assert!((release..=release + 100).contains(at), "{reported:?}");
    }
}

/// A Life Safety Point whose sweeps propose a transition while a step is
/// queued, and whose commit applies the step: its passes change the COV
/// properties the test picks.
struct ScriptedPoint {
    point: LifeSafetyPointObject,
    steps: Arc<std::sync::Mutex<VecDeque<fn(&mut LifeSafetyPointObject)>>>,
}

impl BACnetObject for ScriptedPoint {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.point.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.point.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.point.read_property(property, array_index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.point
            .write_property(property, array_index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.point.property_metadata()
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        self.point.cov()
    }
    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        Some(self)
    }
    fn event_reporting_mut(&mut self) -> Option<&mut dyn EventReporting> {
        Some(self)
    }
}

impl EventReporting for ScriptedPoint {
    fn tick_intrinsic_reporting(
        &mut self,
        _now: bacnet_objects::event::Instant,
    ) -> Option<TransitionOutcome> {
        let queued = !self.steps.lock().unwrap().is_empty();
        queued.then_some(TransitionOutcome {
            change: EventStateChange {
                from: EventState::NORMAL,
                to: EventState::LIFE_SAFETY_ALARM,
            },
            event_type: EventType::CHANGE_OF_LIFE_SAFETY,
            distribute: false,
        })
    }
    fn commit_event_transition(
        &mut self,
        _commit: EventTransitionCommit,
    ) -> Result<(), EventTransitionCommitError> {
        let step = self.steps.lock().unwrap().pop_front();
        if let Some(step) = step {
            step(&mut self.point);
        }
        Ok(())
    }
}

/// The COV reports among `frames`: the subscriber's process and the object.
fn reported_to(frames: &[crate::server::test_transport::SentFrame]) -> Vec<(u32, u32)> {
    frames
        .iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION =>
            {
                let n = COVNotificationRequest::decode(&request.service_request).unwrap();
                Some((
                    n.subscriber_process_identifier,
                    n.monitored_object_identifier.instance_number(),
                ))
            }
            _ => None,
        })
        .collect()
}

/// A pass changes LSP-1's Present_Value and hands it on, in time, to a later
/// pass that changes only its Silenced: that pass reports LSP-1 once, after
/// its own notification, with both changes, so the whole-object subscriber
/// (process 1) hears of the new Present_Value and the Silenced one (process
/// 2) of Silenced. Left out of the later pass, the Present_Value change
/// would reach no one.
#[tokio::test(start_paused = true)]
async fn a_life_safety_change_handed_on_is_reported_once_with_both_changes() {
    let (transport, sent, held) = sweep_transport(true);
    let (mut db, oids) = database(&[1, 1]);
    let steps = Arc::new(std::sync::Mutex::new(VecDeque::new()));
    let point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let lsp = point.object_identifier();
    db.add(Box::new(ScriptedPoint {
        point,
        steps: Arc::clone(&steps),
    }))
    .unwrap();
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .expect("server should start");
    for (process, property) in [(1, None), (2, Some(PropertyIdentifier::SILENCED))] {
        let subscription = bacnet_device_core::cov::CovSubscription {
            subscriber_mac: MacAddr::from_slice(&SUBSCRIBER),
            subscriber_network: None,
            subscriber_process_identifier: process,
            monitored_object_identifier: lsp,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: property,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        };
        let mut table = server.cov_table.write().await;
        table.subscribe(subscription).unwrap();
    }
    let before = sent.len();
    let reported = || reported_to(&sent.frames()[before..]);
    let start = tokio::time::Instant::now();
    let at = |millis| start + Duration::from_millis(millis);
    tokio::time::sleep_until(at(100)).await;
    begin_condition(&server, &oids[0]).await; // AI-1's pass at 1.1 s, held
    tokio::time::sleep_until(at(1_500)).await;
    steps
        .lock()
        .unwrap()
        .push_back(|point| point.set_present_value(LifeSafetyState::ALARM)); // the 2 s pass
    tokio::time::sleep_until(at(1_900)).await;
    begin_condition(&server, &oids[1]).await; // AI-2's at 2.9 s, held
    tokio::time::sleep_until(at(2_500)).await;
    steps
        .lock()
        .unwrap()
        .push_back(|point| point.set_silenced(SilencedState::AUDIBLE_SILENCED)); // with AI-2's
    tokio::time::sleep_until(at(3_500)).await;
    held.add_permits(1); // AI-1's: the 2 s pass hands LSP-1 on
    tokio::time::sleep_until(at(4_400)).await;
    assert_eq!(reported(), [], "AI-2's notification is still held");
    held.add_permits(1);
    tokio::time::sleep_until(at(4_600)).await;
    let mut once = reported();
    once.sort_unstable();
    assert_eq!(once, [(1, 1), (2, 1)]);
    tokio::time::sleep_until(at(9_000)).await;
    assert_eq!(reported().len(), 2, "reported once");
}
