//! The octets a multi-state object saves through its own write path: its
//! reporting file (the kept rows, Description, its detector's parameters
//! and a written State_Text) and its Tags file, pinned against the build
//! before the objects moved onto `IoCore` (#2182).

use super::super::*;
use crate::object_profile::{FileTagsPersistence, ObjectProfile};
use crate::present_value_access::PresentValueAccess;
use crate::write_config::FileWriteConfigPersistence;
use alloc::sync::Arc;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::PropertyIdentifier as P;
use std::path::Path;

/// The reporting file and the Tags file of Multi-state Input, Output and
/// Value 3 after [`writes`], as hex.
const GOLDEN: [(ObjectType, &str, &str); 3] = [
    (
        ObjectType::MULTI_STATE_INPUT,
        concat!(
            "52424e57434630310340000305",
            "0000000966616e207370656564",
            "0601000000070105010000000101000100000007747269707065640000000662726f6b656e000000046261636b",
            "0000000010830000001e0000002d00000002000000020000000500000005",
            "000000034f666600000004536c6f770000000448696768000000034d61780000000753746174652035",
        ),
        "52424e544147303103400003010c00686f74",
    ),
    (
        ObjectType::MULTI_STATE_OUTPUT,
        concat!(
            "52424e57434630310380000305",
            "0000000966616e207370656564",
            "0601000000070105010000000101000100000007747269707065640000000662726f6b656e000000046261636b",
            "0000000010030000001e0000002d00000005",
            "000000034f666600000004536c6f770000000448696768000000034d61780000000753746174652035",
        ),
        "52424e544147303103800003010c00686f74",
    ),
    (
        ObjectType::MULTI_STATE_VALUE,
        concat!(
            "52424e574346303104c0000305",
            "0000000966616e207370656564",
            "0601000000070105010000000101000100000007747269707065640000000662726f6b656e000000046261636b",
            "0000000010830000001e0000002d00000002000000020000000500000005",
            "000000034f666600000004536c6f770000000448696768000000034d61780000000753746174652035",
        ),
        "52424e544147303104c00003010c00686f74",
    ),
];

/// A write of every row the reporting file keeps for a multi-state object,
/// detection last: it is the one that takes the rest of the history away.
/// State_Text is written whole, by element and by its size, and
/// Alarm_Values names the state the size added; the Output has no
/// Alarm_Values and refuses it.
fn writes() -> Vec<(PropertyIdentifier, Option<u32>, PropertyValue)> {
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    vec![
        (P::DESCRIPTION, None, text("fan speed")),
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
        (
            P::STATE_TEXT,
            None,
            PropertyValue::List(vec![text("Off"), text("Low"), text("High"), text("Max")]),
        ),
        (P::STATE_TEXT, Some(2), text("Slow")),
        (P::STATE_TEXT, Some(0), PropertyValue::Unsigned(5)),
        (
            P::ALARM_VALUES,
            None,
            PropertyValue::List(vec![PropertyValue::Unsigned(2), PropertyValue::Unsigned(5)]),
        ),
        (
            P::EVENT_MESSAGE_TEXTS_CONFIG,
            None,
            PropertyValue::List(vec![text("tripped"), text("broken"), text("back")]),
        ),
        (
            P::EVENT_DETECTION_ENABLE,
            None,
            PropertyValue::Boolean(false),
        ),
    ]
}

fn object(kind: ObjectType, reporting: &Path, tags: &Path) -> Box<dyn BACnetObject> {
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
        ObjectType::MULTI_STATE_INPUT => {
            let mut object = MultiStateInputObject::with_tags_persistence(3, "MSI-3", 3, tags)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap();
            object.set_profile(profile).unwrap();
            Box::new(object)
        }
        ObjectType::MULTI_STATE_OUTPUT => {
            let mut object = MultiStateOutputObject::with_tags_persistence(3, "MSO-3", 3, tags)
                .and_then(|object| object.with_reporting_persistence(kept))
                .unwrap();
            object.set_profile(profile).unwrap();
            Box::new(object)
        }
        _ => {
            let access = PresentValueAccess::Commandable;
            let mut object =
                MultiStateValueObject::with_tags_persistence(3, "MSV-3", 3, access, tags)
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
fn multistate_objects_save_the_octets_of_the_build_before() {
    let directory =
        std::env::temp_dir().join(format!("rb-2182-multistate-files-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    for (kind, reporting_golden, tags_golden) in GOLDEN {
        let reporting = directory.join(format!("reporting-{}", kind.to_raw()));
        let tags = directory.join(format!("tags-{}", kind.to_raw()));
        let mut object = object(kind, &reporting, &tags);
        for (property, index, value) in writes() {
            let result = object.write_property(property, index, value, None);
            if kind == ObjectType::MULTI_STATE_OUTPUT && property == P::ALARM_VALUES {
                assert!(result.is_err(), "{kind:?}: an Output has no Alarm_Values");
            } else {
                result.unwrap();
            }
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
