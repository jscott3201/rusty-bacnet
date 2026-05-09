use super::*;
use crate::enums::{ObjectType, PropertyIdentifier};

// --- BACnetDateRange ---

#[test]
fn date_range_encode_decode_round_trip() {
    let range = BACnetDateRange {
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
    };
    let encoded = range.encode();
    assert_eq!(encoded.len(), 8);
    let decoded = BACnetDateRange::decode(&encoded).unwrap();
    assert_eq!(range, decoded);
}

#[test]
fn date_range_encode_decode_all_unspecified() {
    let range = BACnetDateRange {
        start_date: Date {
            year: Date::UNSPECIFIED,
            month: Date::UNSPECIFIED,
            day: Date::UNSPECIFIED,
            day_of_week: Date::UNSPECIFIED,
        },
        end_date: Date {
            year: Date::UNSPECIFIED,
            month: Date::UNSPECIFIED,
            day: Date::UNSPECIFIED,
            day_of_week: Date::UNSPECIFIED,
        },
    };
    let encoded = range.encode();
    let decoded = BACnetDateRange::decode(&encoded).unwrap();
    assert_eq!(range, decoded);
}

#[test]
fn date_range_buffer_too_short() {
    // 7 bytes — one short
    let result = BACnetDateRange::decode(&[0; 7]);
    assert!(result.is_err());
    match result.unwrap_err() {
        Error::BufferTooShort { need, have } => {
            assert_eq!(need, 8);
            assert_eq!(have, 7);
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn date_range_buffer_empty() {
    let result = BACnetDateRange::decode(&[]);
    assert!(result.is_err());
}

#[test]
fn date_range_extra_bytes_ignored() {
    let range = BACnetDateRange {
        start_date: Date {
            year: 100,
            month: 6,
            day: 15,
            day_of_week: 5,
        },
        end_date: Date {
            year: 100,
            month: 6,
            day: 30,
            day_of_week: 6,
        },
    };
    let encoded = range.encode();
    let mut extended = encoded.to_vec();
    extended.extend_from_slice(&[0xFF, 0xFF]); // extra bytes
    let decoded = BACnetDateRange::decode(&extended).unwrap();
    assert_eq!(range, decoded);
}

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
        valid_days: 0b0111_1111, // all days
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
        transitions: 0b0000_0111, // all transitions
    };
    assert_eq!(dest.valid_days & 0x7F, 0x7F);
    assert!(dest.issue_confirmed_notifications);
    assert_eq!(dest.transitions & 0x07, 0x07);
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
        error_class: 2,
        error_code: 31,
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
        log_datum: LogDatum::LogStatus(0b010), // buffer-purged
        status_flags: Some(0b0100),            // FAULT set
    };
    assert_eq!(record.status_flags, Some(0b0100));
    match record.log_datum {
        LogDatum::LogStatus(s) => assert_eq!(s, 0b010),
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
            value: vec![0x10, 0x00], // raw-tagged Null
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
    let dev_oid = ObjectIdentifier::new(ObjectType::DEVICE, 1).unwrap();
    let vs = BACnetValueSource::Object(dev_oid);
    match vs {
        BACnetValueSource::Object(oid) => assert_eq!(oid.instance_number(), 1),
        _ => panic!("wrong variant"),
    }
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
