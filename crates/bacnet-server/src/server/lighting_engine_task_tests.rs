//! Lighting Output's fades, ramps and egress timers on the server's
//! monotonic task (#1384), in paused time: a write that starts one wakes the
//! task, which samples Tracking_Value for COV as it moves, ends the fade or
//! ramp on time, and reports the new Present_Value when an egress runs out.
//!
//! Table 13-1 reports a Lighting Output's Present_Value and Status_Flags, so
//! a SubscribeCOV hears a fade once, when the target goes in; a
//! SubscribeCOVProperty of Tracking_Value hears it move.

use super::cov_notifications_tests::recording_transport;
use super::cov_wire_test_support::real;
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::lighting::LightingOutputObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const TV: PropertyIdentifier = PropertyIdentifier::TRACKING_VALUE;
const LC: PropertyIdentifier = PropertyIdentifier::LIGHTING_COMMAND;

/// The whole-object subscriber's process identifier.
const OBJECT: u32 = 1;
/// The Tracking_Value subscriber's process identifier.
const TRACKING: u32 = 2;

async fn start(
    configure: impl FnOnce(&mut LightingOutputObject),
) -> (BACnetServer<TestTransport>, ObjectIdentifier, SendLog) {
    let (transport, sent) = recording_transport();
    let mut object = LightingOutputObject::new(1, "LO-1").unwrap();
    configure(&mut object);
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1384,
            name: "Lighting-engine-device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(object)).unwrap();
    let config = ServerConfig {
        enable_event_enrollment: false,
        ..ServerConfig::default()
    };
    let server = BACnetServer::start(config, db, transport).await.unwrap();
    settle().await;
    (server, oid, sent)
}

async fn settle() {
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
}

/// Move paused time on by `milliseconds` and let the server catch up.
async fn advance(milliseconds: u64) {
    tokio::time::advance(Duration::from_millis(milliseconds)).await;
    settle().await;
}

/// Subscribe to the whole object, and to Tracking_Value with `increment`.
async fn subscribe(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    increment: Option<f32>,
) {
    let mut table = server.cov_table.write().await;
    for (process, property, cov_increment) in
        [(OBJECT, None, None), (TRACKING, Some(TV), increment)]
    {
        table
            .subscribe(CovSubscription {
                subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, process as u8]),
                subscriber_network: None,
                subscriber_process_identifier: process,
                monitored_object_identifier: oid,
                issue_confirmed_notifications: false,
                expires_at: None,
                last_notified_observation: None,
                monitored_property: property,
                monitored_property_array_index: None,
                cov_increment,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            })
            .unwrap();
    }
}

async fn write(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: PropertyValue,
    priority: Option<u8>,
) {
    server
        .write_local(
            &oid,
            property,
            None,
            value,
            priority,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

/// A Lighting_Command write's value: the command's octets.
fn command(octets: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData(octets.to_vec())
}

/// The first value each COV notification sent since the last call carried,
/// with its subscriber process, as encoded, ordered by process.
fn reports(sent: &SendLog) -> Vec<(u32, PropertyIdentifier, Vec<u8>)> {
    let mut reports: Vec<_> = sent
        .take()
        .into_iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION =>
            {
                let report = COVNotificationRequest::decode(&request.service_request).unwrap();
                let first = &report.list_of_values[0];
                Some((
                    report.subscriber_process_identifier,
                    first.property_identifier,
                    first.value.clone(),
                ))
            }
            _ => None,
        })
        .collect();
    // Subscribers are reported in no set order.
    reports.sort_by_key(|report| report.0);
    reports
}

