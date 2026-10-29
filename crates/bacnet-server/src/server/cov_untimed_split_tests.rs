//! Untimestamped COV-multiple values that one notification cannot carry go
//! out in several, by object item (135-2020 §13.1, §13.16.2, §13.18.1.1;
//! #1038).
//!
//! The common case is the initial report of a SubscribeCOVPropertyMultiple
//! request over many objects. Each notification fits the subscriber's maximum
//! APDU and completes only the references it carries. An unconfirmed report
//! sends every part at once; one that stops partway (communication disabled,
//! the event budget spent, a failed send) leaves the rest owed for the
//! Max_Notification_Delay backstop. A confirmed report sends one part per
//! acknowledgment, and its deferred references are read afresh for the next
//! part, so a newer change goes in place of the value first prepared. Time is
//! paused.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_types::enums::{EnableDisable, EngineeringUnits};
use tokio::time::Instant as TokioInstant;

const SMALL_APDU: u16 = 206;

/// Untimestamped AVs in the context. One AV's item (Present_Value and its
/// Status_Flags companion) takes 23 octets, so a 206-octet notification
/// carries eight and the initial report needs three.
const OBJECTS: u32 = 20;

fn av(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, instance).unwrap()
}

/// Encoded APDU length of `notification`: its service request plus the
/// unconfirmed (2 octets) or unsegmented confirmed (4 octets) header.
fn apdu_len(notification: &COVNotificationMultipleRequest, confirmed: bool) -> usize {
    let mut encoded = BytesMut::new();
    notification.encode(&mut encoded).unwrap();
    encoded.len() + if confirmed { 4 } else { 2 }
}

/// `notification` fits the small APDU, carries each object's item whole
/// (Present_Value with its Status_Flags), and nothing in it is timestamped.
fn check(notification: &COVNotificationMultipleRequest, confirmed: bool) {
    let len = apdu_len(notification, confirmed);
    assert!(
        len <= usize::from(SMALL_APDU),
        "{len}-octet APDU exceeds {SMALL_APDU}"
    );
    for item in &notification.list_of_cov_notifications {
        let properties: Vec<_> = item
            .list_of_values
            .iter()
            .map(|value| value.property_identifier)
            .collect();
        assert_eq!(
            properties,
            [PV, SF],
            "{:?}",
            item.monitored_object_identifier
        );
        assert!(
            item.list_of_values
                .iter()
                .all(|v| v.time_of_change.is_none())
        );
    }
    assert_eq!(notification.timestamp, None);
}

/// Each object of `notification` with its encoded Present_Value.
fn pv_by_object(notification: &COVNotificationMultipleRequest) -> Vec<(ObjectIdentifier, Vec<u8>)> {
    notification
        .list_of_cov_notifications
        .iter()
        .map(|item| {
            let pv = item
                .list_of_values
                .iter()
                .find(|value| value.property_identifier == PV)
                .expect("a Present_Value");
            (item.monitored_object_identifier, pv.value.clone())
        })
        .collect()
}

fn objects_of(notification: &COVNotificationMultipleRequest) -> Vec<ObjectIdentifier> {
    pv_by_object(notification)
        .into_iter()
        .map(|(object, _)| object)
        .collect()
}

