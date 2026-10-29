//! Writes of the Event_Parameters algorithms #2090 modelled: a body under
//! their tag that isn't the production is refused, a string in a character
//! set the stack can't convert is refused as such, and either refusal
//! leaves the stored value as it was. An unset Time_Delay_Normal reads back
//! the algorithm's own delay.

use super::super::*;
use bacnet_types::constructed::BACnetEventParameter;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

fn write(ee: &mut EventEnrollmentObject, bytes: Vec<u8>) -> Result<(), Error> {
    ee.write_property(
        PropertyIdentifier::EVENT_PARAMETERS,
        None,
        PropertyValue::ApplicationData(bytes),
        None,
    )
}

fn stored(ee: &EventEnrollmentObject) -> BACnetEventParameter {
    let PropertyValue::ApplicationData(bytes) = ee
        .read_property(PropertyIdentifier::EVENT_PARAMETERS, None)
        .unwrap()
    else {
        panic!("expected ApplicationData");
    };
    bacnet_encoding::constructed::decode_event_parameter(&bytes, 0)
        .unwrap()
        .0
}

fn refused_with(result: Result<(), Error>, code: ErrorCode) {
    match result.unwrap_err() {
        Error::Remote(RemoteError::Error { class, code: got }) => {
            assert_eq!((class, got), (ErrorClass::PROPERTY, code));
        }
        other => panic!("expected PROPERTY/{code:?}, got {other:?}"),
    }
}

#[test]
fn a_malformed_body_under_a_modelled_tag_is_refused() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::ACCESS_EVENT).unwrap();
    // access-event [13] whose event list holds NULLs, not enumerations: kept
    // as opaque octets before #2090.
    let malformed = vec![
        0xDE, 0x0E, 0x00, 0x00, 0x0F, 0x1E, 0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55, 0x1F, 0xDF,
    ];
    refused_with(write(&mut ee, malformed), ErrorCode::INVALID_DATA_TYPE);
    assert_eq!(stored(&ee), BACnetEventParameter::None);

    // A stored modelled value stays too.
    let good = BACnetEventParameter::UnsignedRange {
        time_delay: 30,
        low_limit: 10,
        high_limit: 300,
    };
    let mut octets = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_event_parameter(&mut octets, &good).unwrap();
    write(&mut ee, octets.to_vec()).unwrap();
    // unsigned-range [11] with its high limit missing.
    refused_with(
        write(&mut ee, vec![0xBE, 0x09, 0x1E, 0x19, 0x0A, 0xBF]),
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(stored(&ee), good);
}

#[test]
fn an_alarm_string_in_an_unsupported_character_set_is_refused_as_such() {
    let mut ee =
        EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_CHARACTERSTRING).unwrap();
    // change-of-characterstring [17] { [0] 0, [1] { a JIS X 0208 string } }
    // (character set 2), and the same with UCS-4 (3) and DBCS (1).
    for charset in [2u8, 3, 1] {
        let wire = vec![
            0xFE, 0x11, 0x09, 0x00, 0x1E, 0x73, charset, 0x30, 0x42, 0x1F, 0xFF, 0x11,
        ];
        refused_with(write(&mut ee, wire), ErrorCode::CHARACTER_SET_NOT_SUPPORTED);
        assert_eq!(stored(&ee), BACnetEventParameter::None, "charset {charset}");
    }
}

#[test]
fn an_unset_time_delay_normal_reads_the_algorithms_delay() {
    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::UNSIGNED_RANGE).unwrap();
    ee.set_event_parameters(BACnetEventParameter::UnsignedRange {
        time_delay: 30,
        low_limit: 10,
        high_limit: 300,
    });
    assert_eq!(
        ee.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(30)
    );
    // buffer-ready has no time delay.
    ee.set_event_parameters(BACnetEventParameter::BufferReady {
        notification_threshold: 5,
        previous_notification_count: 0,
    });
    assert_eq!(
        ee.read_property(PropertyIdentifier::TIME_DELAY_NORMAL, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}
