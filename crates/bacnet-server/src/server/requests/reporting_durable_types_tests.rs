//! The reporting settings of Access Door and Zone and of Event and Alert
//! Enrollment through the server (#2331): a WritePropertyMultiple of several
//! rows stages one save on these objects with no Tags storage, and an Alert
//! Enrollment's refusal of an ACK_NOTIFICATION Notify_Type saves nothing for
//! it, on its own or inside a WritePropertyMultiple.
//!
//! PROPERTY 2; VALUE_OUT_OF_RANGE 37; Notify_Type is property 72.

use super::durable_write_wire_tests::{ERROR_PDU, SIMPLE_ACK_WPM};
use super::mutation_list_wire_tests::wire;
use super::mutation_tests::{oid, value, wpm};
use super::reporting_durable_tests::{INSTANCE, served_by, write_property};
use super::write_config_durable_tests::ConfigStorage;
use super::*;
use bacnet_objects::write_config::WriteConfigSnapshot;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::bitstring::EventTransitionBits;
use bacnet_types::enums::NotifyType;
use std::sync::atomic::Ordering;

use PropertyIdentifier as P;

const TYPES: [ObjectType; 4] = [
    ObjectType::ACCESS_DOOR,
    ObjectType::ACCESS_ZONE,
    ObjectType::EVENT_ENROLLMENT,
    ObjectType::ALERT_ENROLLMENT,
];

fn attempt(property: P, value: PropertyValue) -> BACnetPropertyValue {
    BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: super::mutation_tests::value(value),
        priority: None,
    }
}

fn enable() -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 5,
        data: vec![EventTransitionBits::TO_FAULT.to_bacnet()],
    }
}

fn notify_type(notify_type: NotifyType) -> PropertyValue {
    PropertyValue::Enumerated(notify_type.to_raw())
}

fn saves(storage: &ConfigStorage) -> usize {
    storage.saves.load(Ordering::SeqCst)
}

/// Notification_Class, Event_Enable and Notify_Type in one request stage
/// one save, and a server rebuilt on the storage serves all three.
#[tokio::test(start_paused = true)]
async fn a_write_property_multiple_of_three_rows_saves_once_on_each_type() {
    let rows = [
        (P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
        (P::EVENT_ENABLE, enable()),
        (P::NOTIFY_TYPE, notify_type(NotifyType::EVENT)),
    ];
    for kind in TYPES {
        let storage = Arc::new(ConfigStorage::default());
        let fixture = served_by(kind, &storage).await;
        let request = wpm(vec![WriteAccessSpecification {
            object_identifier: oid(kind, INSTANCE),
            list_of_properties: rows
                .iter()
                .map(|(property, value)| attempt(*property, value.clone()))
                .collect(),
        }]);
        let response = wire(
            &fixture,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            request,
        )
        .await;
        assert_eq!(response, SIMPLE_ACK_WPM, "{kind:?}");
        assert_eq!(saves(&storage), 1, "{kind:?}");
        drop(fixture);
        let rebuilt = served_by(kind, &storage).await;
        for (property, written) in &rows {
            assert_eq!(
                &rebuilt.read(oid(kind, INSTANCE), *property).await,
                written,
                "{kind:?} {property:?} after a rebuild"
            );
        }
    }
}

/// A WriteProperty of ACK_NOTIFICATION to an Alert Enrollment is refused
/// with PROPERTY / VALUE_OUT_OF_RANGE and saves nothing.
#[tokio::test(start_paused = true)]
async fn an_alert_enrollment_refuses_ack_notification_and_saves_nothing() {
    let kind = ObjectType::ALERT_ENROLLMENT;
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served_by(kind, &storage).await;
    let ack = value(notify_type(NotifyType::ACK_NOTIFICATION));
    let request = write_property(kind, P::NOTIFY_TYPE, None, ack);
    let response = wire(&fixture, ConfirmedServiceChoice::WRITE_PROPERTY, request).await;
    assert_eq!(response, [ERROR_PDU, 5, 15, 0x91, 2, 0x91, 37]);
    assert_eq!(saves(&storage), 0);
    assert_eq!(
        fixture.read(oid(kind, INSTANCE), P::NOTIFY_TYPE).await,
        notify_type(NotifyType::ALARM)
    );
}

/// In a WritePropertyMultiple of Notification_Class, an ACK_NOTIFICATION
/// Notify_Type and Event_Detection_Enable to an Alert Enrollment, the class
/// is written and saved once, the error names Notify_Type as the first
/// failed write, and the detection write isn't made.
#[tokio::test(start_paused = true)]
async fn an_alert_enrollment_saves_the_rows_before_a_refused_notify_type_once() {
    let kind = ObjectType::ALERT_ENROLLMENT;
    let alert = oid(kind, INSTANCE);
    let storage = Arc::new(ConfigStorage::default());
    let fixture = served_by(kind, &storage).await;
    let request = wpm(vec![WriteAccessSpecification {
        object_identifier: alert,
        list_of_properties: vec![
            attempt(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
            attempt(P::NOTIFY_TYPE, notify_type(NotifyType::ACK_NOTIFICATION)),
            attempt(P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(false)),
        ],
    }]);
    let response = wire(
        &fixture,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
        request,
    )
    .await;
    // WritePropertyMultiple-Error: the error [0] PROPERTY / VALUE_OUT_OF_RANGE,
    // then the first failed attempt [1]: the enrollment [0] and Notify_Type
    // (72) [1].
    let mut expected = vec![ERROR_PDU, 5, 16, 0x0E, 0x91, 2, 0x91, 37, 0x0F, 0x1E, 0x0C];
    expected.extend_from_slice(&alert.encode());
    expected.extend_from_slice(&[0x19, 72, 0x1F]);
    assert_eq!(response, expected);
    assert_eq!(saves(&storage), 1);
    let Some(Some(WriteConfigSnapshot::Reporting(saved))) = storage.load_saved() else {
        panic!("no reporting settings saved");
    };
    assert_eq!(saved.notification_class, Some(9));
    assert_eq!(
        (saved.notify_type, saved.event_detection_enable),
        (None, None)
    );
    let served = |property| fixture.read(alert, property);
    assert_eq!(
        served(P::NOTIFICATION_CLASS).await,
        PropertyValue::Unsigned(9)
    );
    assert_eq!(
        served(P::EVENT_DETECTION_ENABLE).await,
        PropertyValue::Boolean(true)
    );
    assert_eq!(served(P::NOTIFY_TYPE).await, notify_type(NotifyType::ALARM));
}
