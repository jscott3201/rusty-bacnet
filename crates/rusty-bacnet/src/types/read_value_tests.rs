use super::*;
use crate::types::property_value::Form;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};

const AI: ObjectType = ObjectType::ANALOG_INPUT;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;

/// [`decode_read_value`] for a value with no typed constructed form and no
/// context tag: the value it carries.
fn decode(
    object_type: ObjectType,
    property: PropertyIdentifier,
    array_index: Option<u32>,
    octets: &[u8],
) -> Result<PropertyValue, Error> {
    decode_read_value(object_type, property, array_index, octets).map(|value| {
        assert_eq!(value.element(), None, "{octets:02X?}");
        assert_ne!(value.form, Form::Tagged, "{octets:02X?}");
        value.inner
    })
}

/// Whether [`decode_read_value`] reads `octets` as their tags (#2618),
/// keeping them as they are.
fn tagged(object_type: ObjectType, property: PropertyIdentifier, octets: &[u8]) -> bool {
    let value = decode_read_value(object_type, property, None, octets).unwrap();
    assert_eq!(value.element(), None, "{octets:02X?}");
    assert_eq!(
        value.inner,
        PropertyValue::ApplicationData(octets.to_vec()),
        "{octets:02X?}"
    );
    value.form == Form::Tagged
}

fn oid(instance: u32) -> PropertyValue {
    PropertyValue::ObjectIdentifier(ObjectIdentifier::new(AI, instance).unwrap())
}

fn object_list(instances: &[u32]) -> Vec<u8> {
    instances
        .iter()
        .flat_map(|&instance| {
            let mut element = vec![0xC4];
            element.extend_from_slice(&((AI.to_raw() << 22) | instance).to_be_bytes());
            element
        })
        .collect()
}

#[test]
fn one_application_element_is_the_bare_value() {
    assert_eq!(
        decode(AI, PV, None, &[0x44, 0x41, 0xAC, 0x00, 0x00]).unwrap(),
        PropertyValue::Real(21.5)
    );
}

#[test]
fn several_application_elements_are_a_list_in_wire_order() {
    // A BACnetDateTime on a scalar property: a Date, then a Time.
    let date_time = [0xA4, 126, 10, 3, 6, 0xB4, 12, 30, 0, 0];
    assert_eq!(
        decode(
            ObjectType::LOAD_CONTROL,
            PropertyIdentifier::START_TIME,
            None,
            &date_time
        )
        .unwrap(),
        PropertyValue::List(vec![
            PropertyValue::Date(Date {
                year: 126,
                month: 10,
                day: 3,
                day_of_week: 6
            }),
            PropertyValue::Time(Time {
                hour: 12,
                minute: 30,
                second: 0,
                hundredths: 0
            }),
        ])
    );
}

#[test]
fn a_whole_array_or_list_is_a_list_at_every_length() {
    let device = ObjectType::DEVICE;
    let list = PropertyIdentifier::OBJECT_LIST;
    assert_eq!(
        decode(device, list, None, &object_list(&[1, 2, 3])).unwrap(),
        PropertyValue::List(vec![oid(1), oid(2), oid(3)])
    );
    assert_eq!(
        decode(device, list, None, &object_list(&[7])).unwrap(),
        PropertyValue::List(vec![oid(7)])
    );
    assert_eq!(
        decode(device, list, None, &[]).unwrap(),
        PropertyValue::List(vec![])
    );
    // A BACnetLIST of application values: a multi-state object's Alarm_Values.
    assert_eq!(
        decode(
            ObjectType::MULTI_STATE_INPUT,
            PropertyIdentifier::ALARM_VALUES,
            None,
            &[0x21, 0x02]
        )
        .unwrap(),
        PropertyValue::List(vec![PropertyValue::Unsigned(2)])
    );
    // One element of an array is that element, and index 0 is the count.
    assert_eq!(
        decode(device, list, Some(1), &object_list(&[7])).unwrap(),
        oid(7)
    );
    assert_eq!(
        decode(device, list, Some(0), &[0x21, 0x01]).unwrap(),
        PropertyValue::Unsigned(1)
    );
    // Present_Value is a list only on a Group.
    assert_eq!(
        decode(AI, PV, None, &[0x44, 0x41, 0xAC, 0x00, 0x00]).unwrap(),
        PropertyValue::Real(21.5)
    );
}

