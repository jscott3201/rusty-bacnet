//! The object's rows (Table 12-58): reads, writes and Property_List.

use super::*;
use crate::property_metadata::PropertyConformance;
use bacnet_types::enums::PropertyIdentifier as P;

fn forwarder() -> NotificationForwarderObject {
    NotificationForwarderObject::new(3, "NF-3").unwrap()
}

#[test]
fn notification_forwarder_serves_every_required_row() {
    let nf = forwarder();
    let required: Vec<_> = nf.required_properties().iter().copied().collect();
    assert_eq!(
        required,
        [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::STATUS_FLAGS,
            P::RELIABILITY,
            P::OUT_OF_SERVICE,
            P::RECIPIENT_LIST,
            P::SUBSCRIBED_RECIPIENTS,
            P::PROCESS_IDENTIFIER_FILTER,
            P::LOCAL_FORWARDING_ONLY,
            P::PROPERTY_LIST,
        ]
    );
    for property in required {
        assert!(
            nf.read_property(property, None).is_ok(),
            "{property:?} must read"
        );
    }
    let metadata = nf.property_metadata();
    let subscribed = metadata
        .iter()
        .find(|row| row.property_identifier == P::SUBSCRIBED_RECIPIENTS)
        .unwrap();
    assert_eq!(subscribed.conformance, PropertyConformance::RequiredWrite);
    assert!(subscribed.write_capability.is_writable());

    assert_eq!(
        nf.read_property(P::OBJECT_TYPE, None).unwrap(),
        PropertyValue::Enumerated(ObjectType::NOTIFICATION_FORWARDER.to_raw())
    );
    assert_eq!(
        nf.read_property(P::RELIABILITY, None).unwrap(),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    assert_eq!(
        nf.read_property(P::PROCESS_IDENTIFIER_FILTER, None)
            .unwrap(),
        PropertyValue::Null
    );
    assert_eq!(
        nf.read_property(P::LOCAL_FORWARDING_ONLY, None).unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        nf.read_property(P::RECIPIENT_LIST, None).unwrap(),
        PropertyValue::ApplicationData(Vec::new())
    );
    assert_eq!(
        nf.read_property(P::SUBSCRIBED_RECIPIENTS, None).unwrap(),
        PropertyValue::ApplicationData(Vec::new())
    );
    // Port_Filter is left out until the application configures it.
    assert!(!nf.property_list().contains(&P::PORT_FILTER));
    assert_refused(
        nf.read_property(P::PORT_FILTER, None).map(|_| ()),
        ErrorClass::PROPERTY,
        ErrorCode::UNKNOWN_PROPERTY,
    );
    assert!(nf.is_list_property(P::SUBSCRIBED_RECIPIENTS));
    assert!(nf.is_list_property(P::RECIPIENT_LIST));
    assert!(nf.is_array_property(P::PORT_FILTER));
}

#[test]
fn notification_forwarder_filter_and_service_rows_take_writes() {
    let mut nf = forwarder();
    nf.write_property(
        P::PROCESS_IDENTIFIER_FILTER,
        None,
        PropertyValue::Unsigned(9),
        None,
    )
    .unwrap();
    assert_eq!(nf.process_identifier_filter(), Some(9));
    assert_eq!(
        nf.read_property(P::PROCESS_IDENTIFIER_FILTER, None)
            .unwrap(),
        PropertyValue::Unsigned(9)
    );
    nf.write_property(
        P::PROCESS_IDENTIFIER_FILTER,
        None,
        PropertyValue::Null,
        None,
    )
    .unwrap();
    assert_eq!(nf.process_identifier_filter(), None);
    assert_refused(
        nf.write_property(
            P::PROCESS_IDENTIFIER_FILTER,
            None,
            PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
            None,
        ),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_refused(
        nf.write_property(
            P::PROCESS_IDENTIFIER_FILTER,
            None,
            PropertyValue::Real(1.0),
            None,
        ),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );

    nf.write_property(
        P::LOCAL_FORWARDING_ONLY,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    assert!(nf.local_forwarding_only());
    nf.write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    assert_eq!(
        nf.read_property(P::OUT_OF_SERVICE, None).unwrap(),
        PropertyValue::Boolean(true)
    );
    // Reliability never leaves NO_FAULT_DETECTED, so it takes no writes.
    assert_refused(
        nf.write_property(
            P::RELIABILITY,
            None,
            PropertyValue::Enumerated(Reliability::OPEN_LOOP.to_raw()),
            None,
        ),
        ErrorClass::PROPERTY,
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn notification_forwarder_recipient_list_takes_framed_writes_up_to_the_cap() {
    let mut nf = forwarder();
    let list = [
        destination(address(0, &[1]), 4, false),
        destination(device(20), 5, true),
    ];
    nf.write_property(P::RECIPIENT_LIST, None, framed_destinations(&list), None)
        .unwrap();
    assert_eq!(nf.recipient_list(), list);
    assert_eq!(
        nf.read_property(P::RECIPIENT_LIST, None).unwrap(),
        framed_destinations(&list)
    );
    let full = vec![destination(address(0, &[1]), 4, false); MAX_RECIPIENT_LIST_DESTINATIONS + 1];
    assert!(
        nf.write_property(P::RECIPIENT_LIST, None, framed_destinations(&full), None)
            .is_err()
    );
    assert_eq!(nf.recipient_list(), list, "a refused write changes nothing");
    assert_refused(
        nf.write_property(P::RECIPIENT_LIST, Some(1), framed_destinations(&list), None),
        ErrorClass::PROPERTY,
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
    );
    for _ in list.len()..MAX_RECIPIENT_LIST_DESTINATIONS {
        nf.add_destination(destination(device(1), 1, false))
            .unwrap();
    }
    assert!(
        nf.add_destination(destination(device(1), 1, false))
            .is_err()
    );
}

#[test]
fn notification_forwarder_subscribed_recipients_take_writes_and_lapse() {
    let mut nf = forwarder();
    let (clock, set) = manual_clock();
    nf.bind_monotonic_clock_internal(Some(clock));
    let written = [subscription(device(7), 1, 2), subscription(device(8), 1, 5)];
    nf.write_property(
        P::SUBSCRIBED_RECIPIENTS,
        None,
        framed_subscriptions(&written),
        None,
    )
    .unwrap();
    assert_eq!(nf.subscriptions(), written);
    assert_eq!(nf.next_monotonic_deadline_internal(), Some(2 * MINUTE));
    set(2 * MINUTE);
    assert!(nf.advance_monotonic_time_internal(2 * MINUTE));
    assert_eq!(nf.subscriptions(), [subscription(device(8), 1, 3)]);
    assert_refused(
        nf.write_property(
            P::SUBSCRIBED_RECIPIENTS,
            None,
            framed_subscriptions(&[subscription(device(9), 1, 0)]),
            None,
        ),
        ErrorClass::PROPERTY,
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}

#[test]
fn notification_forwarder_port_filter_writes_change_only_enabled_members() {
    let mut nf = forwarder();
    nf.set_port_filter(Some(vec![
        BACnetPortPermission {
            port_id: 0,
            enabled: true,
        },
        BACnetPortPermission {
            port_id: 4,
            enabled: true,
        },
    ]));
    assert!(nf.property_list().contains(&P::PORT_FILTER));
    assert_eq!(
        nf.read_property(P::PORT_FILTER, Some(0)).unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        nf.read_property(P::PORT_FILTER, Some(2)).unwrap(),
        port(4, true)
    );

    nf.write_property(P::PORT_FILTER, Some(2), port(4, false), None)
        .unwrap();
    nf.write_property(
        P::PORT_FILTER,
        None,
        PropertyValue::List(vec![port(0, false), port(4, false)]),
        None,
    )
    .unwrap();
    assert_eq!(
        nf.read_property(P::PORT_FILTER, None).unwrap(),
        PropertyValue::List(vec![port(0, false), port(4, false)])
    );

    // The size and the Port_IDs are fixed.
    for (index, value, code) in [
        (
            Some(0),
            PropertyValue::Unsigned(3),
            ErrorCode::WRITE_ACCESS_DENIED,
        ),
        (Some(1), port(5, true), ErrorCode::VALUE_OUT_OF_RANGE),
        (Some(3), port(0, true), ErrorCode::INVALID_ARRAY_INDEX),
        (
            None,
            PropertyValue::List(vec![port(0, true)]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            None,
            PropertyValue::List(vec![port(4, true), port(0, true)]),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            None,
            PropertyValue::Boolean(true),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        assert_refused(
            nf.write_property(P::PORT_FILTER, index, value, None),
            ErrorClass::PROPERTY,
            code,
        );
    }
    assert_eq!(
        nf.port_filter().unwrap(),
        [
            BACnetPortPermission {
                port_id: 0,
                enabled: false
            },
            BACnetPortPermission {
                port_id: 4,
                enabled: false
            },
        ],
        "refused writes change nothing"
    );
}
