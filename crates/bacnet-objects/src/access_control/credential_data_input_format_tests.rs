//! Credential Data Input Present_Value kept in step with Supported_Formats
//! and Supported_Format_Classes (Clauses 12.36.4, 12.36.9, 12.36.10 and
//! 12.36.11, #1249).

use std::sync::Arc;

use bacnet_types::constructed::{BACnetAuthenticationFactor, BACnetAuthenticationFactorFormat};
use bacnet_types::enums::{AuthenticationFactorType as F, ErrorCode, PropertyIdentifier as P};

use super::credential_data_input_out_of_service_tests::{
    FixedClock, assert_property_error, card, data, factor, read, reader, sequence,
    set_out_of_service, stamp, stamped, write,
};
use super::*;

/// The UNDEFINED factor: no read current.
fn undefined() -> PropertyValue {
    data(factor(0, 0, &[]))
}

/// The reader's Wiegand 26 card read at 09:30.
fn wiegand_card() -> [PropertyValue; 2] {
    [data(factor(8, 0, &[0x12, 0x34, 0x56])), stamped(9)]
}

/// Present_Value and Update_Time as served.
fn served(reader: &CredentialDataInputObject) -> [PropertyValue; 2] {
    [P::PRESENT_VALUE, P::UPDATE_TIME].map(|property| read(reader, property))
}

fn wiegand26(class: u32) -> (BACnetAuthenticationFactorFormat, u32) {
    (
        BACnetAuthenticationFactorFormat::standard(F::WIEGAND26),
        class,
    )
}

fn vendor_260(class: u32) -> (BACnetAuthenticationFactorFormat, u32) {
    (BACnetAuthenticationFactorFormat::custom(260, 7), class)
}

fn read_of(format_type: F, format_class: u32) -> BACnetAuthenticationFactor {
    BACnetAuthenticationFactor {
        format_type,
        format_class,
        value: vec![0x01],
    }
}

#[test]
fn credential_data_input_set_present_value_refuses_undeclared_factors() {
    let mut reader = reader();
    for factor in [
        // Wiegand 37, which the reader doesn't declare, and Wiegand 26 and the
        // vendor format under other classes.
        read_of(F::WIEGAND37, 0),
        read_of(F::WIEGAND26, 3),
        read_of(F::CUSTOM, 0),
        // 25 lies past the closed production.
        read_of(F::from_raw(25), 0),
        // UNDEFINED and ERROR carry class 0.
        read_of(F::UNDEFINED, 1),
        read_of(F::ERROR, 2),
    ] {
        assert_property_error(
            reader.set_present_value(factor.clone(), stamp(10)),
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
        assert_eq!(served(&reader), wiegand_card(), "{factor:?}");
    }

    // Each declared format with its class is taken, and so are UNDEFINED and
    // ERROR with class 0.
    for (format_type, class) in [
        (F::CUSTOM, 3),
        (F::WIEGAND26, 0),
        (F::ERROR, 0),
        (F::UNDEFINED, 0),
    ] {
        let factor = BACnetAuthenticationFactor {
            format_type,
            format_class: class,
            value: Vec::new(),
        };
        reader.set_present_value(factor, stamp(10)).unwrap();
        assert_eq!(
            served(&reader),
            [data(factor_bytes(format_type, class)), stamped(10)]
        );
    }

    // A reader that declares no format takes only UNDEFINED and ERROR.
    let mut bare = CredentialDataInputObject::new(2, "CDI-2").unwrap();
    assert_property_error(
        bare.set_present_value(card(&[0x01]), stamp(10)),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    bare.set_present_value(read_of(F::ERROR, 0), stamp(10))
        .unwrap();
}

/// A factor with no value octets.
fn factor_bytes(format_type: F, class: u32) -> Vec<u8> {
    factor(format_type.to_raw() as u8, class as u8, &[])
}

#[test]
fn credential_data_input_dropping_the_read_format_resets_present_value() {
    let mut reader = reader();
    // Declaring the same pairs again, or adding one, keeps the read.
    reader
        .set_supported_formats([wiegand26(0), vendor_260(3)])
        .unwrap();
    reader
        .set_supported_formats([
            wiegand26(0),
            vendor_260(3),
            (BACnetAuthenticationFactorFormat::standard(F::WIEGAND37), 0),
        ])
        .unwrap();
    assert_eq!(served(&reader), wiegand_card());

    // Dropping Wiegand 26 puts Present_Value back to UNDEFINED, stamped from
    // the Device clock as any other update.
    reader.set_supported_formats([vendor_260(3)]).unwrap();
    assert_eq!(served(&reader), [undefined(), stamped(11)]);

    // UNDEFINED stays through any later list, and keeps its time.
    reader.bind_clock_internal(Some(Arc::new(FixedClock(12))));
    reader.set_supported_formats([]).unwrap();
    assert_eq!(served(&reader), [undefined(), stamped(11)]);
}

#[test]
fn credential_data_input_moving_the_read_format_to_another_class_resets_present_value() {
    let mut moved = reader();
    moved
        .set_supported_formats([wiegand26(5), vendor_260(3)])
        .unwrap();
    assert_eq!(served(&moved), [undefined(), stamped(11)]);

    // Without a Device clock the reset takes the object's first sequence
    // number.
    let mut unclocked = reader();
    unclocked.bind_clock_internal(None);
    unclocked.set_supported_formats([vendor_260(3)]).unwrap();
    assert_eq!(served(&unclocked), [undefined(), sequence(1)]);
}

#[test]
fn credential_data_input_refused_format_list_keeps_present_value() {
    let mut reader = reader();
    // A CUSTOM format without its vendor members is ill-formed.
    let bare_custom = BACnetAuthenticationFactorFormat::standard(F::CUSTOM);
    assert_property_error(
        reader.set_supported_formats([(bare_custom, 0)]),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
    assert_eq!(served(&reader), wiegand_card());
}

#[test]
fn credential_data_input_dropping_a_format_out_of_service_covers_both_factors() {
    let mut reader = reader();
    set_out_of_service(&mut reader, true);
    // A simulated vendor 260 read at 11:30; the reader's Wiegand 26 read at
    // 09:30 sits aside.
    write(&mut reader, P::PRESENT_VALUE, data(factor(2, 3, &[0xAB]))).unwrap();
    let simulated = [data(factor(2, 3, &[0xAB])), stamped(11)];

    // Dropping Wiegand 26 leaves the simulation served, still declared, and
    // resets the factor set aside.
    reader.bind_clock_internal(Some(Arc::new(FixedClock(12))));
    reader.set_supported_formats([vendor_260(3)]).unwrap();
    assert_eq!(served(&reader), simulated);
    set_out_of_service(&mut reader, false);
    assert_eq!(served(&reader), [undefined(), stamped(12)]);

    // Dropping the simulated format resets the factor served.
    set_out_of_service(&mut reader, true);
    write(&mut reader, P::PRESENT_VALUE, data(factor(2, 3, &[0xCD]))).unwrap();
    reader.bind_clock_internal(Some(Arc::new(FixedClock(13))));
    reader.set_supported_formats([wiegand26(0)]).unwrap();
    assert_eq!(served(&reader), [undefined(), stamped(13)]);
    // A write of the dropped format is refused now.
    assert_property_error(
        write(&mut reader, P::PRESENT_VALUE, data(factor(2, 3, &[0xCD]))),
        ErrorCode::VALUE_OUT_OF_RANGE,
    );
}
