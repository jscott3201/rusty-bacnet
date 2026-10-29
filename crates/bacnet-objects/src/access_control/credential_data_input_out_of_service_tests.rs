//! Credential Data Input Present_Value and Reliability writes while
//! Out_Of_Service is TRUE (Clauses 12.36.4, 12.36.7 and 12.36.8, Table 12-43
//! footnote 1, #1168).

use bacnet_types::error::RemoteError;
use std::sync::Arc;

use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat};
use bacnet_types::enums::{AuthenticationFactorType as F, ErrorClass, ErrorCode};
use bacnet_types::enums::{PropertyIdentifier as P, Reliability};

use super::*;
use crate::clock::{ClockFrame, ClockReader};

/// A clock that always reads 2026-10-02 (a Friday) at `hour`:30.
pub(super) struct FixedClock(pub(super) u8);

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 10,
                day: 2,
                day_of_week: 5,
            },
            local_time: Time {
                hour: self.0,
                minute: 30,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

/// Update_Time as the datetime choice the clock above stamps at `hour`.
pub(super) fn stamped(hour: u8) -> PropertyValue {
    PropertyValue::ApplicationData(vec![0x2E, 0xA4, 126, 10, 2, 5, 0xB4, hour, 30, 0, 0, 0x2F])
}

/// An update time in the sequence-number choice, context tag [1].
pub(super) fn sequence(number: u16) -> PropertyValue {
    let [high, low] = number.to_be_bytes();
    PropertyValue::ApplicationData(if high == 0 {
        vec![0x19, low]
    } else {
        vec![0x1A, high, low]
    })
}

/// A reader of Wiegand 26 cards (class 0) and of vendor 260's format 7
/// (class 3), whose last read was a Wiegand 26 card at 09:30.
pub(super) fn reader() -> CredentialDataInputObject {
    let mut reader = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    reader
        .set_supported_formats([
            (BACnetAuthenticationFactorFormat::standard(F::WIEGAND26), 0),
            (BACnetAuthenticationFactorFormat::custom(260, 7), 3),
        ])
        .unwrap();
    reader
        .set_present_value(card(&[0x12, 0x34, 0x56]), stamp(9))
        .unwrap();
    reader.bind_clock_internal(Some(Arc::new(FixedClock(11))));
    reader
}

pub(super) fn card(value: &[u8]) -> BACnetAuthenticationFactor {
    BACnetAuthenticationFactor {
        format_type: F::WIEGAND26,
        format_class: 0,
        value: value.to_vec(),
    }
}

/// `hour`:30 on 2026-10-02.
pub(super) fn stamp(hour: u8) -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: Date {
            year: 126,
            month: 10,
            day: 2,
            day_of_week: 5,
        },
        time: Time {
            hour,
            minute: 30,
            second: 0,
            hundredths: 0,
        },
    }
}

/// A factor's bytes: format type `[0]`, format class `[1]`, value `[2]`.
pub(super) fn factor(format_type: u8, class: u8, value: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x09, format_type, 0x19, class, 0x28 | value.len() as u8];
    bytes.extend_from_slice(value);
    bytes
}

pub(super) fn data(bytes: Vec<u8>) -> PropertyValue {
    PropertyValue::ApplicationData(bytes)
}

pub(super) fn read(reader: &CredentialDataInputObject, property: P) -> PropertyValue {
    reader.read_property(property, None).unwrap()
}

pub(super) fn write(
    reader: &mut CredentialDataInputObject,
    property: P,
    value: PropertyValue,
) -> Result<(), Error> {
    reader.write_property(property, None, value, None)
}

pub(super) fn set_out_of_service(reader: &mut CredentialDataInputObject, out_of_service: bool) {
    write(
        reader,
        P::OUT_OF_SERVICE,
        PropertyValue::Boolean(out_of_service),
    )
    .unwrap();
}

/// Present_Value, Update_Time, Reliability and Status_Flags as served.
fn served(reader: &CredentialDataInputObject) -> [PropertyValue; 4] {
    [
        P::PRESENT_VALUE,
        P::UPDATE_TIME,
        P::RELIABILITY,
        P::STATUS_FLAGS,
    ]
    .map(|property| read(reader, property))
}

