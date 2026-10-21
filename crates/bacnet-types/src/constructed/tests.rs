use super::*;
use crate::enums::{ObjectType, PropertyIdentifier};

// --- BACnetWeekNDay ---

#[test]
fn week_n_day_encode_decode_round_trip() {
    let wnd = BACnetWeekNDay {
        month: 3,
        week_of_month: 2,
        day_of_week: 5, // Friday
    };
    let encoded = wnd.encode();
    assert_eq!(encoded.len(), 3);
    let decoded = BACnetWeekNDay::decode(&encoded).unwrap();
    assert_eq!(wnd, decoded);
}

#[test]
fn week_n_day_encode_decode_all_any() {
    let wnd = BACnetWeekNDay {
        month: BACnetWeekNDay::ANY,
        week_of_month: BACnetWeekNDay::ANY,
        day_of_week: BACnetWeekNDay::ANY,
    };
    let encoded = wnd.encode();
    assert_eq!(encoded, [0xFF, 0xFF, 0xFF]);
    let decoded = BACnetWeekNDay::decode(&encoded).unwrap();
    assert_eq!(wnd, decoded);
}

#[test]
fn week_n_day_buffer_too_short() {
    // 2 bytes — one short
    let result = BACnetWeekNDay::decode(&[0x03, 0x02]);
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::BufferTooShort { need, have } => {
            assert_eq!(need, 3);
            assert_eq!(have, 2);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn week_n_day_buffer_empty() {
    let result = BACnetWeekNDay::decode(&[]);
    assert!(result.is_err());
}

// --- BACnetObjectPropertyReference ---

#[test]
fn object_property_reference_basic_construction() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let opr = BACnetObjectPropertyReference::new(oid, 85); // prop 85 = present-value
    assert_eq!(opr.object_identifier, oid);
    assert_eq!(opr.property_identifier, 85);
    assert_eq!(opr.property_array_index, None);
}

#[test]
fn object_property_reference_with_index() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let opr = BACnetObjectPropertyReference::new_indexed(oid, 85, 3);
    assert_eq!(opr.property_array_index, Some(3));
}

#[test]
fn object_property_reference_equality() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 5).unwrap();
    let a = BACnetObjectPropertyReference::new(oid, 85);
    let b = BACnetObjectPropertyReference::new(oid, 85);
    assert_eq!(a, b);

    let c = BACnetObjectPropertyReference::new(oid, 77); // different property
    assert_ne!(a, c);
}

// --- BACnetDeviceObjectPropertyReference ---

#[test]
fn device_object_property_reference_local() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 10).unwrap();
    let dopr = BACnetDeviceObjectPropertyReference::new_local(oid, 85);
    assert_eq!(dopr.object_identifier, oid);
    assert_eq!(dopr.property_identifier, 85);
    assert_eq!(dopr.property_array_index, None);
    assert_eq!(dopr.device_identifier, None);
}

#[test]
fn device_object_property_reference_remote() {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 10).unwrap();
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1234).unwrap();
    let dopr = BACnetDeviceObjectPropertyReference::new_remote(oid, 85, dev_oid);
    assert_eq!(dopr.device_identifier, Some(dev_oid));
}

#[test]
fn device_object_property_reference_with_index() {
    let oid = ObjectIdentifier::new(ObjectType::MULTI_STATE_INPUT, 3).unwrap();
    let dopr = BACnetDeviceObjectPropertyReference::new_local(oid, 74).with_index(2); // prop 74 = state-text
    assert_eq!(dopr.property_array_index, Some(2));
    assert_eq!(dopr.device_identifier, None);
}

// --- BACnetAddress ---

#[test]
fn bacnet_address_local_broadcast() {
    let addr = BACnetAddress::local_broadcast();
    assert_eq!(addr.network_number, 0);
    assert!(addr.mac_address.is_empty());
}

#[test]
fn bacnet_address_from_ip() {
    let ip_port: [u8; 6] = [192, 168, 1, 100, 0xBA, 0xC0]; // 192.168.1.100:47808
    let addr = BACnetAddress::from_ip(ip_port);
    assert_eq!(addr.network_number, 0);
    assert_eq!(addr.mac_address.as_slice(), &ip_port);
}

