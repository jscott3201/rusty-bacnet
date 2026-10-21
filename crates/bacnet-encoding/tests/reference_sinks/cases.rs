use bacnet_encoding::constructed::*;
use bacnet_encoding::sink::{EncodeSink, SinkFull};
use bacnet_types::constructed::{
    BACnetDeviceObjectPropertyReference, BACnetDeviceObjectReference,
    BACnetObjectPropertyReference, PropertyReference,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;

pub enum Reference {
    Property(PropertyReference),
    ObjectProperty(BACnetObjectPropertyReference),
    Setpoint(BACnetObjectPropertyReference),
    DeviceProperty(BACnetDeviceObjectPropertyReference),
    DeviceObject(BACnetDeviceObjectReference),
}

pub struct Case {
    pub name: String,
    pub reference: Reference,
    pub expected: Vec<u8>,
}

impl Case {
    pub fn encode<S: EncodeSink + ?Sized>(&self, sink: &mut S) -> Result<usize, SinkFull> {
        match &self.reference {
            Reference::Property(r) => encode_property_reference_to_sink(sink, r),
            Reference::ObjectProperty(r) => encode_object_property_reference_to_sink(sink, r),
            Reference::Setpoint(r) => encode_setpoint_reference_to_sink(sink, r),
            Reference::DeviceProperty(r) => {
                encode_device_object_property_reference_to_sink(sink, r)
            }
            Reference::DeviceObject(r) => encode_device_object_reference_to_sink(sink, r),
        }
    }
    pub fn encode_legacy(&self, sink: &mut BytesMut) {
        match &self.reference {
            Reference::Property(r) => encode_property_reference(sink, r),
            Reference::ObjectProperty(r) => encode_object_property_reference(sink, r),
            Reference::Setpoint(r) => encode_setpoint_reference(sink, r),
            Reference::DeviceProperty(r) => encode_device_object_property_reference(sink, r),
            Reference::DeviceObject(r) => encode_device_object_reference(sink, r),
        }
    }
    pub fn bound(&self) -> usize {
        match self.reference {
            Reference::Property(_) | Reference::DeviceObject(_) => 10,
            Reference::ObjectProperty(_) => 15,
            Reference::Setpoint(_) => 17,
            Reference::DeviceProperty(_) => 20,
        }
    }
}

fn object() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 17).unwrap()
}
fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 500).unwrap()
}
fn join(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

// Goldens below concatenate independently tabulated member octets. No encoder,
// decoder, tag helper or integer-width helper supplies the expected bytes.
fn family(name: &str, property: u32, index: Option<u32>, members: [&[u8]; 4]) -> Vec<Case> {
    let [property_0, property_1, index_1, index_2] = members;
    let object_member = [0x0C, 0x00, 0x00, 0x00, 0x11];
    let device_member = [0x3C, 0x02, 0x00, 0x01, 0xF4];
    let body = join(&[&object_member, property_1, index_2]);
    let local = BACnetObjectPropertyReference {
        object_identifier: object(),
        property_identifier: property,
        property_array_index: index,
    };
    let mut cases = vec![
        Case {
            name: format!("{name}/property"),
            reference: Reference::Property(PropertyReference {
                property_identifier: PropertyIdentifier::from_raw(property),
                property_array_index: index,
            }),
            expected: join(&[property_0, index_1]),
        },
        Case {
            name: format!("{name}/object-property"),
            reference: Reference::ObjectProperty(local.clone()),
            expected: body.clone(),
        },
        Case {
            name: format!("{name}/setpoint"),
            reference: Reference::Setpoint(local),
            expected: join(&[&[0x0E], &body, &[0x0F]]),
        },
    ];
    for (suffix, selected_device, expected) in [
        ("local", None, body.clone()),
        ("remote", Some(device()), join(&[&body, &device_member])),
    ] {
        cases.push(Case {
            name: format!("{name}/device-property-{suffix}"),
            reference: Reference::DeviceProperty(BACnetDeviceObjectPropertyReference {
                object_identifier: object(),
                property_identifier: property,
                property_array_index: index,
                device_identifier: selected_device,
            }),
            expected,
        });
    }
    cases
}

pub fn wire_cases() -> Vec<Case> {
    let mut cases = family(
        "present",
        85,
        None,
        [&[0x09, 0x55], &[0x19, 0x55], &[], &[]],
    );
    cases.extend(family(
        "array-count",
        87,
        Some(0),
        [&[0x09, 0x57], &[0x19, 0x57], &[0x19, 0], &[0x29, 0]],
    ));
    cases.extend(family(
        "array-element",
        87,
        Some(4),
        [&[0x09, 0x57], &[0x19, 0x57], &[0x19, 4], &[0x29, 4]],
    ));
    for (name, selected_device, expected) in [
        ("local", None, &[0x1C, 0, 0, 0, 0x11][..]),
        (
            "remote",
            Some(device()),
            &[0x0C, 0x02, 0, 0x01, 0xF4, 0x1C, 0, 0, 0, 0x11][..],
        ),
    ] {
        cases.push(Case {
            name: format!("{name}/device-object"),
            reference: Reference::DeviceObject(BACnetDeviceObjectReference {
                device_identifier: selected_device,
                object_identifier: object(),
            }),
            expected: expected.to_vec(),
        });
    }
    cases
}

// Raw properties/indices test retained encoder input handling, not semantic
// validity for a particular property or service. Every unsigned width is literal.
type UnsignedLiteral = (u32, &'static [u8], &'static [u8], &'static [u8]);
const WIDTHS: &[UnsignedLiteral] = &[
    (0, &[0x09, 0], &[0x19, 0], &[0x29, 0]),
    (255, &[0x09, 0xFF], &[0x19, 0xFF], &[0x29, 0xFF]),
    (256, &[0x0A, 1, 0], &[0x1A, 1, 0], &[0x2A, 1, 0]),
    (
        65535,
        &[0x0A, 0xFF, 0xFF],
        &[0x1A, 0xFF, 0xFF],
        &[0x2A, 0xFF, 0xFF],
    ),
    (65536, &[0x0B, 1, 0, 0], &[0x1B, 1, 0, 0], &[0x2B, 1, 0, 0]),
    (
        16777215,
        &[0x0B, 0xFF, 0xFF, 0xFF],
        &[0x1B, 0xFF, 0xFF, 0xFF],
        &[0x2B, 0xFF, 0xFF, 0xFF],
    ),
    (
        16777216,
        &[0x0C, 1, 0, 0, 0],
        &[0x1C, 1, 0, 0, 0],
        &[0x2C, 1, 0, 0, 0],
    ),
    (
        u32::MAX,
        &[0x0C, 0xFF, 0xFF, 0xFF, 0xFF],
        &[0x1C, 0xFF, 0xFF, 0xFF, 0xFF],
        &[0x2C, 0xFF, 0xFF, 0xFF, 0xFF],
    ),
];

pub fn compatibility_cases() -> Vec<Case> {
    let mut cases = Vec::new();
    for &(property, property_0, property_1, _) in WIDTHS {
        cases.extend(family(
            &format!("raw-{property}/absent"),
            property,
            None,
            [property_0, property_1, &[], &[]],
        ));
        for &(index, _, index_1, index_2) in WIDTHS {
            cases.extend(family(
                &format!("raw-{property}/index-{index}"),
                property,
                Some(index),
                [property_0, property_1, index_1, index_2],
            ));
        }
    }
    let maximum = ObjectIdentifier::new(ObjectType::from_raw(1023), 4194303).unwrap();
    let max_body = [
        0x0C, 0xFF, 0xFF, 0xFF, 0xFF, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF, 0x2C, 0xFF, 0xFF, 0xFF, 0xFF,
    ];
    let max_ref = BACnetObjectPropertyReference {
        object_identifier: maximum,
        property_identifier: u32::MAX,
        property_array_index: Some(u32::MAX),
    };
    cases.push(Case {
        name: "max-identifier/object-property".into(),
        reference: Reference::ObjectProperty(max_ref.clone()),
        expected: max_body.to_vec(),
    });
    cases.push(Case {
        name: "max-identifier/setpoint".into(),
        reference: Reference::Setpoint(max_ref),
        expected: join(&[&[0x0E], &max_body, &[0x0F]]),
    });
    cases.push(Case {
        name: "non-device/device-property".into(),
        reference: Reference::DeviceProperty(BACnetDeviceObjectPropertyReference {
            object_identifier: maximum,
            property_identifier: u32::MAX,
            property_array_index: Some(u32::MAX),
            device_identifier: Some(object()),
        }),
        expected: join(&[&max_body, &[0x3C, 0, 0, 0, 0x11]]),
    });
    cases.push(Case {
        name: "non-device/device-object".into(),
        reference: Reference::DeviceObject(BACnetDeviceObjectReference {
            device_identifier: Some(object()),
            object_identifier: maximum,
        }),
        expected: vec![0x0C, 0, 0, 0, 0x11, 0x1C, 0xFF, 0xFF, 0xFF, 0xFF],
    });
    cases.push(Case {
        name: "max-device-field/device-object".into(),
        reference: Reference::DeviceObject(BACnetDeviceObjectReference {
            device_identifier: Some(maximum),
            object_identifier: object(),
        }),
        expected: vec![0x0C, 0xFF, 0xFF, 0xFF, 0xFF, 0x1C, 0, 0, 0, 0x11],
    });
    cases
}

pub fn all_cases() -> Vec<Case> {
    let mut cases = wire_cases();
    cases.extend(compatibility_cases());
    cases
}