fn status_flags(fault: bool, out_of_service: bool) -> PropertyValue {
    let mut flags = StatusFlags::empty();
    flags.set(StatusFlags::FAULT, fault);
    flags.set(StatusFlags::OUT_OF_SERVICE, out_of_service);
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags.bits() << 4],
    }
}

pub(super) fn assert_property_error(result: Result<(), Error>, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code: actual }))
            if class == ErrorClass::PROPERTY
                && actual == code),
        "expected PROPERTY / {code:?}, got {result:?}"
    );
}

/// The reader's own values: the Wiegand 26 card at 09:00, no fault.
fn device(out_of_service: bool) -> [PropertyValue; 4] {
    [
        data(factor(8, 0, &[0x12, 0x34, 0x56])),
        data(vec![0x2E, 0xA4, 126, 10, 2, 5, 0xB4, 9, 30, 0, 0, 0x2F]),
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
        status_flags(false, out_of_service),
    ]
}

#[test]
fn credential_data_input_refuses_simulated_rows_in_service() {
    let mut reader = reader();
    for (property, value) in [
        (P::PRESENT_VALUE, data(factor(8, 0, &[0x01]))),
        (
            P::RELIABILITY,
            PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw()),
        ),
        // In service the refusal comes before any datatype check.
        (P::PRESENT_VALUE, PropertyValue::Real(1.0)),
        (P::RELIABILITY, PropertyValue::Null),
    ] {
        assert_property_error(
            write(&mut reader, property, value),
            ErrorCode::WRITE_ACCESS_DENIED,
        );
        assert_eq!(served(&reader), device(false), "{property:?}");
    }
    // Both rows have a write route, open only out of service.
    assert!(reader.is_writable_property(P::PRESENT_VALUE));
    assert!(reader.is_writable_property(P::RELIABILITY));
}

#[test]
fn credential_data_input_takes_simulated_rows_out_of_service() {
    let mut reader = reader();
    set_out_of_service(&mut reader, true);
    assert_eq!(served(&reader), device(true));

    // A vendor 260 factor, class 3: served, with Update_Time stamped from the
    // Device clock at 11:30.
    write(&mut reader, P::PRESENT_VALUE, data(factor(2, 3, &[0xAB]))).unwrap();
    assert_eq!(read(&reader, P::PRESENT_VALUE), data(factor(2, 3, &[0xAB])));
    assert_eq!(read(&reader, P::UPDATE_TIME), stamped(11));

    // The form the server hands over, one chunk per context tag, is the same
    // factor. The same card again is a new read and stamps again.
    reader.bind_clock_internal(Some(Arc::new(FixedClock(12))));
    for _ in 0..2 {
        write(
            &mut reader,
            P::PRESENT_VALUE,
            PropertyValue::List(vec![
                data(vec![0x09, 0x08]),
                data(vec![0x19, 0x00]),
                data(vec![0x2B, 0x12, 0x34, 0x56]),
            ]),
        )
        .unwrap();
        assert_eq!(
            read(&reader, P::PRESENT_VALUE),
            data(factor(8, 0, &[0x12, 0x34, 0x56]))
        );
        assert_eq!(read(&reader, P::UPDATE_TIME), stamped(12));
    }

    // UNDEFINED and ERROR, each with class 0, need no declared format.
    for format_type in [0, 1] {
        write(
            &mut reader,
            P::PRESENT_VALUE,
            data(factor(format_type, 0, &[])),
        )
        .unwrap();
        assert_eq!(
            read(&reader, P::PRESENT_VALUE),
            data(factor(format_type, 0, &[]))
        );
    }

    // A simulated fault sets the FAULT flag and leaves Update_Time alone; a
    // proprietary value is taken too.
    reader.bind_clock_internal(Some(Arc::new(FixedClock(13))));
    for reliability in [Reliability::UNRELIABLE_OTHER, Reliability::from_raw(64)] {
        write(
            &mut reader,
            P::RELIABILITY,
            PropertyValue::Enumerated(reliability.to_raw()),
        )
        .unwrap();
        assert_eq!(
            read(&reader, P::RELIABILITY),
            PropertyValue::Enumerated(reliability.to_raw())
        );
        assert_eq!(read(&reader, P::STATUS_FLAGS), status_flags(true, true));
        assert_eq!(read(&reader, P::UPDATE_TIME), stamped(12));
    }
}