// --- BACnetRecipient ---

#[test]
fn bacnet_recipient_device_variant() {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap();
    let recipient = BACnetRecipient::Device(dev_oid);
    match recipient {
        BACnetRecipient::Device(oid) => assert_eq!(oid.instance_number(), 42),
        BACnetRecipient::Address(_) => panic!("wrong variant"),
    }
}

#[test]
fn bacnet_recipient_address_variant() {
    let addr = BACnetAddress {
        network_number: 100,
        mac_address: MacAddr::from_slice(&[0x01, 0x02, 0x03]),
    };
    let recipient = BACnetRecipient::Address(addr.clone());
    match recipient {
        BACnetRecipient::Device(_) => panic!("wrong variant"),
        BACnetRecipient::Address(a) => assert_eq!(a, addr),
    }
}

// --- BACnetDestination ---

#[test]
fn bacnet_destination_construction() {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 99).unwrap();
    let dest = BACnetDestination {
        valid_days: DaysOfWeek::all(),
        from_time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        to_time: Time {
            hour: 23,
            minute: 59,
            second: 59,
            hundredths: 99,
        },
        recipient: BACnetRecipient::Device(dev_oid),
        process_identifier: 1,
        issue_confirmed_notifications: true,
        transitions: EventTransitionBits::all(),
    };
    assert_eq!(dest.valid_days, DaysOfWeek::all());
    assert!(dest.issue_confirmed_notifications);
    assert_eq!(dest.transitions, EventTransitionBits::all());
}

// --- LogDatum ---

#[test]
fn log_datum_variants_clone_eq() {
    let real = LogDatum::RealValue(72.5_f32);
    assert_eq!(real.clone(), LogDatum::RealValue(72.5_f32));

    let bits = LogDatum::BitstringValue {
        unused_bits: 3,
        data: vec![0b1010_0000],
    };
    assert_eq!(bits.clone(), bits);

    let fail = LogDatum::Failure {
        error_class: crate::enums::ErrorClass::PROPERTY,
        error_code: crate::enums::ErrorCode::UNKNOWN_OBJECT,
    };
    assert_eq!(fail.clone(), fail);

    assert_eq!(LogDatum::NullValue, LogDatum::NullValue);
    assert_ne!(LogDatum::BooleanValue(true), LogDatum::BooleanValue(false));
}

// --- BACnetLogRecord ---

#[test]
fn log_record_construction() {
    let record = BACnetLogRecord {
        date: Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        },
        time: Time {
            hour: 10,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::RealValue(23.4_f32),
        status_flags: None,
    };
    assert_eq!(record.date.year, 124);
    assert_eq!(record.status_flags, None);
}

#[test]
fn log_record_with_status_flags() {
    let record = BACnetLogRecord {
        date: Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        },
        time: Time {
            hour: 0,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        log_datum: LogDatum::LogStatus(crate::bitstring::LogStatus::BUFFER_PURGED),
        status_flags: Some(crate::primitives::StatusFlags::FAULT),
    };
    assert_eq!(
        record.status_flags,
        Some(crate::primitives::StatusFlags::FAULT)
    );
    match record.log_datum {
        LogDatum::LogStatus(s) => assert_eq!(s, crate::bitstring::LogStatus::BUFFER_PURGED),
        _ => panic!("wrong datum variant"),
    }
}

// --- BACnetCalendarEntry ---

#[test]
fn calendar_entry_variants() {
    let date_entry = BACnetCalendarEntry::Date(Date {
        year: 124,
        month: 6,
        day: 15,
        day_of_week: 6,
    });
    let range_entry = BACnetCalendarEntry::DateRange(BACnetDateRange {
        start_date: Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        },
        end_date: Date {
            year: 124,
            month: 12,
            day: 31,
            day_of_week: 2,
        },
    });
    let wnd_entry = BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
        month: BACnetWeekNDay::ANY,
        week_of_month: 1,
        day_of_week: 1, // first Monday of every month
    });
    // Just verify they can be constructed and cloned
    let _a = date_entry.clone();
    let _b = range_entry.clone();
    let _c = wnd_entry.clone();
}

