//! The octets an analog object saves through its own write path: its
//! reporting file (the kept rows, Description and OUT_OF_RANGE's
//! parameters) and its Tags file, pinned against the build before the
//! objects moved onto `IoCore` (#2182).

use super::super::*;
use crate::object_profile::{FileTagsPersistence, ObjectProfile};
use crate::present_value_access::PresentValueAccess;
use crate::write_config::FileWriteConfigPersistence;
use alloc::sync::Arc;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::PropertyIdentifier as P;
use std::path::Path;

/// The reporting file and the Tags file of Analog Input, Output and Value
/// 3 after [`writes`], as hex.
const GOLDEN: [(ObjectType, &str, &str); 3] = [
    (
        ObjectType::ANALOG_INPUT,
        concat!(
            "52424e57434630310000000305",
            "0000000d626f696c657220737570706c79",
            "0601000000070105010000000101000100000004686967680000000662726f6b656e",
            "000000046261636b00000000003f0000001e0000002d42a10000c0a800003fc0000003",
        ),
        "52424e544147303100000003010c00686f74",
    ),
    (
        ObjectType::ANALOG_OUTPUT,
        concat!(
            "52424e57434630310040000305",
            "0000000d626f696c657220737570706c79",
            "0601000000070105010000000101000100000004686967680000000662726f6b656e",
            "000000046261636b00000000003f0000001e0000002d42a10000c0a800003fc0000003",
        ),
        "52424e544147303100400003010c00686f74",
    ),
    (
        ObjectType::ANALOG_VALUE,
        concat!(
            "52424e57434630310080000305",
            "0000000d626f696c657220737570706c79",
            "0601000000070105010000000101000100000004686967680000000662726f6b656e",
            "000000046261636b00000000003f0000001e0000002d42a10000c0a800003fc0000003",
        ),
        "52424e544147303100800003010c00686f74",
    ),
];

/// A write of every row the reporting file keeps for an analog object,
/// detection last: it is the one that takes the rest of the history away.
fn writes() -> Vec<(PropertyIdentifier, Option<u32>, PropertyValue)> {
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    vec![
        (P::DESCRIPTION, None, text("boiler supply")),
        (P::NOTIFICATION_CLASS, None, PropertyValue::Unsigned(7)),
        (
            P::EVENT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 5,
                data: vec![0b1010_0000],
            },
        ),
        (P::NOTIFY_TYPE, None, PropertyValue::Enumerated(1)),
        (P::TIME_DELAY, None, PropertyValue::Unsigned(30)),
        (P::TIME_DELAY_NORMAL, None, PropertyValue::Unsigned(45)),
        (P::HIGH_LIMIT, None, PropertyValue::Real(80.5)),
        (P::LOW_LIMIT, None, PropertyValue::Real(-5.25)),
        (P::DEADBAND, None, PropertyValue::Real(1.5)),
        (
            P::LIMIT_ENABLE,
            None,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![0b1100_0000],
            },
        ),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            None,
            PropertyValue::List(vec![text("high"), text("broken"), text("back")]),
        ),
        (
            P::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
        ),
    ]
}

fn object(kind: ObjectType, reporting: &Path, tags: &Path) -> Box<dyn BACnetObject> {
    let units = EngineeringUnits::DEGREES_FAHRENHEIT;
    let kept = Arc::new(FileWriteConfigPersistence::new(reporting).unwrap());
    let tags = Arc::new(FileTagsPersistence::new(tags).unwrap());
    let profile = ObjectProfile {
        tags: Some(vec![
            BACnetNameValue {
                name: "zone".into(),
                value: Some(PropertyValue::Unsigned(4)),
            },
            BACnetNameValue {
                name: "hot".into(),
                value: None,
            },
        ]),
        ..ObjectProfile::default()
    };
    match kind {
        ObjectType::ANALOG_INPUT => {
            let mut object = AnalogInputObject::with_tags_persistence(3, "AI-3", units, tags)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap();
            object.set_profile(profile).unwrap();
            Box::new(object)
        }
        ObjectType::ANALOG_OUTPUT => {
            let mut object = AnalogOutputObject::with_tags_persistence(3, "AO-3", units, tags)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap();
            object.set_profile(profile).unwrap();
            Box::new(object)
        }
        _ => {
            let access = PresentValueAccess::Commandable;
            let mut object =
                AnalogValueObject::with_tags_persistence(3, "AV-3", units, access, tags)
                    .and_then(|object| object.with_reporting_persistence(kept))
                    .unwrap();
            object.set_profile(profile).unwrap();
            Box::new(object)
        }
    }
}

fn hex(octets: &[u8]) -> String {
    octets.iter().map(|octet| format!("{octet:02x}")).collect()
}

#[test]
fn analog_objects_save_the_octets_of_the_build_before() {
    let directory =
        std::env::temp_dir().join(format!("rb-2182-analog-files-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for (kind, reporting_golden, tags_golden) in GOLDEN {
        let reporting = directory.join(format!("reporting-{}", kind.to_raw()));
        let tags = directory.join(format!("tags-{}", kind.to_raw()));
        let mut object = object(kind, &reporting, &tags);
        for (property, index, value) in writes() {
            object.write_property(property, index, value, None).unwrap();
        }
        // The second tag alone: a real change to the configured pair.
        let PropertyValue::List(mut configured) = object.read_property(P::TAGS, None).unwrap()
        else {
            panic!("{kind:?}: Tags is an array");
        };
        let second = configured.pop().unwrap();
        object
            .write_property(P::TAGS, None, PropertyValue::List(vec![second]), None)
            .unwrap();
        // A write nobody staged saves before it returns.
        drop(object);
        let reporting = hex(&std::fs::read(&reporting).unwrap());
        let tags = hex(&std::fs::read(&tags).unwrap());
        assert_eq!(
            (reporting.as_str(), tags.as_str()),
            (reporting_golden, tags_golden),
            "{kind:?}"
        );
    }
    std::fs::remove_dir_all(&directory).unwrap();
}