#[test]
fn credential_data_input_without_a_clock_stamps_update_time_with_a_sequence_number() {
    let mut reader = reader();
    reader.bind_clock_internal(None);
    set_out_of_service(&mut reader, true);
    // Each simulated read takes the object's next number, from 1, so Update_Time
    // moves even for the same factor; a Reliability write takes none.
    for number in 1..=2 {
        write(&mut reader, P::PRESENT_VALUE, data(factor(8, 0, &[0x77]))).unwrap();
        assert_eq!(read(&reader, P::UPDATE_TIME), sequence(number));
    }
    write(
        &mut reader,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw()),
    )
    .unwrap();
    assert_eq!(read(&reader, P::UPDATE_TIME), sequence(2));

    // Dropping Wiegand 26 resets the factor served and then the reader's
    // factor set aside, each an update with the next number.
    reader
        .set_supported_formats([(BACnetAuthenticationFactorFormat::custom(260, 7), 3)])
        .unwrap();
    assert_eq!(read(&reader, P::UPDATE_TIME), sequence(3));
    set_out_of_service(&mut reader, false);
    assert_eq!(
        [
            read(&reader, P::PRESENT_VALUE),
            read(&reader, P::UPDATE_TIME)
        ],
        [data(factor(0, 0, &[])), sequence(4)]
    );

    // With a clock again, updates take its date and time.
    reader.bind_clock_internal(Some(Arc::new(FixedClock(12))));
    set_out_of_service(&mut reader, true);
    write(&mut reader, P::PRESENT_VALUE, data(factor(2, 3, &[0x01]))).unwrap();
    assert_eq!(read(&reader, P::UPDATE_TIME), stamped(12));
}

#[test]
fn sequence_numbers_climb_to_the_top_of_the_range_and_skip_zero() {
    assert_eq!(super::next_sequence(0), 1);
    assert_eq!(super::next_sequence(1), 2);
    assert_eq!(super::next_sequence(u16::MAX - 1), u16::MAX);
    assert_eq!(super::next_sequence(u16::MAX), 1);
}

#[test]
fn credential_data_input_simulated_rows_outside_their_datatypes_are_refused_unchanged() {
    const INVALID_DATA_TYPE: ErrorCode = ErrorCode::INVALID_DATA_TYPE;
    const VALUE_OUT_OF_RANGE: ErrorCode = ErrorCode::VALUE_OUT_OF_RANGE;
    let mut reader = reader();
    set_out_of_service(&mut reader, true);
    let mut wide_type = vec![0x0A, 0x01, 0x00];
    wide_type.extend_from_slice(&[0x19, 0x00, 0x28]);
    let mut trailing = factor(8, 0, &[0x01]);
    trailing.extend_from_slice(&[0x39, 0x00]);
    for (property, value, code) in [
        // Not one BACnetAuthenticationFactor.
        (
            P::PRESENT_VALUE,
            PropertyValue::Enumerated(8),
            INVALID_DATA_TYPE,
        ),
        (P::PRESENT_VALUE, PropertyValue::Null, INVALID_DATA_TYPE),
        (
            P::PRESENT_VALUE,
            data(vec![0x09, 0x08, 0x19, 0x00]),
            INVALID_DATA_TYPE,
        ),
        (
            P::PRESENT_VALUE,
            data(vec![0x19, 0x00, 0x09, 0x08, 0x28]),
            INVALID_DATA_TYPE,
        ),
        (P::PRESENT_VALUE, data(trailing), INVALID_DATA_TYPE),
        (
            P::PRESENT_VALUE,
            PropertyValue::List(vec![
                data(vec![0x09, 0x08]),
                PropertyValue::Unsigned(0),
                data(vec![0x28]),
            ]),
            INVALID_DATA_TYPE,
        ),
        // 25 and 256 lie past the closed production.
        (
            P::PRESENT_VALUE,
            data(factor(25, 0, &[])),
            VALUE_OUT_OF_RANGE,
        ),
        (P::PRESENT_VALUE, data(wide_type), VALUE_OUT_OF_RANGE),
        // A format the reader doesn't declare, and a declared one with
        // another class.
        (
            P::PRESENT_VALUE,
            data(factor(9, 0, &[0x01])),
            VALUE_OUT_OF_RANGE,
        ),
        (
            P::PRESENT_VALUE,
            data(factor(8, 3, &[0x01])),
            VALUE_OUT_OF_RANGE,
        ),
        (
            P::PRESENT_VALUE,
            data(factor(2, 0, &[0x01])),
            VALUE_OUT_OF_RANGE,
        ),
        // UNDEFINED and ERROR carry class 0.
        (
            P::PRESENT_VALUE,
            data(factor(0, 1, &[])),
            VALUE_OUT_OF_RANGE,
        ),
        (
            P::PRESENT_VALUE,
            data(factor(1, 2, &[])),
            VALUE_OUT_OF_RANGE,
        ),
        // 11 is reserved for ASHRAE, 65536 past the datatype.
        (
            P::RELIABILITY,
            PropertyValue::Enumerated(11),
            VALUE_OUT_OF_RANGE,
        ),
        (
            P::RELIABILITY,
            PropertyValue::Enumerated(65_536),
            VALUE_OUT_OF_RANGE,
        ),
        (
            P::RELIABILITY,
            PropertyValue::Unsigned(7),
            INVALID_DATA_TYPE,
        ),
        (P::RELIABILITY, PropertyValue::Null, INVALID_DATA_TYPE),
    ] {
        assert_property_error(write(&mut reader, property, value), code);
        assert_eq!(served(&reader), device(true), "{property:?} {code:?}");
    }
}

