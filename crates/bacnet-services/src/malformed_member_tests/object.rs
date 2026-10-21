//! Object and property access: ReadProperty, WriteProperty, the
//! ReadPropertyMultiple ACK, CreateObject, DeleteObject and the list
//! services. DeleteObject's identifier is application-tagged (Clause 21);
//! the others read context tags.

use super::*;
use crate::list_manipulation::ListElementRequest;
use crate::object_mgmt::{CreateObjectRequest, DeleteObjectRequest};
use crate::read_property::{ReadPropertyACK, ReadPropertyRequest};
use crate::rpm::ReadPropertyMultipleACK;
use crate::write_property::WritePropertyRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode};

/// AV-1 as an application object identifier.
const AV_1_APP: &[u8] = &[0xC4, 0x00, 0x80, 0x00, 0x01];
/// AV-1 as a `[0]` object identifier.
const AV_1: &[u8] = &[0x0C, 0x00, 0x80, 0x00, 0x01];
/// Present_Value as a `[1]` property identifier.
const PV: &[u8] = &[0x19, 0x55];
/// REAL 72.0 inside an opening and closing `[3]`.
const VALUE_72: &[u8] = &[0x3E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x3F];

#[test]
fn read_property_request() {
    let rows: &[Row<'_>] = &[
        ("well formed", &cat(&[AV_1, PV]), Decodes),
        (
            "with an array index",
            &cat(&[AV_1, PV, &[0x29, 0x02]]),
            Decodes,
        ),
        (
            "object identifier as an application tag",
            &cat(&[AV_1_APP, PV]),
            Malformed,
        ),
        (
            "object identifier as [1]",
            &[0x1C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x55],
            Malformed,
        ),
        ("object identifier cut short", &[0x0C, 0x00, 0x80], Short),
        (
            "object identifier of five octets, cut short",
            &[0x0D, 0x05, 0x00, 0x80, 0x00],
            Malformed,
        ),
        (
            "property identifier as an application tag",
            &cat(&[AV_1, &[0x21, 0x55]]),
            Malformed,
        ),
        (
            "property identifier cut short",
            &cat(&[AV_1, &[0x1A, 0x55]]),
            Short,
        ),
        (
            "array index as [3]",
            &cat(&[AV_1, PV, &[0x39, 0x02]]),
            Malformed,
        ),
        (
            "array index cut short",
            &cat(&[AV_1, PV, &[0x2A, 0x01]]),
            Short,
        ),
        (
            "an octet after the property",
            &cat(&[AV_1, PV, &[0x00]]),
            Malformed,
        ),
        (
            "an octet after the array index",
            &cat(&[AV_1, PV, &[0x29, 0x02, 0x00]]),
            Malformed,
        ),
    ];
    check(decoder!(ReadPropertyRequest), rows);
}

#[test]
fn read_property_ack() {
    let rows: &[Row<'_>] = &[
        ("well formed", &cat(&[AV_1, PV, VALUE_72]), Decodes),
        (
            "with an array index",
            &cat(&[AV_1, PV, &[0x29, 0x01], VALUE_72]),
            Decodes,
        ),
        (
            "object identifier as an application tag",
            &cat(&[AV_1_APP, PV, VALUE_72]),
            Malformed,
        ),
        ("object identifier cut short", &[0x0C, 0x00, 0x80], Short),
        (
            "array index cut short",
            &cat(&[AV_1, PV, &[0x2A, 0x01]]),
            Short,
        ),
        (
            "value without its [3]",
            &cat(&[AV_1, PV, &[0x44, 0x42, 0x90, 0x00, 0x00]]),
            Malformed,
        ),
        ("no value", &cat(&[AV_1, PV]), Malformed),
        (
            "an octet after the value",
            &cat(&[AV_1, PV, VALUE_72, &[0x00]]),
            Malformed,
        ),
    ];
    check(decoder!(ReadPropertyACK), rows);
}

#[test]
fn write_property_request() {
    let out_of_range = Kind::Protocol(ErrorClass::SERVICES, ErrorCode::PARAMETER_OUT_OF_RANGE);
    let rows: &[Row<'_>] = &[
        ("well formed", &cat(&[AV_1, PV, VALUE_72]), Decodes),
        (
            "with priority 8",
            &cat(&[AV_1, PV, VALUE_72, &[0x49, 0x08]]),
            Decodes,
        ),
        (
            "object identifier as [1]",
            &[0x1C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x55],
            Malformed,
        ),
        ("object identifier cut short", &[0x0C, 0x00, 0x80], Short),
        (
            "array index cut short",
            &cat(&[AV_1, PV, &[0x2A, 0x01]]),
            Short,
        ),
        (
            "value without its [3]",
            &cat(&[AV_1, PV, &[0x44, 0x42, 0x90, 0x00, 0x00]]),
            Malformed,
        ),
        (
            "priority as an application tag",
            &cat(&[AV_1, PV, VALUE_72, &[0x21, 0x08]]),
            Malformed,
        ),
        (
            "priority cut short",
            &cat(&[AV_1, PV, VALUE_72, &[0x4A, 0x00]]),
            Short,
        ),
        (
            "priority 17",
            &cat(&[AV_1, PV, VALUE_72, &[0x49, 0x11]]),
            out_of_range,
        ),
        // Read at full width, so 300 is out of range rather than too wide.
        (
            "priority 300",
            &cat(&[AV_1, PV, VALUE_72, &[0x4A, 0x01, 0x2C]]),
            out_of_range,
        ),
        (
            "an octet after the priority",
            &cat(&[AV_1, PV, VALUE_72, &[0x49, 0x08, 0x00]]),
            Malformed,
        ),
    ];
    check(decoder!(WritePropertyRequest), rows);
}

