//! A Notification Class lookup that fails closed moves its own
//! undelivered-notification counter once per transition (#1142).

use super::*;
use bacnet_types::enums::EngineeringUnits;

fn no_bindings() -> Arc<RwLock<super::device_bindings::DeviceBindingTable>> {
    Arc::new(RwLock::new(
        super::device_bindings::DeviceBindingTable::new(),
    ))
}

#[tokio::test]
async fn each_failed_closed_lookup_moves_only_its_own_counter() {
    let none = EventNotificationCounters::default();
    for (case, expected) in [
        (
            "missing-class",
            EventNotificationCounters {
                notification_class_missing: 1,
                ..none
            },
        ),
        (
            "list-unavailable",
            EventNotificationCounters {
                recipient_list_unavailable: 1,
                ..none
            },
        ),
        (
            "list-invalid",
            EventNotificationCounters {
                recipient_list_invalid: 1,
                ..none
            },
        ),
        (
            "list-past-the-cap",
            EventNotificationCounters {
                recipient_list_too_long: 1,
                ..none
            },
        ),
        // A valid list with nothing to send to is configuration, not a
        // suppression.
        ("empty-list", none),
        ("no-eligible-destination", none),
    ] {
        let (broadcasts, unicasts, counters) =
            distribute_counted(non_matched_database(case), no_bindings(), DccState::Enable).await;
        assert!(broadcasts.is_empty() && unicasts.is_empty(), "{case}");
        assert_eq!(counters, expected, "{case}");
    }
}

#[tokio::test]
async fn delivered_and_dcc_held_transitions_move_no_counter() {
    let mut db = clocked_test_database();
    db.add(Box::new(TestNotificationClass::new(
        TestRecipientList::Broadcasts(CAP),
    )))
    .unwrap();
    let (broadcasts, _, counters) = distribute_counted(db, no_bindings(), DccState::Enable).await;
    assert_eq!(broadcasts.len(), CAP as usize);
    assert_eq!(counters, EventNotificationCounters::default());

    // DCC holds every notification back before the lookup runs, so even a
    // missing class is not counted while initiation is disabled.
    let (broadcasts, unicasts, counters) = distribute_counted(
        non_matched_database("missing-class"),
        no_bindings(),
        DccState::DisableInitiation,
    )
    .await;
    assert!(broadcasts.is_empty() && unicasts.is_empty());
    assert_eq!(counters, EventNotificationCounters::default());
}

/// A running server counts on its own write path, through the public
/// accessor, and its totals stay readable after `stop()`.
#[tokio::test]
async fn running_server_counts_a_missing_class_from_a_local_write() {
    let mut ai = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    for (property, value) in [
        (PropertyIdentifier::HIGH_LIMIT, 80.0f32),
        (PropertyIdentifier::LOW_LIMIT, 20.0),
        (PropertyIdentifier::DEADBAND, 2.0),
    ] {
        ai.write_property(property, None, PropertyValue::Real(value), None)
            .unwrap();
    }
    for (property, unused_bits, byte) in [
        (PropertyIdentifier::LIMIT_ENABLE, 6, 0xC0),
        (PropertyIdentifier::EVENT_ENABLE, 5, 0xE0),
    ] {
        let value = PropertyValue::BitString {
            unused_bits,
            data: vec![byte],
        };
        ai.write_property(property, None, value, None).unwrap();
    }
    ai.set_present_value(50.0);
    let mut db = ObjectDatabase::new();
    db.add(Box::new(ai)).unwrap();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::default().with_name("Dev"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    // No Notification Class 0, which the AnalogInput names by default.
    let (transport, sent) = routing_transport();
    let mut server = BACnetServer::start_clockless(ServerConfig::default(), db, transport)
        .await
        .unwrap();
    assert_eq!(
        server.event_notification_counters(),
        EventNotificationCounters::default()
    );

    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    server
        .set_present_value_local(&oid, PropertyValue::Real(81.0))
        .await
        .unwrap();
    let expected = EventNotificationCounters {
        notification_class_missing: 1,
        ..Default::default()
    };
    assert_eq!(server.event_notification_counters(), expected);
    assert!(sent.broadcasts().is_empty() && sent.unicasts().is_empty());

    server.stop().await.unwrap();
    assert_eq!(server.event_notification_counters(), expected);
}