// --- BACnetSpecialEvent ---

#[test]
fn special_event_inline_calendar_entry() {
    let event = BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(BACnetWeekNDay {
            month: 12,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: BACnetWeekNDay::ANY,
        })),
        list_of_time_values: vec![BACnetTimeValue {
            time: Time {
                hour: 8,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            value: PropertyValue::Null,
        }],
        event_priority: 16, // lowest priority
    };
    assert_eq!(event.event_priority, 16);
    assert_eq!(event.list_of_time_values.len(), 1);
}

#[test]
fn special_event_calendar_reference() {
    let cal_oid = ObjectIdentifier::new(ObjectType::CALENDAR, 0).unwrap();
    let event = BACnetSpecialEvent {
        period: SpecialEventPeriod::CalendarReference(cal_oid),
        list_of_time_values: vec![],
        event_priority: 1, // highest priority
    };
    match &event.period {
        SpecialEventPeriod::CalendarReference(oid) => {
            assert_eq!(oid.instance_number(), 0);
        }
        SpecialEventPeriod::CalendarEntry(_) => panic!("wrong period variant"),
    }
}

// --- BACnetRecipientProcess ---

#[test]
fn recipient_process_construction() {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 100).unwrap();
    let rp = BACnetRecipientProcess {
        recipient: BACnetRecipient::Device(dev_oid),
        process_identifier: 42,
    };
    assert_eq!(rp.process_identifier, 42);
    match &rp.recipient {
        BACnetRecipient::Device(oid) => assert_eq!(oid.instance_number(), 100),
        BACnetRecipient::Address(_) => panic!("wrong variant"),
    }
}

// --- BACnetCOVSubscription ---

#[test]
fn cov_subscription_creation() {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 200).unwrap();
    let ai_oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let sub = BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 7,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            ai_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        issue_confirmed_notifications: true,
        time_remaining: 300,
        cov_increment: Some(0.5),
    };
    assert_eq!(sub.recipient.process_identifier, 7);
    assert_eq!(
        sub.monitored_property_reference
            .object_identifier
            .instance_number(),
        1
    );
    assert!(sub.issue_confirmed_notifications);
    assert_eq!(sub.time_remaining, 300);
    assert_eq!(sub.cov_increment, Some(0.5));
}

#[test]
fn cov_subscription_without_increment() {
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 50).unwrap();
    let bv_oid = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 3).unwrap();
    let sub = BACnetCOVSubscription {
        recipient: BACnetRecipientProcess {
            recipient: BACnetRecipient::Device(dev_oid),
            process_identifier: 1,
        },
        monitored_property_reference: BACnetObjectPropertyReference::new(
            bv_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ),
        issue_confirmed_notifications: false,
        time_remaining: 0,
        cov_increment: None,
    };
    assert!(!sub.issue_confirmed_notifications);
    assert_eq!(sub.cov_increment, None);
}

// --- BACnetValueSource ---

#[test]
fn value_source_none_variant() {
    let vs = BACnetValueSource::None;
    assert_eq!(vs, BACnetValueSource::None);
}

#[test]
fn value_source_object_variant() {
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let object = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 7).unwrap();
    for device_identifier in [None, Some(device)] {
        let reference = BACnetDeviceObjectReference {
            device_identifier,
            object_identifier: object,
        };
        let source = BACnetValueSource::Object(reference.clone());
        assert_eq!(source, BACnetValueSource::Object(reference));
    }
}

#[test]
fn device_object_reference_device_identifier_must_be_a_device() {
    let object = ObjectIdentifier::new(ObjectType::ACCESS_DOOR, 7).unwrap();
    let reference = |device_identifier| BACnetDeviceObjectReference {
        device_identifier,
        object_identifier: object,
    };
    assert!(reference(None).device_identifier_is_device());
    for instance in [0, 9, ObjectIdentifier::MAX_INSTANCE] {
        let device = ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap();
        assert!(reference(Some(device)).device_identifier_is_device());
    }
    for object_type in [ObjectType::ANALOG_VALUE, ObjectType::ACCESS_DOOR] {
        let other = ObjectIdentifier::new(object_type, 9).unwrap();
        assert!(!reference(Some(other)).device_identifier_is_device());
    }
}