#[test]
fn context_tagged_content_without_a_typed_form_reads_as_its_tags() {
    // A property with no typed form: a Lighting Output's Lighting_Command,
    // a context-tagged SEQUENCE (operation [0]). Load Control's shed levels,
    // the example here before, read typed since #1499.
    assert!(tagged(
        ObjectType::LIGHTING_OUTPUT,
        PropertyIdentifier::LIGHTING_COMMAND,
        &[0x09, 0x02]
    ));
    // Elements a typed read would split, read from a property on an object
    // type with no typed form for it: two BACnetPortPermission elements, a
    // BACnetDestination (application fields around a context-tagged
    // recipient) and a Group-style result (object [0], results [1]).
    let vendor = ObjectType::from_raw(200);
    let port_filter = [0x09, 0x01, 0x19, 0x01, 0x09, 0x02, 0x19, 0x00];
    let mut destination = vec![0x82, 0x01, 0xFE, 0xB4, 0, 0, 0, 0, 0xB4, 23, 59, 59, 99];
    destination.extend_from_slice(&[0x0C, 0x02, 0x00, 0x00, 0x09, 0x21, 0x01, 0x10]);
    destination.extend_from_slice(&[0x82, 0x05, 0xE0]);
    let group = [0x0C, 0, 0, 0, 1, 0x1E, 0x29, 0x55, 0x4E, 0x10, 0x4F, 0x1F];
    for (object_type, property, octets) in [
        (vendor, PropertyIdentifier::PORT_FILTER, &port_filter[..]),
        (
            ObjectType::NOTIFICATION_CLASS,
            PropertyIdentifier::PORT_FILTER,
            &port_filter[..],
        ),
        (vendor, PropertyIdentifier::RECIPIENT_LIST, &destination[..]),
        (AI, PV, &group[..]),
    ] {
        assert!(tagged(object_type, property, octets), "{octets:02X?}");
    }
    // A vendor property: context fields around application values, nested.
    let fields = [
        0x09, 0x02, 0x1E, 0x44, 0x3F, 0xC0, 0, 0, 0x2E, 0x11, 0x2F, 0x1F, 0x21, 7,
    ];
    assert!(tagged(vendor, PropertyIdentifier::from_raw(1000), &fields));
}

#[test]
fn malformed_octets_are_an_error_wherever_they_sit() {
    // A REAL cut short.
    assert!(decode(AI, PV, None, &[0x44, 0x41]).is_err());
    // A valid first element followed by a truncated one.
    assert!(decode(AI, PV, None, &[0x21, 0x01, 0x44]).is_err());
    // An opening context tag with no closing tag, after an application value.
    assert!(decode(AI, PV, None, &[0x21, 0x01, 0x1E, 0x21, 0x01]).is_err());
}

#[test]
fn well_framed_content_the_model_cannot_hold_comes_back_as_octets() {
    // Each element is well framed, but PropertyValue has no form for it.
    let unrepresentable: [&[u8]; 6] = [
        &[0x75, 0x05, 0x03, 0, 0, 0, 0x41], // UCS-4 CharacterString "A"
        &[0x74, 0x01, 0x03, 0xB5, 0x41],    // DBCS, code page 949
        &[0x73, 0x02, 0x30, 0x21],          // JIS X 0208
        &[0x73, 0x00, 0xFF, 0xFE],          // UTF-8 that isn't
        &[0x95, 0x05, 0x01, 0, 0, 0, 0],    // ENUMERATED past u32
        &[0x01, 0x00],                      // NULL with a content octet
    ];
    let object_list = PropertyIdentifier::OBJECT_LIST;
    for element in unrepresentable {
        // Alone, after an application value, after a context element, and
        // inside one: the tree would hold a value the model can't.
        let framed = [&[0x0E][..], element, &[0x0F]].concat();
        for octets in [
            element.to_vec(),
            [&[0x21, 0x01][..], element].concat(),
            [&[0x09, 0x01][..], element].concat(),
            framed,
        ] {
            for (object_type, property) in [(AI, PV), (ObjectType::DEVICE, object_list)] {
                let value = decode_read_value(object_type, property, None, &octets).unwrap();
                assert_eq!(
                    (value.inner, value.form),
                    (PropertyValue::ApplicationData(octets.clone()), Form::Plain),
                    "{octets:02X?}"
                );
            }
        }
    }
}

#[test]
fn a_tree_past_the_decoders_bound_keeps_its_octets() {
    // One context element, then MAX_TAG_TREE_NODES NULLs: one past the bound.
    let mut octets = vec![0x09, 0x01];
    octets.extend(std::iter::repeat_n(
        0x00,
        bacnet_encoding::tags::tree::MAX_TAG_TREE_NODES,
    ));
    let value = decode_read_value(AI, PV, None, &octets).unwrap();
    assert_eq!(value.form, Form::Plain);
    assert_eq!(value.inner, PropertyValue::ApplicationData(octets.clone()));
    // At the bound, it reads as its tags.
    octets.pop();
    assert!(tagged(AI, PV, &octets));
}

