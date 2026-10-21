//! Access Zone intrinsic reporting through the running server (#1305): a
//! CHANGE_OF_STATE on Occupancy_State reaches the recipients of the zone's
//! Notification Class once Time_Delay has run, Event_Enable withholds only
//! the distribution, a fault notification reports Occupancy_State, the
//! zone's Table 13-5 property, and a zone in alarm is listed by
//! GetEventInformation, GetAlarmSummary and GetEnrollmentSummary.

use super::event_notifications_tests::{
    decode_broadcast_notification, local_broadcast_destination, recording_transport,
};
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_encoding::constructed::decode_bacnet_property_value;
use bacnet_objects::access_control::AccessZoneObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::notification_class::NotificationClass;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::alarm_event::NotificationParameters;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::constructed::BACnetPropertyStates;
use bacnet_types::enums::{
    AccessZoneOccupancyState, EventState, EventType, NotifyType, Reliability,
};
use bacnet_types::primitives::StatusFlags;
use bytes::Bytes;
use PropertyIdentifier as P;

/// The zone's Notification Class; class 0 doesn't exist here, so a
/// notification can reach the wire only by following the zone's own one.
const CLASS: u32 = 5;

/// A zone with an upper limit of 5 that alarms above it after `time_delay`
/// seconds and distributes the transitions in `enable`, configured as a
/// client would configure it.
fn zone(time_delay: u64, enable: EventTransitionBits) -> AccessZoneObject {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.set_occupancy_limits(0, 5).unwrap();
    for (property, value) in [
        (
            P::ALARM_VALUES,
            PropertyValue::List(vec![PropertyValue::Enumerated(
                AccessZoneOccupancyState::ABOVE_UPPER_LIMIT.to_raw(),
            )]),
        ),
        (P::TIME_DELAY, PropertyValue::Unsigned(time_delay)),
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(CLASS.into())),
        (
            P::EVENT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![enable.to_bacnet()],
            },
        ),
    ] {
        zone.write_property(property, None, value, None).unwrap();
    }
    zone
}