async fn read(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn a_fade_shorter_than_the_routine_pass_is_sampled_and_ended_on_time() {
    let (mut server, oid, sent) = start(|_| {}).await;
    // Past the task's first pass; its next routine one is at 2 s.
    advance(1_200).await;
    subscribe(&server, oid, None).await;
    // FADE_TO 90.0 % (0x42B40000) over 300 ms (0x012C) at priority 8.
    let fade = [
        0x09, 0x01, 0x1C, 0x42, 0xB4, 0x00, 0x00, 0x4A, 0x01, 0x2C, 0x59, 0x08,
    ];
    write(&server, oid, LC, command(&fade), None).await;
    // The write reports the new Present_Value, and a first Tracking_Value.
    assert_eq!(
        reports(&sent),
        [(OBJECT, PV, real(90.0)), (TRACKING, TV, real(0.0))]
    );
    // A 1 percent step is 3.3 ms of this fade, so each 100 ms grid point
    // samples, and the last is the end.
    for expected in [30.0, 60.0, 90.0] {
        advance(100).await;
        assert_eq!(reports(&sent), [(TRACKING, TV, real(expected))]);
    }
    assert_eq!(
        read(&server, oid, PropertyIdentifier::IN_PROGRESS).await,
        PropertyValue::Enumerated(0)
    );
    let deadline = server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .next_monotonic_deadline_internal();
    assert_eq!(deadline, None);
    advance(5_000).await;
    assert_eq!(reports(&sent), []);
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_ramp_reports_tracking_value_by_increment_and_present_value_once() {
    let (mut server, oid, sent) = start(|object| {
        object
            .write_property(
                PropertyIdentifier::COV_INCREMENT,
                None,
                PropertyValue::Real(25.0),
                None,
            )
            .unwrap();
    })
    .await;
    subscribe(&server, oid, Some(50.0)).await;
    // RAMP_TO 100.0 % (0x42C80000) at 50.0 %/s (0x42480000): 2 s.
    let ramp = [
        0x09, 0x02, 0x1C, 0x42, 0xC8, 0x00, 0x00, 0x2C, 0x42, 0x48, 0x00, 0x00,
    ];
    write(&server, oid, LC, command(&ramp), None).await;
    assert_eq!(
        reports(&sent),
        [(OBJECT, PV, real(100.0)), (TRACKING, TV, real(0.0))]
    );
    // COV_Increment 25 samples Tracking_Value every quarter, 0.5 s; the
    // subscription's own increment of 50 reports every other sample.
    let mut heard = Vec::new();
    for _ in 0..4 {
        advance(500).await;
        heard.push(reports(&sent));
    }
    assert_eq!(
        heard,
        [
            vec![],
            vec![(TRACKING, TV, real(50.0))],
            vec![],
            vec![(TRACKING, TV, real(100.0))],
        ]
    );
    assert_eq!(read(&server, oid, TV).await, PropertyValue::Real(100.0));
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_egress_reports_present_value_when_it_runs_out() {
    let (mut server, oid, sent) = start(|object| {
        for (property, value) in [
            (
                PropertyIdentifier::BLINK_WARN_ENABLE,
                PropertyValue::Boolean(true),
            ),
            (PropertyIdentifier::EGRESS_TIME, PropertyValue::Unsigned(2)),
        ] {
            object.write_property(property, None, value, None).unwrap();
        }
    })
    .await;
    write(&server, oid, PV, PropertyValue::Real(80.0), Some(8)).await;
    subscribe(&server, oid, None).await;
    // Present_Value -3.0 is WARN_OFF: blink, hold for 2 s, then 0.0.
    write(&server, oid, PV, PropertyValue::Real(-3.0), Some(8)).await;
    assert_eq!(
        reports(&sent),
        [(OBJECT, PV, real(80.0)), (TRACKING, TV, real(80.0))]
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::EGRESS_ACTIVE).await,
        PropertyValue::Boolean(true)
    );
    advance(1_999).await;
    assert_eq!(reports(&sent), []);
    advance(1).await;
    assert_eq!(
        reports(&sent),
        [(OBJECT, PV, real(0.0)), (TRACKING, TV, real(0.0))]
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::EGRESS_ACTIVE).await,
        PropertyValue::Boolean(false)
    );
    server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn a_trim_change_fades_in_on_the_task_and_reports_trim_active() {
    let (mut server, oid, sent) = start(|object| {
        object.set_high_end_trim(Some(100.0)).unwrap();
        object.set_trim_fade_time(300).unwrap();
    })
    .await;
    subscribe(&server, oid, None).await;
    write(&server, oid, PV, PropertyValue::Real(90.0), Some(8)).await;
    assert_eq!(
        reports(&sent),
        [(OBJECT, PV, real(90.0)), (TRACKING, TV, real(90.0))]
    );
    // High_End_Trim 60 (4194335): the bound moves from 100 down to 60 over
    // 300 ms, and Tracking_Value follows it from 90 down at each 100 ms grid
    // point. Present_Value stays 90, so whole-object subscribers hear
    // nothing.
    write(
        &server,
        oid,
        PropertyIdentifier::HIGH_END_TRIM,
        PropertyValue::Real(60.0),
        None,
    )
    .await;
    assert_eq!(reports(&sent), []);
    // TRIM_ACTIVE is 5.
    assert_eq!(
        read(&server, oid, PropertyIdentifier::IN_PROGRESS).await,
        PropertyValue::Enumerated(5)
    );
    // 100 - 40/3 is a little over 86.66, so the first grid point holds
    // Tracking_Value at the bound; the end holds it at the trim.
    let mut heard = Vec::new();
    for _ in 0..3 {
        advance(100).await;
        heard.push(reports(&sent));
    }
    assert_eq!(
        heard,
        [
            vec![(TRACKING, TV, real(100.0 - 40.0 / 3.0))],
            vec![(TRACKING, TV, real(100.0 - 80.0 / 3.0))],
            vec![(TRACKING, TV, real(60.0))],
        ]
    );
    assert_eq!(
        read(&server, oid, PropertyIdentifier::IN_PROGRESS).await,
        PropertyValue::Enumerated(5)
    );
    let deadline = server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .next_monotonic_deadline_internal();
    assert_eq!(deadline, None);
    server.stop().await.unwrap();
}

/// Subscribe `process` to Tracking_Value alone, with `increment`, and return
/// the subscription's key.
async fn subscribe_tracking(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    process: u32,
    increment: Option<f32>,
) -> crate::cov::CovSubscriptionKey {
    server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, process as u8]),
            subscriber_network: None,
            subscriber_process_identifier: process,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: Some(TV),
            monitored_property_array_index: None,
            cov_increment: increment,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap()
        .key()
        .clone()
}

