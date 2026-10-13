//! Credential Data Input Present_Value and Update_Time, and Access Point
//! Access_Event_Time, in their Clause 21 forms (#1133).
//!
//! Table 12-43 types Present_Value as a BACnetAuthenticationFactor and
//! Update_Time as a BACnetTimeStamp; Table 12-36 types Access_Event_Time as a
//! BACnetTimeStamp too. Each reads back as one framed value that the shared
//! codecs decode to what the application set.

use bacnet_encoding::constructed::decode_authentication_factor;
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat};
use bacnet_types::enums::AuthenticationFactorType;

use super::*;

/// 2026-09-29 (a Tuesday) 15:00:07.00.
fn date() -> Date {
    Date {
        year: 126,
        month: 9,
        day: 29,
        day_of_week: 2,
    }
}

fn time() -> Time {
    Time {
        hour: 15,
        minute: 0,
        second: 7,
        hundredths: 0,
    }
}

/// Each BACnetTimeStamp choice and its bytes: time `[0]` as four raw octets,
/// sequence-number `[1]` as an Unsigned, datetime `[2]` framed around an
/// application Date and Time.
fn stamps() -> Vec<(BACnetTimeStamp, Vec<u8>)> {
    vec![
        (
            BACnetTimeStamp::Time(time()),
            vec![0x0C, 0x0F, 0x00, 0x07, 0x00],
        ),
        (BACnetTimeStamp::SequenceNumber(7), vec![0x19, 0x07]),
        (BACnetTimeStamp::SequenceNumber(0), vec![0x19, 0x00]),
        (
            BACnetTimeStamp::DateTime {
                date: date(),
                time: time(),
            },
            vec![
                0x2E, 0xA4, 0x7E, 0x09, 0x1D, 0x02, 0xB4, 0x0F, 0x00, 0x07, 0x00, 0x2F,
            ],
        ),
    ]
}

/// The bytes of a framed read, checked to decode as one whole timestamp.
fn stamp_read(value: PropertyValue) -> (Vec<u8>, BACnetTimeStamp) {
    let PropertyValue::ApplicationData(bytes) = value else {
        panic!("expected a framed BACnetTimeStamp, got {value:?}");
    };
    let (stamp, end) = decode_timestamp_choice(&bytes, 0).unwrap();
    assert_eq!(end, bytes.len());
    (bytes, stamp)
}

#[test]
fn credential_data_input_present_value_is_an_authentication_factor() {
    let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    cdi.set_supported_formats([(
        BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::WIEGAND26),
        1,
    )])
    .unwrap();
    let factor = BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::WIEGAND26,
        format_class: 1,
        value: vec![0x12, 0x34, 0x56],
    };
    cdi.set_present_value(factor.clone(), BACnetTimeStamp::SequenceNumber(1))
        .unwrap();
    let PropertyValue::ApplicationData(bytes) = cdi
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
    else {
        panic!("expected a framed BACnetAuthenticationFactor");
    };
    // format type [0] WIEGAND26 (8), format class [1] 1, value [2] 3 octets.
    assert_eq!(bytes, [0x09, 0x08, 0x19, 0x01, 0x2B, 0x12, 0x34, 0x56]);
    assert_eq!(
        decode_authentication_factor(&bytes, 0).unwrap(),
        (factor, bytes.len())
    );
}

#[test]
fn credential_data_input_update_time_serves_each_timestamp_choice() {
    let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    cdi.set_supported_formats([(
        BACnetAuthenticationFactorFormat::standard(AuthenticationFactorType::SIMPLE_NUMBER16),
        0,
    )])
    .unwrap();
    let factor = BACnetAuthenticationFactor {
        format_type: AuthenticationFactorType::SIMPLE_NUMBER16,
        format_class: 0,
        value: vec![0x00, 0x2A],
    };
    for (stamp, expected) in stamps() {
        // The same factor read again still moves Update_Time.
        cdi.set_present_value(factor.clone(), stamp.clone())
            .unwrap();
        let read = cdi
            .read_property(PropertyIdentifier::UPDATE_TIME, None)
            .unwrap();
        assert_eq!(stamp_read(read), (expected, stamp));
    }
}

#[test]
fn access_point_access_event_time_serves_each_timestamp_choice() {
    let mut point = AccessPointObject::new(1, "AP-1").unwrap();
    for (tag, (stamp, expected)) in (1..).zip(stamps()) {
        point
            .set_access_event(AccessEventReport {
                time: Some(stamp.clone()),
                credential: None,
                ..AccessEventReport::new(AccessEvent::GRANTED, tag)
            })
            .unwrap();
        let read = point
            .read_property(PropertyIdentifier::ACCESS_EVENT_TIME, None)
            .unwrap();
        assert_eq!(stamp_read(read), (expected, stamp));
        assert_eq!(
            point
                .read_property(PropertyIdentifier::ACCESS_EVENT_TAG, None)
                .unwrap(),
            PropertyValue::Unsigned(tag)
        );
    }
}