/// Add AV-2 to AV-20 to a harness database, which holds AV-1.
fn add_objects(db: &mut ObjectDatabase) {
    for instance in 2..=OBJECTS {
        let name = format!("AV-{instance}");
        db.add(Box::new(
            AnalogValueObject::new(instance, &name, EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    }
}

/// A server with AV-1 to AV-20 under `config`, for a subscriber whose
/// maximum APDU is 206 octets.
async fn start(config: ServerConfig) -> Harness {
    let mut h = Harness::start_with(config, add_objects).await;
    h.request_max_apdu = SMALL_APDU;
    h
}

/// Subscribe every AV without timestamps in one context with `delay`. The
/// initial report is left untaken.
async fn subscribe(h: &mut Harness, confirmed: bool, delay: u32) {
    let specs = (1..=OBJECTS)
        .map(|instance| (av(instance), vec![(PV, false)]))
        .collect();
    h.subscribe_with_delay(confirmed, specs, delay).await;
}

/// [`start`] and [`subscribe`].
async fn harness(config: ServerConfig, confirmed: bool, delay: u32) -> Harness {
    let mut h = start(config).await;
    subscribe(&mut h, confirmed, delay).await;
    h
}

/// The Present_Value the untimestamped reference of `object` last completed.
async fn completed(h: &Harness, object: ObjectIdentifier) -> Option<PropertyValue> {
    let mut table = h.server.cov_table.write().await;
    table
        .subscriptions_for(&object)
        .into_iter()
        .find(|sub| !sub.timestamped)
        .and_then(|sub| sub.last_notified_observation.as_ref())
        .map(|observation| observation.sample().value().clone())
}

/// Every object exactly once across `taken`, each with Present_Value 0.0
/// unless `changed` names its value.
fn assert_each_object_once(
    taken: &[COVNotificationMultipleRequest],
    changed: &[(ObjectIdentifier, f32)],
) {
    let mut conveyed: Vec<_> = taken.iter().flat_map(pv_by_object).collect();
    conveyed.sort_by_key(|(object, _)| object.instance_number());
    let expected: Vec<_> = (1..=OBJECTS)
        .map(|instance| {
            let value = changed
                .iter()
                .find(|(object, _)| *object == av(instance))
                .map_or(0.0, |(_, value)| *value);
            (av(instance), real(value))
        })
        .collect();
    assert_eq!(conveyed, expected, "every object once, at its newest value");
}

/// Confirmed parts, each checked and acknowledged before the next, until
/// every object has been conveyed. Returns them.
async fn take_confirmed(
    h: &Harness,
    mut taken: Vec<COVNotificationMultipleRequest>,
) -> Vec<COVNotificationMultipleRequest> {
    while taken
        .iter()
        .map(|n| n.list_of_cov_notifications.len())
        .sum::<usize>()
        < OBJECTS as usize
    {
        let notification = h.notification().await;
        check(&notification, true);
        h.no_notification().await;
        h.ack().await;
        h.settle().await;
        taken.push(notification);
    }
    h.no_notification().await;
    taken
}

#[tokio::test(start_paused = true)]
async fn an_unconfirmed_initial_report_too_large_for_one_apdu_goes_out_by_object_item() {
    let mut h = harness(ServerConfig::default(), false, 10).await;
    let mut taken: Vec<COVNotificationMultipleRequest> = Vec::new();
    while taken
        .iter()
        .map(|n| n.list_of_cov_notifications.len())
        .sum::<usize>()
        < OBJECTS as usize
    {
        let notification = h.notification().await;
        check(&notification, false);
        taken.push(notification);
    }
    h.no_notification().await;
    assert_eq!(taken.len(), 3, "eight objects per notification");
    assert_each_object_once(&taken, &[]);
    // Each part completed the references it carried when it was sent.
    for instance in 1..=OBJECTS {
        assert_eq!(
            completed(&h, av(instance)).await,
            Some(PropertyValue::Real(0.0))
        );
    }
    assert_eq!(h.server.cov_counters().notifications_sent, 3);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_confirmed_initial_report_too_large_for_one_apdu_goes_out_one_part_per_ack() {
    let mut h = harness(ServerConfig::default(), true, 10).await;
    let mut taken = Vec::new();
    let mut done = Vec::new();
    while done.len() < OBJECTS as usize {
        let notification = h.notification().await;
        check(&notification, true);
        // Nothing more until this part's Ack, which completes the references
        // it carried and only those.
        h.no_notification().await;
        let carried = objects_of(&notification);
        for object in &carried {
            assert_eq!(completed(&h, *object).await, None);
        }
        h.ack().await;
        h.settle().await;
        done.extend(carried);
        for instance in 1..=OBJECTS {
            let want = done
                .contains(&av(instance))
                .then_some(PropertyValue::Real(0.0));
            assert_eq!(completed(&h, av(instance)).await, want, "AV-{instance}");
        }
        taken.push(notification);
    }
    h.no_notification().await;
    assert_eq!(taken.len(), 3);
    assert_each_object_once(&taken, &[]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn disabling_communication_between_untimestamped_parts_leaves_the_rest_until_reenabled() {
    let config = ServerConfig {
        dcc_policy: crate::server::DccPolicy::LegacyPermissive,
        ..ServerConfig::default()
    };
    let mut h = start(config).await;
    // Initiation is disabled right after the first part goes out.
    h.disable_after_notification(0);
    subscribe(&mut h, false, 1).await;
    let first = h.notification().await;
    check(&first, false);
    assert_eq!(first.list_of_cov_notifications.len(), 8, "a first part");
    // A deferred object changes meanwhile: its newer value goes in place of
    // the one first prepared.
    let deferred = av(OBJECTS);
    assert!(!objects_of(&first).contains(&deferred));
    h.write_local_to(deferred, 42.0).await;
    // Clause 16.1: nothing more while initiation is disabled, not even at the
    // Max_Notification_Delay deadline.
    tokio::time::sleep(Duration::from_secs(3)).await;
    h.no_notification().await;
    h.dcc(EnableDisable::ENABLE, None).await;
    let mut taken = vec![first];
    while taken
        .iter()
        .map(|n| n.list_of_cov_notifications.len())
        .sum::<usize>()
        < OBJECTS as usize
    {
        let notification = h.notification().await;
        check(&notification, false);
        taken.push(notification);
    }
    h.no_notification().await;
    assert_each_object_once(&taken, &[(deferred, 42.0)]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_stopped_unconfirmed_report_leaves_its_remaining_parts_for_the_deadline() {
    for budget in [true, false] {
        let mut config = ServerConfig::default();
        if budget {
            // One notification per event.
            config.cov_policy.max_notifications_per_event = 1;
        }
        let mut h = start(config).await;
        if !budget {
            // The second part's send fails; the first was sent.
            h.fail_notification(1);
        }
        let started = TokioInstant::now();
        subscribe(&mut h, false, 2).await;
        let first = h.notification().await;
        check(&first, false);
        h.no_notification().await;
        let mut taken = vec![first];
        while taken
            .iter()
            .map(|n| n.list_of_cov_notifications.len())
            .sum::<usize>()
            < OBJECTS as usize
        {
            let notification = h.notification().await;
            check(&notification, false);
            taken.push(notification);
        }
        assert!(
            started.elapsed() >= Duration::from_secs(2),
            "budget={budget}: the rest waited for the deadline"
        );
        h.no_notification().await;
        assert_each_object_once(&taken, &[]);
        h.server.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn confirmed_deferred_parts_outlast_a_follow_up_dropped_under_dcc() {
    let config = ServerConfig {
        dcc_policy: crate::server::DccPolicy::LegacyPermissive,
        ..ServerConfig::default()
    };
    let mut h = harness(config, true, 1).await;
    let first = h.notification().await;
    check(&first, true);
    // Initiation is disabled while the first part is outstanding, so the
    // follow-up its Ack owes is dropped (Clause 16.1).
    h.server
        .comm_state
        .set_for_test(DccState::DisableInitiation);
    h.ack().await;
    h.settle().await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    h.no_notification().await;
    // The deferred references are owed, so re-enabling sends them.
    h.dcc(EnableDisable::ENABLE, None).await;
    let taken = take_confirmed(&h, vec![first]).await;
    assert_eq!(taken.len(), 3);
    assert_each_object_once(&taken, &[]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_newer_change_supersedes_a_deferred_untimestamped_part() {
    let mut h = harness(ServerConfig::default(), true, 10).await;
    let first = h.notification().await;
    check(&first, true);
    let deferred = av(OBJECTS);
    assert!(!objects_of(&first).contains(&deferred));
    // The context is busy with the first part: the change waits, and the
    // follow-up reads the deferred value afresh.
    h.write_local_to(deferred, 42.0).await;
    h.no_notification().await;
    h.ack().await;
    h.settle().await;
    let taken = take_confirmed(&h, vec![first]).await;
    assert_each_object_once(&taken, &[(deferred, 42.0)]);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn objects_a_part_deferred_go_ahead_of_newer_changes() {
    let mut h = harness(ServerConfig::default(), true, 10).await;
    let first = h.notification().await;
    check(&first, true);
    // Every object of the first part changes before its Ack, so all of them
    // report again; the deferred ones still go first.
    let sent = objects_of(&first);
    let changed: Vec<_> = sent.iter().map(|object| (*object, 7.0)).collect();
    for (object, value) in &changed {
        h.write_local_to(*object, *value).await;
    }
    h.ack().await;
    h.settle().await;
    let second = h.notification().await;
    check(&second, true);
    assert!(
        objects_of(&second)
            .iter()
            .all(|object| !sent.contains(object)),
        "the deferred objects lead the next part"
    );
    h.no_notification().await;
    h.ack().await;
    h.settle().await;
    let taken = take_confirmed(&h, vec![second]).await;
    let mut seen: Vec<_> = taken.iter().flat_map(pv_by_object).collect();
    seen.sort_by_key(|(object, _)| object.instance_number());
    for instance in 1..=OBJECTS {
        let value = if sent.contains(&av(instance)) {
            7.0
        } else {
            0.0
        };
        assert_eq!(
            seen.iter()
                .filter(|(object, _)| *object == av(instance))
                .map(|(_, pv)| pv.clone())
                .collect::<Vec<_>>(),
            vec![real(value)],
            "AV-{instance} once more, at its newest value"
        );
    }
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_untimestamped_value_too_large_for_any_notification_is_left_out_without_stalling() {
    use bacnet_objects::value_types::CharacterStringValueObject;
    let csv = ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            CharacterStringValueObject::new(1, "CSV-1").unwrap(),
        ))
        .unwrap();
        add_objects(db);
    })
    .await;
    // A 200-character value alone exceeds a 206-octet notification.
    write_text(&h, csv, "a".repeat(200)).await;
    h.request_max_apdu = SMALL_APDU;
    let mut specs = vec![(csv, vec![(PV, false)])];
    specs.extend((1..=OBJECTS).map(|instance| (av(instance), vec![(PV, false)])));
    h.subscribe_with_delay(true, specs, 1).await;
    // Every AV goes out, one part per Ack; the string never does, and it
    // holds nothing back, not even after the backstop's deadline.
    let taken = take_confirmed(&h, Vec::new()).await;
    assert!(taken.iter().all(|n| !objects_of(n).contains(&csv)));
    assert_each_object_once(&taken, &[]);
    tokio::time::sleep(Duration::from_secs(3)).await;
    h.no_notification().await;
    assert_eq!(completed(&h, csv).await, None);
    // The context is idle, so the next change reports at once.
    h.write_local_to(av(1), 3.0).await;
    let report = h.notification().await;
    check(&report, true);
    assert_eq!(pv_by_object(&report), vec![(av(1), real(3.0))]);
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

/// Write `text` to the Present_Value of `object`, a CharacterString Value.
async fn write_text(h: &Harness, object: ObjectIdentifier, text: String) {
    h.server
        .write_local(
            &object,
            PV,
            None,
            PropertyValue::CharacterString(text),
            Some(8),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn each_report_leaving_out_an_oversized_untimestamped_value_counts_it() {
    use bacnet_objects::value_types::CharacterStringValueObject;
    let csv = ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            CharacterStringValueObject::new(1, "CSV-1").unwrap(),
        ))
        .unwrap();
    })
    .await;
    // A 200-character value alone exceeds a 206-octet notification.
    write_text(&h, csv, "a".repeat(200)).await;
    h.request_max_apdu = SMALL_APDU;
    let specs = vec![(csv, vec![(PV, false)]), (av1(), vec![(PV, false)])];
    h.subscribe_with_delay(false, specs, 1).await;
    // The initial report carries AV-1 alone; leaving the string out counts
    // once, and no timestamped change is involved (#1066).
    let report = h.notification().await;
    check(&report, false);
    assert_eq!(objects_of(&report), [av1()]);
    h.no_notification().await;
    let counters = h.server.cov_counters();
    assert_eq!(counters.untimed_references_oversized, 1);
    assert_eq!(counters.timed_changes_dropped, 0);
    // A newer value that is still too large: its report sends nothing and
    // counts again.
    write_text(&h, csv, "b".repeat(200)).await;
    h.no_notification().await;
    assert_eq!(h.server.cov_counters().untimed_references_oversized, 2);
    // A value that fits goes out, and counts nothing.
    write_text(&h, csv, "c".into()).await;
    let report = h.notification().await;
    check(&report, false);
    assert_eq!(objects_of(&report), [csv]);
    h.no_notification().await;
    let counters = h.server.cov_counters();
    assert_eq!(counters.untimed_references_oversized, 2);
    assert_eq!(counters.timed_changes_dropped, 0);
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_split_report_going_out_holds_back_a_later_change_until_its_last_part() {
    let mut h = start(ServerConfig::default()).await;
    // The transport holds the initial report's second part.
    let release = h.hold_notification(1);
    subscribe(&mut h, false, 10).await;
    let first = h.notification().await;
    check(&first, false);
    let late = av(OBJECTS);
    assert!(!objects_of(&first).contains(&late));
    // A change to an object of a later part, while the report is still going
    // out, stands back: it must not reach the subscriber ahead of the older
    // value that part carries.
    h.write_local_to(late, 42.0).await;
    h.no_notification().await;
    release.add_permits(1);
    let mut conveyed = Vec::new();
    while conveyed.last() != Some(&real(42.0)) {
        let notification = h.notification().await;
        check(&notification, false);
        conveyed.extend(
            pv_by_object(&notification)
                .into_iter()
                .filter(|(object, _)| *object == late)
                .map(|(_, pv)| pv),
        );
    }
    assert_eq!(conveyed, [real(0.0), real(42.0)], "the newer value last");
    h.no_notification().await;
    assert_eq!(completed(&h, late).await, Some(PropertyValue::Real(42.0)));
    h.server.stop().await.unwrap();
}