#[test]
fn device_object_property_reference_device_identifier_must_be_a_device() {
    let object = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 7).unwrap();
    let local = BACnetDeviceObjectPropertyReference::new_local(object, 85);
    assert!(local.device_identifier_is_device());
    let device = ObjectIdentifier::new(ObjectType::DEVICE, 9).unwrap();
    assert!(
        BACnetDeviceObjectPropertyReference::new_remote(object, 85, device)
            .device_identifier_is_device()
    );
    for object_type in [ObjectType::ANALOG_VALUE, ObjectType::CHANNEL] {
        let other = ObjectIdentifier::new(object_type, 9).unwrap();
        let reference = BACnetDeviceObjectPropertyReference::new_remote(object, 85, other);
        assert!(!reference.device_identifier_is_device());
    }
}

#[test]
fn a_reference_to_the_reserved_instance_is_unset() {
    let oid = |object_type, instance| ObjectIdentifier::new(object_type, instance).unwrap();
    let reserved = ObjectIdentifier::WILDCARD_INSTANCE;
    let unset = BACnetObjectPropertyReference::new(oid(ObjectType::ACCUMULATOR, reserved), 85);
    assert!(unset.is_unset());
    assert!(BACnetObjectPropertyReference::new_indexed(unset.object_identifier, 87, 3).is_unset());
    assert!(
        !BACnetObjectPropertyReference::new(oid(ObjectType::ACCUMULATOR, reserved - 1), 85)
            .is_unset()
    );

    // Device-qualified: the object or the Device at the reserved instance.
    let object = oid(ObjectType::ANALOG_INPUT, 7);
    let local = BACnetDeviceObjectPropertyReference::new_local(object, 85);
    assert!(!local.is_unset());
    assert!(!BACnetDeviceObjectPropertyReference::new_remote(
        object,
        85,
        oid(ObjectType::DEVICE, 9)
    )
    .is_unset());
    assert!(BACnetDeviceObjectPropertyReference::new_remote(
        object,
        85,
        oid(ObjectType::DEVICE, reserved)
    )
    .is_unset());
    assert!(BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::ANALOG_INPUT, reserved),
        85
    )
    .is_unset());
}

#[test]
fn value_source_address_variant() {
    let addr = BACnetAddress::from_ip([192, 168, 1, 10, 0xBA, 0xC0]);
    let vs = BACnetValueSource::Address(addr.clone());
    match vs {
        BACnetValueSource::Address(a) => assert_eq!(a, addr),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn access_rule_new_sets_each_specifier_from_its_reference() {
    use crate::enums::{AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier};

    let schedule = ObjectIdentifier::new(ObjectType::SCHEDULE, 1).unwrap();
    let time_range = BACnetDeviceObjectPropertyReference::new_local(
        schedule,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    );
    let point = BACnetDeviceObjectReference::from(
        ObjectIdentifier::new(ObjectType::ACCESS_POINT, 2).unwrap(),
    );

    let specified = BACnetAccessRule::new(Some(time_range.clone()), Some(point.clone()), true);
    assert_eq!(
        specified,
        BACnetAccessRule {
            time_range_specifier: AccessRuleTimeRangeSpecifier::SPECIFIED,
            time_range: Some(time_range),
            location_specifier: AccessRuleLocationSpecifier::SPECIFIED,
            location: Some(point),
            enable: true,
        }
    );

    let open = BACnetAccessRule::new(None, None, false);
    assert_eq!(
        open.time_range_specifier,
        AccessRuleTimeRangeSpecifier::ALWAYS
    );
    assert_eq!(open.location_specifier, AccessRuleLocationSpecifier::ALL);
    assert_eq!(
        (open.time_range, open.location, open.enable),
        (None, None, false)
    );
}
