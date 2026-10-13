//! Color and Color Temperature fades on the server's monotonic task (#1474),
//! in paused time: the write that starts one wakes the task, which samples
//! Tracking_Value for COV as it moves and ends the fade on time.
//!
//! Neither object has Status_Flags, and Present_Value holds a fade's target
//! from the start, so a SubscribeCOV hears a fade once, when the target goes
//! in; a SubscribeCOVProperty of Tracking_Value hears it move.

use super::cov_notifications_tests::recording_transport;
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_objects::color::{ColorObject, ColorTemperatureObject};
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_types::constructed::BACnetXyColor;

const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const TV: PropertyIdentifier = PropertyIdentifier::TRACKING_VALUE;
const CC: PropertyIdentifier = PropertyIdentifier::COLOR_COMMAND;

/// The whole-object subscriber's process identifier.
const OBJECT: u32 = 1;
/// The Tracking_Value subscriber's process identifier.
const TRACKING: u32 = 2;

async fn start(
    object: Box<dyn BACnetObject>,
) -> (BACnetServer<TestTransport>, ObjectIdentifier, SendLog) {
    let (transport, sent) = recording_transport();
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1474,
            name: "Color-engine-device".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(object).unwrap();
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

/// Subscribe to the whole object, and to Tracking_Value.
async fn subscribe(server: &BACnetServer<TestTransport>, oid: ObjectIdentifier) {
    let mut table = server.cov_table.write().await;
    for (process, property) in [(OBJECT, None), (TRACKING, Some(TV))] {
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
                cov_increment: None,
                notification_kind: CovNotificationKind::Single,
                timestamped: false,
            })
            .unwrap();
    }
}

/// Write Color_Command's `octets` the way the server's own writes go.
async fn command(server: &BACnetServer<TestTransport>, oid: ObjectIdentifier, octets: &[u8]) {
    server
        .write_local(
            &oid,
            CC,
            None,
            PropertyValue::ApplicationData(octets.to_vec()),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

/// A COV notification's subscriber process and the values it carried, as
/// encoded.
type Report = (u32, Vec<(PropertyIdentifier, Vec<u8>)>);

/// What each COV notification sent since the last call carried, ordered by
/// subscriber process.
fn reports(sent: &SendLog) -> Vec<Report> {
    let mut reports: Vec<_> = sent
        .take()
        .into_iter()
        .filter_map(|frame| match frame.apdu() {
            Apdu::UnconfirmedRequest(request)
                if request.service_choice
                    == UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION =>
            {
                let report = COVNotificationRequest::decode(&request.service_request).unwrap();
                let values = report
                    .list_of_values
                    .iter()
                    .map(|value| (value.property_identifier, value.value.clone()))
                    .collect();
                Some((report.subscriber_process_identifier, values))
            }
            _ => None,
        })
        .collect();
    // Subscribers are reported in no set order.
    reports.sort_by_key(|report| report.0);
    reports
}

/// One report of `property` holding `value`, to `process`.
fn report(process: u32, property: PropertyIdentifier, value: Vec<u8>) -> Report {
    (process, vec![(property, value)])
}

fn kelvin(value: u16) -> Vec<u8> {
    [&[0x22][..], &value.to_be_bytes()].concat()
}

fn xy(x: f32, y: f32) -> Vec<u8> {
    [&[0x44][..], &x.to_be_bytes(), &[0x44], &y.to_be_bytes()].concat()
}

#[tokio::test(start_paused = true)]
async fn a_temperature_fade_is_sampled_on_the_grid_and_ended_on_time() {
    let object = ColorTemperatureObject::new(1, "CT-1").unwrap();
    let (mut server, oid, sent) = start(Box::new(object)).await;
    // Past the task's first pass; its next routine one is at 2 s.
    advance(1_200).await;
    subscribe(&server, oid).await;
    // FADE_TO_CCT 4300 K (0x10CC) over 300 ms (0x012C), from 4000 K.
    command(
        &server,
        oid,
        &[0x09, 0x02, 0x2A, 0x10, 0xCC, 0x3A, 0x01, 0x2C],
    )
    .await;
    // The write reports the new Present_Value alone (no Status_Flags), and a
    // first Tracking_Value.
    assert_eq!(
        reports(&sent),
        [
            report(OBJECT, PV, kelvin(4_300)),
            report(TRACKING, TV, kelvin(4_000)),
        ]
    );
    // A 10 K step is 10 ms of this fade, so each 100 ms grid point samples,
    // and the last is the end.
    for expected in [4_100, 4_200, 4_300] {
        advance(100).await;
        assert_eq!(reports(&sent), [report(TRACKING, TV, kelvin(expected))]);
    }
    let in_progress = server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::IN_PROGRESS, None)
        .unwrap();
    assert_eq!(in_progress, PropertyValue::Enumerated(0));
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
async fn a_colour_fade_reports_each_xy_sample() {
    let mut object = ColorObject::new(1, "CLR-1").unwrap();
    object
        .set_present_value(BACnetXyColor::new(0.25, 0.5))
        .unwrap();
    let (mut server, oid, sent) = start(Box::new(object)).await;
    advance(1_200).await;
    subscribe(&server, oid).await;
    // FADE_TO_COLOR to (0.75, 0.25) over 400 ms (0x0190).
    let fade = [
        &[0x09, 0x01, 0x1E][..],
        &xy(0.75, 0.25),
        &[0x1F, 0x3A, 0x01, 0x90],
    ]
    .concat();
    command(&server, oid, &fade).await;
    assert_eq!(
        reports(&sent),
        [
            report(OBJECT, PV, xy(0.75, 0.25)),
            report(TRACKING, TV, xy(0.25, 0.5)),
        ]
    );
    // A quarter of the way each 100 ms.
    for (x, y) in [(0.375, 0.4375), (0.5, 0.375), (0.625, 0.3125), (0.75, 0.25)] {
        advance(100).await;
        assert_eq!(reports(&sent), [report(TRACKING, TV, xy(x, y))]);
    }
    advance(5_000).await;
    assert_eq!(reports(&sent), []);
    server.stop().await.unwrap();
}

/// A Tracking_Value subscriber asking for 2 K hears a slow temperature ramp
/// every 2 K, each 100 ms, rather than at the object's own 10 K step, every
/// half second (#1510).
#[tokio::test(start_paused = true)]
async fn a_finer_tracking_value_subscriber_samples_a_temperature_ramp_more_often() {
    let object = ColorTemperatureObject::new(1, "CT-1").unwrap();
    let (mut server, oid, sent) = start(Box::new(object)).await;
    server
        .cov_table
        .write()
        .await
        .subscribe(CovSubscription {
            subscriber_mac: MacAddr::from_slice(&[127, 0, 0, 1, 0xBA, 3]),
            subscriber_network: None,
            subscriber_process_identifier: 3,
            monitored_object_identifier: oid,
            issue_confirmed_notifications: false,
            expires_at: None,
            last_notified_observation: None,
            monitored_property: Some(TV),
            monitored_property_array_index: None,
            cov_increment: Some(2.0),
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        })
        .unwrap();
    // RAMP_TO_CCT 4400 K (0x1130) at 20 K/s (0x14), from 4000 K: 20 s.
    command(&server, oid, &[0x09, 0x03, 0x2A, 0x11, 0x30, 0x49, 0x14]).await;
    assert_eq!(reports(&sent), [report(3, TV, kelvin(4_000))]);
    for expected in [4_002, 4_004, 4_006, 4_008, 4_010] {
        advance(100).await;
        assert_eq!(reports(&sent), [report(3, TV, kelvin(expected))]);
    }
    server.stop().await.unwrap();
}