#[test]
fn credential_data_input_return_to_service_serves_the_reader_again() {
    let mut reader = reader();
    set_out_of_service(&mut reader, true);
    write(&mut reader, P::PRESENT_VALUE, data(factor(2, 3, &[0xAB]))).unwrap();
    write(
        &mut reader,
        P::RELIABILITY,
        PropertyValue::Enumerated(Reliability::UNRELIABLE_OTHER.to_raw()),
    )
    .unwrap();
    let simulated = served(&reader);

    // Out of service the application's reads go aside, and its Reliability
    // is refused as on the other Reliability carriers.
    reader.set_present_value(card(&[0x99]), stamp(10)).unwrap();
    assert_property_error(
        reader.set_reliability_internal(Reliability::COMMUNICATION_FAILURE),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(served(&reader), simulated);

    // The return to service serves the reader's read and the Reliability
    // from before, dropping the simulation.
    set_out_of_service(&mut reader, false);
    assert_eq!(
        served(&reader),
        [
            data(factor(8, 0, &[0x99])),
            data(vec![0x2E, 0xA4, 126, 10, 2, 5, 0xB4, 10, 30, 0, 0, 0x2F]),
            PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw()),
            status_flags(false, false),
        ]
    );

    // In service the application's values are served at once.
    reader
        .set_reliability_internal(Reliability::COMMUNICATION_FAILURE)
        .unwrap();
    assert_eq!(read(&reader, P::STATUS_FLAGS), status_flags(true, false));
    reader
        .set_reliability_internal(Reliability::NO_FAULT_DETECTED)
        .unwrap();
    reader
        .set_present_value(card(&[0x12, 0x34, 0x56]), stamp(9))
        .unwrap();
    assert_eq!(served(&reader), device(false));

    // A second period out of service starts from the reader's values.
    set_out_of_service(&mut reader, true);
    assert_eq!(served(&reader), device(true));
    set_out_of_service(&mut reader, false);
    assert_eq!(served(&reader), device(false));
}

#[test]
fn credential_data_input_null_out_of_service_write_keeps_the_simulation() {
    let mut reader = reader();
    set_out_of_service(&mut reader, true);
    write(&mut reader, P::PRESENT_VALUE, data(factor(1, 0, &[]))).unwrap();
    let simulated = served(&reader);
    write(&mut reader, P::OUT_OF_SERVICE, PropertyValue::Null).unwrap();
    assert_eq!(served(&reader), simulated);
    set_out_of_service(&mut reader, false);
    assert_eq!(served(&reader), device(false));
}

#[test]
fn credential_data_input_set_reliability_internal_checks_the_production() {
    let mut reader = reader();
    for raw in [11, 65_536] {
        let result = reader.set_reliability_internal(Reliability::from_raw(raw));
        assert_property_error(result, ErrorCode::VALUE_OUT_OF_RANGE);
    }
    assert_eq!(served(&reader), device(false));
    reader
        .set_reliability_internal(Reliability::from_raw(64))
        .unwrap();
    assert_eq!(read(&reader, P::RELIABILITY), PropertyValue::Enumerated(64));
}