/// Tracking_Value subscribers finer than COV_Increment get denser samples
/// (#1510): the finest one sets the step, at most one sample per 100 ms grid
/// point, and each subscriber still hears only its own steps.
#[tokio::test(start_paused = true)]
async fn the_finest_tracking_value_subscriber_sets_the_sample_step() {
    let (mut server, oid, sent) = start(|object| {
        object
            .write_property(
                PropertyIdentifier::COV_INCREMENT,
                None,
                PropertyValue::Real(25.0),
                None,
            )
            .unwrap();
    })
    .await;
    // Increments of 1, 10 and 50 percent on processes 3, 4 and 5.
    for (process, increment) in [(3, 1.0), (4, 10.0), (5, 50.0)] {
        subscribe_tracking(&server, oid, process, Some(increment)).await;
    }
    // RAMP_TO 100.0 % at 50.0 %/s, 5 percent each 100 ms, over 2 s.
    let ramp = [
        0x09, 0x02, 0x1C, 0x42, 0xC8, 0x00, 0x00, 0x2C, 0x42, 0x48, 0x00, 0x00,
    ];
    write(&server, oid, LC, command(&ramp), None).await;
    let first: Vec<_> = [3, 4, 5].map(|p| (p, TV, real(0.0))).into();
    assert_eq!(reports(&sent), first);
    // A 1 percent step is 20 ms, so the grid bounds it: a sample every
    // 100 ms, not every quarter as COV_Increment 25 alone would give.
    for tenth in 1..=20u8 {
        advance(100).await;
        let level = f32::from(tenth) * 5.0;
        let mut expected = vec![(3, TV, real(level))];
        if tenth % 2 == 0 {
            expected.push((4, TV, real(level)));
        }
        if tenth % 10 == 0 {
            expected.push((5, TV, real(level)));
        }
        assert_eq!(reports(&sent), expected, "at {} ms", u32::from(tenth) * 100);
    }
    server.stop().await.unwrap();
}

/// A subscriber increment finer than COV_Increment but coarser than the ramp
/// moves in one 100 ms grid cell sets the step itself, and once that
/// subscriber goes the object's own step comes back (#1510).
#[tokio::test(start_paused = true)]
async fn an_unsubscribe_gives_back_the_objects_own_sample_step() {
    let (mut server, oid, sent) = start(|object| {
        object
            .write_property(
                PropertyIdentifier::COV_INCREMENT,
                None,
                PropertyValue::Real(25.0),
                None,
            )
            .unwrap();
    })
    .await;
    // Process 2 gives no increment, so it hears every sample; process 3
    // asks for 10 percent.
    subscribe_tracking(&server, oid, 2, None).await;
    let fine = subscribe_tracking(&server, oid, 3, Some(10.0)).await;
    // RAMP_TO 100.0 % at 50.0 %/s, 5 percent each 100 ms, over 2 s.
    let ramp = [
        0x09, 0x02, 0x1C, 0x42, 0xC8, 0x00, 0x00, 0x2C, 0x42, 0x48, 0x00, 0x00,
    ];
    write(&server, oid, LC, command(&ramp), None).await;
    assert_eq!(reports(&sent), [(2, TV, real(0.0)), (3, TV, real(0.0))]);
    // 10 percent is two grid cells of this ramp: a sample every 200 ms, not
    // at every grid point.
    let mut heard = Vec::new();
    for _ in 0..4 {
        advance(100).await;
        heard.push(reports(&sent));
    }
    let both = |level| vec![(2, TV, real(level)), (3, TV, real(level))];
    assert_eq!(heard, [vec![], both(10.0), vec![], both(20.0)]);
    // Without process 3, the sample already planned for 600 ms stands; after
    // it, COV_Increment's quarter, 500 ms, is the step again.
    server.cov_table.write().await.unsubscribe(&fine);
    for at in (500..=2_000).step_by(100) {
        advance(100).await;
        let expected = match at {
            600 => vec![(2, TV, real(30.0))],
            1_100 => vec![(2, TV, real(55.0))],
            1_600 => vec![(2, TV, real(80.0))],
            2_000 => vec![(2, TV, real(100.0))],
            _ => vec![],
        };
        assert_eq!(reports(&sent), expected, "at {at} ms");
    }
    server.stop().await.unwrap();
}
