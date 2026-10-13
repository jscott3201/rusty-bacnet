//! Value_Source on a Color object and a noncommandable Analog Value that
//! track it (#1552), over WriteProperty, WritePropertyMultiple and
//! ReadProperty, the replies as the handlers encode them.
//!
//! Device 30 (`02 00 00 1E`) writes Present_Value, which publishes
//! `1E 1C 02 00 00 1E 1F`: `[1]` around a device-object reference with only
//! its object identifier `[1]`. A writer the server can't tie to a Device
//! publishes its address, `[2]` around network 0 and its MAC as an
//! application OctetString. Value_Source is property 433 (`92 01 B1` as an
//! Enumerated).

use super::lighting_required_rows::{assert_refused, db_with, read_wire};
use super::*;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::color::ColorObject;
use bacnet_objects::command_source::{CommandDeviceBinding, CommandOrigin};
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::MacAddr;

const VS: PropertyIdentifier = PropertyIdentifier::VALUE_SOURCE;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const DEVICE_30: [u8; 7] = [0x1E, 0x1C, 0x02, 0x00, 0x00, 0x1E, 0x1F];
/// A forwarded source naming Device 77 (`02 00 00 4D`).
const DEVICE_77: [u8; 7] = [0x1E, 0x1C, 0x02, 0x00, 0x00, 0x4D, 0x1F];
const MAC: [u8; 6] = [10, 0, 0, 5, 0xBA, 0xC0];

fn writer(device: Option<u32>) -> CommandOrigin {
    CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&MAC),
        },
        binding: device.map_or(CommandDeviceBinding::Unknown, |instance| {
            CommandDeviceBinding::Unique(
                ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
            )
        }),
    }
}

fn other_writer() -> CommandOrigin {
    CommandOrigin::Remote {
        actual_address: BACnetAddress {
            network_number: 0,
            mac_address: MacAddr::from_slice(&[10, 0, 0, 6, 0xBA, 0xC0]),
        },
        binding: CommandDeviceBinding::Unique(
            ObjectIdentifier::new(ObjectType::DEVICE, 31).unwrap(),
        ),
    }
}

/// A WriteProperty of `octets` to `property` from `origin`.
fn write_from(
    db: &mut ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    octets: &[u8],
    origin: &CommandOrigin,
) -> Result<(), Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        property_value: octets.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property_observed(db, &request, None, None, Some(origin)).map(|_| ())
}

fn tracked_color() -> (ObjectDatabase, ObjectIdentifier) {
    let mut color = ColorObject::new(1, "CLR-1").unwrap();
    color.set_value_source_tracking(true);
    db_with(Box::new(color))
}

/// An xy colour (0.25, 0.5) as two application REALs.
const XY: [u8; 10] = [0x44, 0x3E, 0x80, 0x00, 0x00, 0x44, 0x3F, 0x00, 0x00, 0x00];

#[test]
fn a_colour_write_publishes_its_writer_who_alone_corrects_it() {
    let (mut db, oid) = tracked_color();
    assert_eq!(read_wire(&db, oid, VS), [0x08]);
    // Property_List ends with Value_Source, after Transition (385, 01 81).
    let list = read_wire(&db, oid, PropertyIdentifier::PROPERTY_LIST);
    assert!(
        list.ends_with(&[0x92, 0x01, 0x81, 0x92, 0x01, 0xB1]),
        "{list:02X?}"
    );

    write_from(&mut db, oid, PV, &XY, &writer(Some(30))).unwrap();
    assert_eq!(read_wire(&db, oid, VS), DEVICE_30);
    // Another device may not correct it (Clause 19.5.1.3).
    assert_refused(
        write_from(&mut db, oid, VS, &DEVICE_77, &other_writer()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
    assert_eq!(read_wire(&db, oid, VS), DEVICE_30);
    // The writer may, and keeps the right to.
    write_from(&mut db, oid, VS, &DEVICE_77, &writer(Some(30))).unwrap();
    assert_eq!(read_wire(&db, oid, VS), DEVICE_77);
    write_from(&mut db, oid, VS, &[0x08], &writer(Some(30))).unwrap();
    assert_eq!(read_wire(&db, oid, VS), [0x08]);
    // A Color_Command FADE_TO_COLOR sets Present_Value, so its writer is
    // the source now.
    let fade = [&[0x09, 0x01, 0x1E][..], &XY, &[0x1F]].concat();
    write_from(
        &mut db,
        oid,
        PropertyIdentifier::COLOR_COMMAND,
        &fade,
        &other_writer(),
    )
    .unwrap();
    assert_eq!(
        read_wire(&db, oid, VS),
        [0x1E, 0x1C, 0x02, 0x00, 0x00, 0x1F, 0x1F]
    );
    // A context-free handler names no writer, so a tracked object refuses it.
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: oid,
        property_identifier: PV,
        property_array_index: None,
        property_value: XY.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    assert_refused(
        handle_write_property(&mut db, &request).map(|_| ()),
        ErrorCode::WRITE_ACCESS_DENIED,
    );
}

#[test]
fn an_unbound_writer_publishes_its_address_and_corrects_in_the_same_wpm() {
    let mut value =
        AnalogValueObject::with_access(1, "AV-1", 62, PresentValueAccess::Writable).unwrap();
    value.set_value_source_tracking(true);
    let (mut db, oid) = db_with(Box::new(value));
    // REAL 42.0 is 42 28 00 00.
    let real = [0x44, 0x42, 0x28, 0x00, 0x00];
    write_from(&mut db, oid, PV, &real, &writer(None)).unwrap();
    let address = [&[0x2E, 0x21, 0x00, 0x65, 0x06][..], &MAC, &[0x2F]].concat();
    assert_eq!(read_wire(&db, oid, VS), address);

    // Clause 19.5.1.3 lets the writer correct the source in the
    // WritePropertyMultiple that writes the value, after it.
    let mut request = BytesMut::new();
    let property = |property_identifier, value: &[u8]| BACnetPropertyValue {
        property_identifier,
        property_array_index: None,
        value: value.to_vec(),
        priority: None,
    };
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: oid,
            list_of_properties: vec![property(PV, &real), property(VS, &DEVICE_77)],
        }],
    }
    .encode(&mut request)
    .unwrap();
    let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
    let outcome = handle_write_property_multiple_observed(
        &mut db,
        &request,
        &mut snapshots,
        None,
        None,
        None,
        Some(&writer(Some(30))),
    );
    match outcome {
        WritePropertyMultipleOutcome::Success { .. } => {}
        WritePropertyMultipleOutcome::Error { error, .. } => panic!("{error:?}"),
        WritePropertyMultipleOutcome::Reject { reason } => panic!("{reason:?}"),
    }
    assert_eq!(read_wire(&db, oid, VS), DEVICE_77);
}