#[test]
fn only_broken_framing_is_an_error() {
    for octets in [
        &[0x9D, 0x10][..],                   // length past the end
        &[0x09, 0x01, 0x1F][..],             // closing tag with no opening
        &[0x0E, 0x21, 0x01, 0x2F][..],       // closing tag of another number
        &[0x09, 0x01, 0x0E, 0x75, 0x09][..], // value cut short inside a frame
        &[0x0E][..],                         // opening tag alone
    ] {
        assert!(decode(AI, PV, None, octets).is_err(), "{octets:02X?}");
    }
}

/// Every application datatype, its octets and its value (#2618 pins these
/// before the tag-tree decoder takes over the framing walk).
pub(super) fn application_values() -> Vec<(Vec<u8>, PropertyValue)> {
    let mut object = vec![0xC4];
    object.extend_from_slice(&((AI.to_raw() << 22) | 7).to_be_bytes());
    vec![
        (vec![0x00], PropertyValue::Null),
        (vec![0x10], PropertyValue::Boolean(false)),
        (vec![0x11], PropertyValue::Boolean(true)),
        (vec![0x21, 0x48], PropertyValue::Unsigned(72)),
        (vec![0x31, 0xB8], PropertyValue::Signed(-72)),
        (vec![0x44, 0x42, 0x90, 0, 0], PropertyValue::Real(72.0)),
        (
            vec![0x55, 0x08, 0x40, 0x52, 0, 0, 0, 0, 0, 0],
            PropertyValue::Double(72.0),
        ),
        (
            vec![0x63, 0x12, 0x34, 0xFF],
            PropertyValue::OctetString(vec![0x12, 0x34, 0xFF]),
        ),
        (
            vec![0x73, 0x00, b'h', b'i'],
            PropertyValue::CharacterString("hi".into()),
        ),
        (
            vec![0x73, 0x05, b'A', 0xE9],
            PropertyValue::CharacterString("A\u{e9}".into()),
        ),
        (
            vec![0x75, 0x05, 0x04, 0x00, b'A', 0x00, b'B'],
            PropertyValue::CharacterString("AB".into()),
        ),
        (
            vec![0x82, 0x05, 0xE0],
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0xE0],
            },
        ),
        (vec![0x91, 0x03], PropertyValue::Enumerated(3)),
        (
            vec![0xA4, 126, 10, 3, 6],
            PropertyValue::Date(Date {
                year: 126,
                month: 10,
                day: 3,
                day_of_week: 6,
            }),
        ),
        (
            vec![0xB4, 12, 30, 0, 0],
            PropertyValue::Time(Time {
                hour: 12,
                minute: 30,
                second: 0,
                hundredths: 0,
            }),
        ),
        (object, oid(7)),
    ]
}

#[test]
fn application_values_keep_their_typed_form() {
    let device = ObjectType::DEVICE;
    let object_list = PropertyIdentifier::OBJECT_LIST;
    for (octets, value) in application_values() {
        // Alone: the bare value.
        assert_eq!(
            decode(AI, PV, None, &octets).unwrap(),
            value,
            "{octets:02X?}"
        );
        // After another value: a list in wire order.
        let pair = [&[0x21, 0x01][..], &octets].concat();
        assert_eq!(
            decode(AI, PV, None, &pair).unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(1), value.clone()]),
            "{pair:02X?}"
        );
        // A whole array: a list at every length.
        assert_eq!(
            decode(device, object_list, None, &octets).unwrap(),
            PropertyValue::List(vec![value.clone()]),
            "{octets:02X?}"
        );
        // An indexed element: the bare value.
        assert_eq!(
            decode(device, object_list, Some(3), &octets).unwrap(),
            value,
            "{octets:02X?}"
        );
    }
}

#[test]
fn a_date_time_priority_array_splits_into_its_slots() {
    // A DateTime Value's Priority_Array (#2098): slot 2 is the date-time
    // choice under context tag 1, the rest NULL.
    let slot = [0x1E, 0xA4, 126, 10, 3, 6, 0xB4, 12, 30, 0, 0, 0x1F];
    let mut octets = vec![0x00];
    octets.extend_from_slice(&slot);
    octets.extend(std::iter::repeat_n(0x00, 14));
    let value = decode(
        ObjectType::DATETIME_VALUE,
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        &octets,
    )
    .unwrap();
    let PropertyValue::List(slots) = value else {
        panic!("a whole Priority_Array is a list: {value:?}");
    };
    assert_eq!(slots.len(), 16);
    assert_eq!(slots[0], PropertyValue::Null);
    assert_eq!(slots[1], PropertyValue::ApplicationData(slot.to_vec()));
    assert_eq!(slots[15], PropertyValue::Null);
}
