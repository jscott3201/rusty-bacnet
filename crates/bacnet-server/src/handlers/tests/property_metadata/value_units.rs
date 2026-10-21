//! Units on Integer, Positive Integer and Large Analog Value over the wire
//! (#1092): Tables 12-50, 12-51 and 12-46 code it R, so RPM REQUIRED and ALL
//! carry it, ReadProperty answers with a BACnetEngineeringUnits enumeration,
//! WriteProperty is refused, and the PICS lists it required and read-only.

use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::value_types::{
    IntegerValueObject, LargeAnalogValueObject, PositiveIntegerValueObject,
};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::PropertyValue;
use PropertyIdentifier as P;

/// The three numeric value objects, Units set to degrees Celsius when
/// `configured`, otherwise left at NO_UNITS.
fn numeric_values(configured: bool) -> [Box<dyn BACnetObject>; 3] {
    let mut iv = IntegerValueObject::new(7, "IV-7").unwrap();
    let mut piv = PositiveIntegerValueObject::new(7, "PIV-7").unwrap();
    let mut lav = LargeAnalogValueObject::new(7, "LAV-7").unwrap();
    if configured {
        iv.set_units(EngineeringUnits::DEGREES_CELSIUS).unwrap();
        piv.set_units(EngineeringUnits::DEGREES_CELSIUS).unwrap();
        lav.set_units(EngineeringUnits::DEGREES_CELSIUS).unwrap();
    }
    [Box::new(iv), Box::new(piv), Box::new(lav)]
}

const ALL: [P; 13] = [
    P::OBJECT_IDENTIFIER,
    P::OBJECT_NAME,
    P::DESCRIPTION,
    P::OBJECT_TYPE,
    P::PRESENT_VALUE,
    P::STATUS_FLAGS,
    P::OUT_OF_SERVICE,
    P::RELIABILITY,
    P::UNITS,
    P::PRIORITY_ARRAY,
    P::RELINQUISH_DEFAULT,
    P::COV_INCREMENT,
    P::CURRENT_COMMAND_PRIORITY,
];
const REQUIRED: [P; 6] = [
    P::OBJECT_IDENTIFIER,
    P::OBJECT_NAME,
    P::OBJECT_TYPE,
    P::PRESENT_VALUE,
    P::STATUS_FLAGS,
    P::UNITS,
];
const OPTIONAL: [P; 7] = [
    P::DESCRIPTION,
    P::OUT_OF_SERVICE,
    P::RELIABILITY,
    P::PRIORITY_ARRAY,
    P::RELINQUISH_DEFAULT,
    P::COV_INCREMENT,
    P::CURRENT_COMMAND_PRIORITY,
];

#[test]
fn rpm_numeric_value_selectors_carry_units() {
    for configured in [false, true] {
        for object in numeric_values(configured) {
            let oid = object.object_identifier();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            for (selector, expected) in [
                (P::ALL, ALL.as_slice()),
                (P::REQUIRED, REQUIRED.as_slice()),
                (P::OPTIONAL, OPTIONAL.as_slice()),
            ] {
                assert_rpm_selector_bytes(&db, oid, selector, expected);
            }
        }
    }
}

fn read_units(db: &ObjectDatabase, oid: ObjectIdentifier) -> Vec<u8> {
    let mut request = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: P::UNITS,
        property_array_index: None,
    }
    .encode(&mut request);
    let mut response = BytesMut::new();
    handle_read_property(db, &request, &mut response).unwrap();
    ReadPropertyACK::decode(&response).unwrap().property_value
}

#[test]
fn numeric_value_units_read_over_the_wire_and_refuse_writes() {
    for configured in [false, true] {
        for object in numeric_values(configured) {
            let oid = object.object_identifier();
            let mut db = ObjectDatabase::new();
            db.add(object).unwrap();
            // NO_UNITS is 95 and DEGREES_CELSIUS 62, application Enumerated.
            let expected = if configured { [0x91, 62] } else { [0x91, 95] };
            assert_eq!(read_units(&db, oid), expected, "{oid:?}");

            let mut value = BytesMut::new();
            encode_property_value(&mut value, &PropertyValue::Enumerated(98)).unwrap();
            let mut request = BytesMut::new();
            WritePropertyRequest {
                object_identifier: oid,
                property_identifier: P::UNITS,
                property_array_index: None,
                property_value: value.to_vec(),
                priority: None,
            }
            .encode(&mut request)
            .unwrap();
            assert!(matches!(
                handle_write_property(&mut db, &request),
                Err(Error::Protocol { class, code })
                    if class == ErrorClass::PROPERTY
                        && code == ErrorCode::WRITE_ACCESS_DENIED
            ));
            assert_eq!(read_units(&db, oid), expected, "{oid:?}");
        }
    }
}

#[test]
fn pics_numeric_value_units_is_required_and_read_only() {
    use crate::pics::{generate_pics, PicsConfig};
    use crate::server::ServerConfig;

    for object in numeric_values(true) {
        let kind = object.object_identifier().object_type();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let support = &pics.supported_object_types[0];
        assert_eq!(support.object_type, kind);
        let units = support
            .supported_properties
            .iter()
            .find(|row| row.property_id == P::UNITS)
            .unwrap_or_else(|| panic!("{kind:?} lists Units"));
        assert!(units.access.readable);
        assert!(!units.access.optional, "{kind:?}");
        assert!(!units.access.writable, "{kind:?}");
    }
}