async fn start(zone: AccessZoneObject) -> (BACnetServer<TestTransport>, ObjectIdentifier, SendLog) {
    let (transport, sent) = recording_transport();
    let oid = zone.object_identifier();
    let mut class = NotificationClass::new(CLASS, "NC-5").unwrap();
    class
        .add_destination(local_broadcast_destination())
        .unwrap();
    let mut db = clocked_test_database();
    db.add(Box::new(zone)).unwrap();
    db.add(Box::new(class)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::start(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    (server, oid, sent)
}

/// A local write, run through the same post-write event path as a client's.
async fn write(
    server: &BACnetServer<TestTransport>,
    oid: ObjectIdentifier,
    property: P,
    value: PropertyValue,
) {
    server
        .write_local(
            &oid,
            property,
            None,
            value,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
}

fn take(sent: &SendLog) -> Vec<Bytes> {
    sent.take().into_iter().map(|frame| frame.npdu).collect()
}

async fn event_state(server: &BACnetServer<TestTransport>, oid: ObjectIdentifier) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(P::EVENT_STATE, None)
        .unwrap()
}

fn change_of_state(state: AccessZoneOccupancyState, flags: StatusFlags) -> NotificationParameters {
    NotificationParameters::ChangeOfState {
        new_state: BACnetPropertyStates::ZoneOccupancyState(state.to_raw()),
        status_flags: flags,
    }
}

#[tokio::test(start_paused = true)]
async fn access_zone_occupancy_alarm_reaches_notification_class_recipients() {
    let (server, oid, sent) = start(zone(2, EventTransitionBits::all())).await;

    write(&server, oid, P::ADJUST_VALUE, PropertyValue::Signed(6)).await;
    assert!(sent.is_empty(), "Time_Delay holds the transition back");
    tokio::time::sleep(Duration::from_secs(5)).await;
    let offnormal = decode_broadcast_notification(&take(&sent));
    assert_eq!(offnormal.event_object_identifier, oid);
    assert_eq!(offnormal.notification_class, CLASS);
    assert_eq!(offnormal.event_type, EventType::CHANGE_OF_STATE);
    assert_eq!(offnormal.notify_type, NotifyType::ALARM);
    assert_eq!(
        (offnormal.from_state, offnormal.to_state),
        (EventState::NORMAL, EventState::OFFNORMAL)
    );
    assert_eq!(
        offnormal.event_values,
        Some(change_of_state(
            AccessZoneOccupancyState::ABOVE_UPPER_LIMIT,
            StatusFlags::IN_ALARM,
        ))
    );
    assert_eq!(
        event_state(&server, oid).await,
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw())
    );

    // Three leave: back at 3, inside the limit.
    write(&server, oid, P::ADJUST_VALUE, PropertyValue::Signed(-3)).await;
    assert!(sent.is_empty());
    tokio::time::sleep(Duration::from_secs(5)).await;
    let normal = decode_broadcast_notification(&take(&sent));
    assert_eq!(
        (normal.from_state, normal.to_state),
        (EventState::OFFNORMAL, EventState::NORMAL)
    );
    assert_eq!(
        normal.event_values,
        Some(change_of_state(
            AccessZoneOccupancyState::NORMAL,
            StatusFlags::empty(),
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn access_zone_event_enable_withholds_only_the_distribution() {
    let (server, oid, sent) = start(zone(0, EventTransitionBits::TO_NORMAL)).await;

    write(&server, oid, P::ADJUST_VALUE, PropertyValue::Signed(6)).await;
    assert!(sent.is_empty(), "TO_OFFNORMAL is not distributed");
    assert_eq!(
        event_state(&server, oid).await,
        PropertyValue::Enumerated(EventState::OFFNORMAL.to_raw()),
        "the transition still happened"
    );

    // Zero clears the count.
    write(&server, oid, P::ADJUST_VALUE, PropertyValue::Signed(0)).await;
    let normal = decode_broadcast_notification(&take(&sent));
    assert_eq!(
        (normal.from_state, normal.to_state),
        (EventState::OFFNORMAL, EventState::NORMAL)
    );
}

#[tokio::test(start_paused = true)]
async fn access_zone_fault_notification_reports_occupancy_state() {
    let (server, oid, sent) = start(zone(0, EventTransitionBits::all())).await;

    write(
        &server,
        oid,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(true),
    )
    .await;
    assert!(sent.is_empty());
    let unreliable = Reliability::UNRELIABLE_OTHER;
    write(
        &server,
        oid,
        P::RELIABILITY,
        PropertyValue::Enumerated(unreliable.to_raw()),
    )
    .await;
    let fault = decode_broadcast_notification(&take(&sent));
    assert_eq!(fault.event_type, EventType::CHANGE_OF_RELIABILITY);
    assert_eq!(
        (fault.from_state, fault.to_state),
        (EventState::NORMAL, EventState::FAULT)
    );
    let Some(NotificationParameters::ChangeOfReliability {
        reliability,
        status_flags,
        property_values,
    }) = fault.event_values
    else {
        panic!(
            "expected CHANGE_OF_RELIABILITY values, got {:?}",
            fault.event_values
        );
    };
    assert_eq!(reliability, unreliable);
    assert_eq!(
        status_flags,
        StatusFlags::IN_ALARM | StatusFlags::FAULT | StatusFlags::OUT_OF_SERVICE
    );
    // Occupancy_State alone, NORMAL with the count at zero.
    let (entry, end) = decode_bacnet_property_value(&property_values, 0).unwrap();
    assert_eq!(end, property_values.len());
    assert_eq!(entry.property_identifier, P::OCCUPANCY_STATE);
    assert_eq!(entry.value, [0x91, 0x00]);
}

#[tokio::test(start_paused = true)]
async fn access_zone_in_alarm_is_listed_by_the_summary_services() {
    use crate::handlers::{
        handle_get_alarm_summary, handle_get_enrollment_summary, handle_get_event_information,
    };
    use bacnet_services::alarm_event::{GetEventInformationAck, GetEventInformationRequest};
    use bacnet_services::alarm_summary::GetAlarmSummaryAck;
    use bacnet_services::enrollment_summary::{
        GetEnrollmentSummaryAck, GetEnrollmentSummaryRequest,
    };
    use bacnet_types::enums::AcknowledgmentFilter;

    let (server, oid, _sent) = start(zone(0, EventTransitionBits::all())).await;
    write(&server, oid, P::ADJUST_VALUE, PropertyValue::Signed(6)).await;
    let db = server.database().read().await;

    let mut request = BytesMut::new();
    GetEventInformationRequest {
        last_received_object_identifier: None,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_get_event_information(&db, &request, &mut ack).unwrap();
    let ack = GetEventInformationAck::decode(&ack).unwrap();
    let [summary] = ack.list_of_event_summaries.as_slice() else {
        panic!("one event summary, got {:?}", ack.list_of_event_summaries);
    };
    assert_eq!(summary.object_identifier, oid);
    assert_eq!(summary.event_state, EventState::OFFNORMAL);
    assert_eq!(summary.notify_type, NotifyType::ALARM);
    assert_eq!(summary.event_enable, EventTransitionBits::all());

    let mut ack = BytesMut::new();
    handle_get_alarm_summary(&db, &mut ack).unwrap();
    let alarms = GetAlarmSummaryAck::decode(&ack).unwrap().entries;
    assert_eq!(
        alarms
            .iter()
            .map(|entry| (entry.object_identifier, entry.alarm_state))
            .collect::<Vec<_>>(),
        [(oid, EventState::OFFNORMAL)]
    );

    let mut request = BytesMut::new();
    GetEnrollmentSummaryRequest {
        acknowledgment_filter: AcknowledgmentFilter::ALL,
        enrollment_filter: None,
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    }
    .encode(&mut request);
    let mut ack = BytesMut::new();
    handle_get_enrollment_summary(&db, &request, &mut ack).unwrap();
    let enrollments = GetEnrollmentSummaryAck::decode(&ack).unwrap().entries;
    let [enrollment] = enrollments.as_slice() else {
        panic!("one enrollment, got {enrollments:?}");
    };
    assert_eq!(enrollment.object_identifier, oid);
    assert_eq!(enrollment.event_type, EventType::CHANGE_OF_STATE);
    assert_eq!(enrollment.event_state, EventState::OFFNORMAL);
    assert_eq!(enrollment.notification_class, Some(CLASS));
}