#[test]
fn read_property_multiple_ack() {
    let result = |element: &[u8]| cat(&[AV_1, &[0x1E, 0x29, 0x55], element, &[0x1F]]);
    // PROPERTY / UNKNOWN_PROPERTY inside [5].
    let error = [0x5E, 0x91, 0x02, 0x91, 0x20, 0x5F];
    let rows: &[Row<'_>] = &[
        (
            "a value",
            &result(&[0x4E, 0x44, 0x42, 0x90, 0x00, 0x00, 0x4F]),
            Decodes,
        ),
        ("an error", &result(&error), Decodes),
        (
            "object identifier as an application tag",
            &cat(&[AV_1_APP, &[0x1E, 0x1F]]),
            Malformed,
        ),
        ("object identifier cut short", &[0x0C, 0x00, 0x80], Short),
        (
            "property identifier cut short",
            &cat(&[AV_1, &[0x1E, 0x2A, 0x55]]),
            Short,
        ),
        (
            "array index cut short",
            &cat(&[AV_1, &[0x1E, 0x29, 0x55, 0x3A, 0x01]]),
            Short,
        ),
        (
            "error class as an Unsigned",
            &result(&[0x5E, 0x21, 0x02, 0x91, 0x20, 0x5F]),
            Malformed,
        ),
        (
            "error class cut short",
            &cat(&[AV_1, &[0x1E, 0x29, 0x55, 0x5E, 0x92, 0x02]]),
            Short,
        ),
        (
            "an octet after the results",
            &cat(&[result(&error).as_slice(), &[0x00]]),
            Malformed,
        ),
    ];
    check(decoder!(ReadPropertyMultipleACK), rows);
}

#[test]
fn create_object_request() {
    let rows: &[Row<'_>] = &[
        ("by type", &[0x0E, 0x09, 0x02, 0x0F], Decodes),
        (
            "by identifier",
            &[0x0E, 0x1C, 0x00, 0x80, 0x00, 0x05, 0x0F],
            Decodes,
        ),
        (
            "type as an application tag",
            &[0x0E, 0x21, 0x02, 0x0F],
            Malformed,
        ),
        ("specifier [2]", &[0x0E, 0x29, 0x02, 0x0F], Malformed),
        ("type cut short", &[0x0E, 0x0A, 0x02], Short),
        ("identifier cut short", &[0x0E, 0x1C, 0x00, 0x80], Short),
        (
            "identifier of three octets",
            &[0x0E, 0x1B, 0x00, 0x80, 0x00, 0x0F],
            Malformed,
        ),
        ("specifier not closed", &[0x0E, 0x09, 0x02], Malformed),
        (
            "an octet after the specifier",
            &[0x0E, 0x09, 0x02, 0x0F, 0x00],
            Malformed,
        ),
    ];
    check(decoder!(CreateObjectRequest), rows);
}

#[test]
fn delete_object_request() {
    let rows: &[Row<'_>] = &[
        ("well formed", AV_1_APP, Decodes),
        ("identifier as context [0]", AV_1, Malformed),
        (
            "identifier as an Unsigned",
            &[0x24, 0x00, 0x80, 0x00, 0x01],
            Malformed,
        ),
        ("identifier cut short", &[0xC4, 0x00, 0x80], Short),
        (
            "identifier of three octets",
            &[0xC3, 0x00, 0x80, 0x00],
            Malformed,
        ),
        (
            "identifier of five octets, cut short",
            &[0xC5, 0x05, 0x00, 0x80],
            Malformed,
        ),
        ("empty", &[], Malformed),
        (
            "an octet after the identifier",
            &cat(&[AV_1_APP, &[0x00]]),
            Malformed,
        ),
        (
            "a second identifier",
            &cat(&[AV_1_APP, &[0xC4, 0x00, 0x80, 0x00, 0x02]]),
            Malformed,
        ),
    ];
    check(decoder!(DeleteObjectRequest), rows);
}

#[test]
fn list_element_request() {
    let elements: &[u8] = &[0x3E, 0x21, 0x01, 0x3F];
    let rows: &[Row<'_>] = &[
        ("well formed", &cat(&[AV_1, PV, elements]), Decodes),
        (
            "with an array index",
            &cat(&[AV_1, PV, &[0x29, 0x02], elements]),
            Decodes,
        ),
        ("object identifier cut short", &[0x0C, 0x00, 0x80], Short),
        (
            "object identifier of five octets, cut short",
            &[0x0D, 0x05, 0x00, 0x80, 0x00],
            Malformed,
        ),
        (
            "property identifier as an application tag",
            &cat(&[AV_1, &[0x21, 0x55], elements]),
            Malformed,
        ),
        (
            "property identifier cut short",
            &cat(&[AV_1, &[0x1A, 0x55]]),
            Short,
        ),
        (
            "array index cut short",
            &cat(&[AV_1, PV, &[0x2A, 0x01]]),
            Short,
        ),
        (
            "elements without their [3]",
            &cat(&[AV_1, PV, &[0x21, 0x01]]),
            Malformed,
        ),
        (
            "an octet after the elements",
            &cat(&[AV_1, PV, elements, &[0x00]]),
            Malformed,
        ),
    ];
    check(decoder!(ListElementRequest), rows);
}
