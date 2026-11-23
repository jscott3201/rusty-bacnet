//! A Staging plan due at start-up whose targets alarm to Device recipients
//! the server holds no binding for (#2573): `start()` writes the targets and
//! returns, without waiting for any device; the devices are looked for in
//! the background, under the usual wait, and each silent one counts once.
//! `stop()` while they are looked for returns at once and leaves no lane,
//! no task and no send behind. The plan's COV fanout still goes, after the
//! alarms are handed off.
//!
//! STG-1 drives BV-1, BV-2 and BV-3 active at start-up; BV-k alarms when
//! active through Notification Class k, which names Device 9 + k, silent
//! unless a test answers for it.

use super::*;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};

/// The targets, and the silent devices their classes name.
const TARGETS: u32 = 3;

fn bv(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance).unwrap()
}

/// BV-k, alarming through Notification Class k to Device 9 + k, for each
/// target, and STG-1 driving them all active.
fn alarming_targets(db: &mut ObjectDatabase) {
    for k in 1..=TARGETS {
        let mut class = NotificationClass::new(k, format!("NC-{k}")).unwrap();
        let mut to_device = destination_for(BACnetRecipient::Device(device(9 + k)), false);
        to_device.process_identifier = 9 + k;
        class.add_destination(to_device).unwrap();
        db.add(Box::new(class)).unwrap();
        let mut value = BinaryValueObject::new(k, format!("BV-{k}")).unwrap();
        let every_transition = PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0xE0],
        };
        let class = PropertyValue::Unsigned(k.into());
        for (property, setting) in [
            (PropertyIdentifier::EVENT_ENABLE, every_transition),
            (PropertyIdentifier::NOTIFICATION_CLASS, class),
        ] {
            BACnetObject::write_property(&mut value, property, None, setting, None).unwrap();
        }
        db.add(Box::new(value)).unwrap();
    }
    let stage = |limit: f32, active: bool| BACnetStageLimitValue {
        limit,
        values: vec![active; TARGETS as usize],
        deadband: 1.0,
    };
    let config = StagingConfig {
        present_value: 15.0,
        min_present_value: 0.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![stage(10.0, false), stage(20.0, true)],
        target_references: (1..=TARGETS)
            .map(|k| BACnetDeviceObjectReference {
                device_identifier: None,
                object_identifier: bv(k),
            })
            .collect(),
        stage_names: None,
    };
    db.add(Box::new(StagingObject::new(1, "STG-1", config).unwrap()))
        .unwrap();
}

/// The server, started: its start-up plan alarms to three silent devices.
async fn started() -> Harness {
    start_for_and(ServerConfig::default(), &[], &[], alarming_targets).await
}

async fn event_state(h: &Harness, oid: ObjectIdentifier) -> PropertyValue {
    let db = h.server.database().read().await;
    let object = db.get(&oid).unwrap();
    object
        .read_property(PropertyIdentifier::EVENT_STATE, None)
        .unwrap()
}

/// `start()` returns with the targets written and their alarms committed,
/// before any device is looked for to the end; then each device gets its
/// Who-Is and, silent past the wait, counts once.
#[tokio::test(start_paused = true)]
async fn start_returns_without_waiting_for_silent_devices() {
    let begun = Instant::now();
    let mut h = started().await;
    assert!(
        begun.elapsed() < Duration::from_millis(10),
        "{:?}",
        begun.elapsed()
    );
    let offnormal = PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw());
    for k in 1..=TARGETS {
        assert_eq!(event_state(&h, bv(k)).await, offnormal, "BV-{k}");
    }
    let mut looked_for = Vec::new();
    until("a Who-Is for each device", || {
        looked_for.extend(who_is_sent(&h).into_iter().map(|(_, who_is)| who_is));
        looked_for.len() == TARGETS as usize
    })
    .await;
    for k in 1..=TARGETS {
        assert!(looked_for.contains(&targeted(9 + k)), "Device {}", 9 + k);
    }
    assert_eq!(counted(&h), (0, 0), "still waiting");
    tokio::time::sleep(WAIT).await;
    h.settle().await;
    assert_eq!(counted(&h), (u64::from(TARGETS), 0));
    assert_eq!(open_lanes(&h), 0);
    assert!(notifications(&h).is_empty());
    h.server.stop().await.unwrap();
}

/// `stop()` while the devices are looked for, the plan's COV fanout still
/// waiting for its notifications in a task, returns at once, and leaves no
/// lane, no task and nothing more to send.
#[tokio::test(start_paused = true)]
async fn stop_while_the_devices_are_looked_for_leaves_nothing() {
    let mut h = started().await;
    let log = h.server.test_network().transport().sent();
    until("the Who-Is", || !who_is_sent(&h).is_empty()).await;
    assert!(
        !h.server.request_tasks.is_empty(),
        "the fanout waits in a task"
    );
    let begun = Instant::now();
    h.server.stop().await.unwrap();
    assert!(
        begun.elapsed() < Duration::from_millis(10),
        "{:?}",
        begun.elapsed()
    );
    assert!(h.server.request_tasks.is_empty());
    {
        let lanes = lanes(&h.server.notification_transactions);
        assert_eq!((lanes.open(), lanes.tasks.len()), (0, 0));
    }
    let sent = log.len();
    tokio::time::sleep(WAIT * 2).await;
    assert_eq!(log.len(), sent, "nothing is sent after stop()");
    assert_eq!(counted(&h), (0, 0));
}

/// Whether `frame` is an event notification.
fn is_event(frame: &crate::server::test_transport::SentFrame) -> bool {
    matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
        if request.service_choice == UnconfirmedServiceChoice::UNCONFIRMED_EVENT_NOTIFICATION)
}

/// A subscriber to BV-1 there before the plan's COV fanout, which waits for
/// the start-up alarms: once the devices answer and the alarms go, the
/// fanout reports BV-1 to it, after every alarm.
#[tokio::test(start_paused = true)]
async fn the_plans_cov_fanout_follows_its_alarms() {
    let mut h = started().await;
    h.server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[10, 0, 0, 30, 0xBA, 0xC0]),
            subscriber_network: None,
            subscriber_process_identifier: 1,
            monitored_object_identifier: bv(1),
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: Some(PropertyIdentifier::PRESENT_VALUE),
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
    let mut looked_for = 0;
    until("a Who-Is for each device", || {
        looked_for += who_is_sent(&h).len();
        looked_for == TARGETS as usize
    })
    .await;
    assert_eq!(cov_reports(&h), 0, "the fanout waits for the alarms");
    for k in 1..=TARGETS {
        let mac = [10, 0, 0, 20 + k as u8, 0xBA, 0xC0];
        deliver(&h, &i_am(9 + k), &mac, None).await;
    }
    until("the report", || cov_reports(&h) == 1).await;
    let frames = h.server.test_network().transport().sent().frames();
    let report = frames.iter().position(is_cov).unwrap();
    let alarms: Vec<_> = (frames.iter().enumerate())
        .filter(|(_, frame)| is_event(frame))
        .map(|(at, _)| at)
        .collect();
    assert_eq!(alarms.len(), TARGETS as usize);
    assert!(
        alarms.iter().all(|&at| at < report),
        "{alarms:?} then {report}"
    );
    h.server.stop().await.unwrap();
}
