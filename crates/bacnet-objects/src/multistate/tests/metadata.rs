//! The multi-state objects' property tables: their order, their
//! conditions, the write capabilities their dispatch honours, and the
//! State_Text, Alarm_Values and command rows they serve.

use super::super::*;
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyPresenceCondition::{
        Commandable, IntrinsicReportingOptional, IntrinsicReportingRequired,
    },
    PropertyWriteCapability::{Always, ReadOnly, Through, WhenOutOfService},
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;

fn objects() -> [Box<dyn BACnetObject>; 3] {
    [
        Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()),
    ]
}

fn assert_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
        if class == ErrorClass::PROPERTY && code == expected),
        "expected {expected:?}, got {error:?}"
    );
}

/// The event rows the three multi-state tables require of an object
/// that reports intrinsically, besides the watched or fed-back value.
const INTRINSIC_REQUIRED: [P; 7] = [
    P::EVENT_DETECTION_ENABLE,
    P::EVENT_ENABLE,
    P::TIME_DELAY,
    P::NOTIFY_TYPE,
    P::NOTIFICATION_CLASS,
    P::ACKED_TRANSITIONS,
    P::EVENT_TIME_STAMPS,
];

#[test]
fn property_metadata_multistate_exact_sets_legacy_order_and_indexed_list() {
    // Independent fixture: the original MSI list, with per-type insertions
    // and the explicitly selected read-only additions for MSV/MSO.
    let base = [
        P::OBJECT_IDENTIFIER,
        P::OBJECT_NAME,
        P::DESCRIPTION,
        P::OBJECT_TYPE,
        P::PRESENT_VALUE,
        P::STATUS_FLAGS,
        P::EVENT_STATE,
        P::EVENT_DETECTION_ENABLE,
        P::EVENT_ENABLE,
        P::TIME_DELAY,
        P::TIME_DELAY_NORMAL,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::EVENT_MESSAGE_TEXTS,
        P::EVENT_MESSAGE_TEXTS_CONFIG,
        P::EVENT_ALGORITHM_INHIBIT_REF,
        P::EVENT_ALGORITHM_INHIBIT,
        P::OUT_OF_SERVICE,
        P::NUMBER_OF_STATES,
        P::RELIABILITY,
        P::RELIABILITY_EVALUATION_INHIBIT,
        P::STATE_TEXT,
    ];
    for mut object in objects() {
        let kind = object.object_identifier().object_type();
        let output = kind == ObjectType::MULTI_STATE_OUTPUT;
        let commandable = kind != ObjectType::MULTI_STATE_INPUT;
        let commands = [
            P::PRIORITY_ARRAY,
            P::RELINQUISH_DEFAULT,
            P::CURRENT_COMMAND_PRIORITY,
        ];
        let mut expected = base.to_vec();
        let mut requires = vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::EVENT_STATE,
            P::OUT_OF_SERVICE,
            P::NUMBER_OF_STATES,
        ];
        // Tables 12-21, 12-22 and 12-23 require these of an object that
        // reports intrinsically (#1485), with the watched or fed-back
        // value below.
        requires.extend(INTRINSIC_REQUIRED);
        if commandable {
            expected.splice(21..21, commands);
        }
        if output {
            expected.insert(5, P::FEEDBACK_VALUE);
            requires.extend(commands);
            requires.push(P::FEEDBACK_VALUE);
        } else {
            expected.push(P::ALARM_VALUES);
            requires.push(P::ALARM_VALUES);
        }
        if commandable {
            expected.extend([P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME]);
            requires.extend([P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME]);
        }
        // Metadata order is the list's.
        let mut required: Vec<_> = expected
            .iter()
            .copied()
            .filter(|p| requires.contains(p))
            .collect();
        required.push(P::PROPERTY_LIST);
        assert!(matches!(object.property_metadata(), Cow::Borrowed(_)));
        let original = object.property_metadata().into_owned();
        for enabled in [false, true] {
            object
                .write_property(
                    P::EVENT_DETECTION_ENABLE,
                    None,
                    PropertyValue::Boolean(enabled),
                    None,
                )
                .unwrap();
            assert_eq!(object.property_metadata().as_ref(), original);
            assert_eq!(
                crate::test_rows::listed(&object.property_metadata()),
                crate::test_rows::listed_from(&expected)
            );
            assert_eq!(
                crate::test_rows::required(&object.property_metadata()),
                required
            );
            assert_eq!(original.len(), expected.len() + 1);
            for row in &original {
                let p = row.property_identifier;
                let condition = match p {
                    P::VALUE_SOURCE => Some(crate::property_metadata::PropertyPresenceCondition::ValueSourceTracking),
                    P::VALUE_SOURCE_ARRAY | P::LAST_COMMAND_TIME => Some(crate::property_metadata::PropertyPresenceCondition::CommandableValueSourceTracking),
                    P::PRIORITY_ARRAY | P::RELINQUISH_DEFAULT | P::CURRENT_COMMAND_PRIORITY => {
                        (!output).then_some(Commandable)
                    }
                    p if INTRINSIC_REQUIRED.contains(&p) => Some(IntrinsicReportingRequired),
                    P::ALARM_VALUES | P::FEEDBACK_VALUE => Some(IntrinsicReportingRequired),
                    P::TIME_DELAY_NORMAL
                    | P::EVENT_MESSAGE_TEXTS
                    | P::EVENT_MESSAGE_TEXTS_CONFIG
                    | P::EVENT_ALGORITHM_INHIBIT_REF
                    | P::EVENT_ALGORITHM_INHIBIT => Some(IntrinsicReportingOptional),
                    _ => None,
                };
                assert_eq!(row.presence_condition, condition, "{kind:?} {p:?}");
                let conformance = if output && p == P::PRESENT_VALUE {
                    RequiredWrite
                } else if condition.is_none() && required.contains(&p) {
                    RequiredRead
                } else {
                    Optional
                };
                assert_eq!(row.conformance, conformance, "{kind:?} {p:?}");
                assert!(object.read_property(p, None).is_ok(), "{kind:?} {p:?}");
            }
            let wire: Vec<_> = expected
                .iter()
                .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
                .map(|p| PropertyValue::Enumerated(p.to_raw()))
                .collect();
            assert_eq!(
                object.read_property(P::PROPERTY_LIST, None).unwrap(),
                PropertyValue::List(wire.clone())
            );
            assert_eq!(
                object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
                PropertyValue::Unsigned(wire.len() as u64)
            );
            for (index, value) in wire.iter().enumerate() {
                assert_eq!(
                    object
                        .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                        .unwrap(),
                    *value
                );
            }
            assert_error(
                object
                    .read_property(P::PROPERTY_LIST, Some(wire.len() as u32 + 1))
                    .unwrap_err(),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
            for p in [
                P::ALARM_VALUES,
                P::FEEDBACK_VALUE,
                P::PRIORITY_ARRAY,
                P::RELINQUISH_DEFAULT,
                P::CURRENT_COMMAND_PRIORITY,
                P::VALUE_SOURCE,
                P::LAST_COMMAND_TIME,
            ] {
                if !expected.contains(&p) {
                    assert_error(
                        object.read_property(p, None).unwrap_err(),
                        ErrorCode::UNKNOWN_PROPERTY,
                    );
                    assert!(!crate::property_metadata::is_writable_property(&*object, p));
                }
            }
        }
    }
}

#[test]
fn property_metadata_multistate_write_capabilities_match_dispatch() {
    for out_of_service in [false, true] {
        for mut object in objects() {
            let input = object.object_identifier().object_type() == ObjectType::MULTI_STATE_INPUT;
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            // Event_Algorithm_Inhibit takes a write only while detection is
            // on, which an output doesn't start with (#1329).
            object
                .write_property(
                    P::EVENT_DETECTION_ENABLE,
                    None,
                    PropertyValue::Boolean(true),
                    None,
                )
                .unwrap();
            for row in object.property_metadata().into_owned() {
                let p = row.property_identifier;
                let capability = match p {
                    P::VALUE_SOURCE => {
                        crate::property_metadata::PropertyWriteCapability::WhenCommandOwner
                    }
                    P::PRESENT_VALUE if input => WhenOutOfService,
                    P::RELIABILITY => WhenOutOfService,
                    // Set by a whole State_Text write (#1443).
                    P::NUMBER_OF_STATES => Through(P::STATE_TEXT),
                    P::OBJECT_NAME
                    | P::DESCRIPTION
                    | P::PRESENT_VALUE
                    | P::OUT_OF_SERVICE
                    | P::RELINQUISH_DEFAULT
                    | P::EVENT_DETECTION_ENABLE
                    | P::EVENT_ENABLE
                    | P::TIME_DELAY
                    | P::TIME_DELAY_NORMAL
                    | P::NOTIFY_TYPE
                    | P::NOTIFICATION_CLASS
                    | P::RELIABILITY_EVALUATION_INHIBIT
                    | P::STATE_TEXT
                    | P::ALARM_VALUES
                    | P::FEEDBACK_VALUE
                    | P::EVENT_MESSAGE_TEXTS_CONFIG
                    | P::EVENT_ALGORITHM_INHIBIT_REF
                    | P::EVENT_ALGORITHM_INHIBIT => Always,
                    _ => ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    crate::property_metadata::is_writable_property(&*object, p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let index = match p {
                    P::PRIORITY_ARRAY => Some(8),
                    P::STATE_TEXT => Some(1),
                    _ => None,
                };
                let value = object.read_property(p, index).unwrap();
                let result = object.write_property_from(
                    p,
                    index,
                    value,
                    None,
                    &crate::command_source::test_origin(),
                );
                if capability == crate::property_metadata::PropertyWriteCapability::WhenCommandOwner
                    || capability == Always
                    || (capability == WhenOutOfService && out_of_service)
                {
                    result.unwrap();
                } else {
                    assert_error(result.unwrap_err(), ErrorCode::WRITE_ACCESS_DENIED);
                }
            }
            assert!(!crate::property_metadata::is_writable_property(
                &*object,
                P::ALL
            ));
            assert_error(
                object
                    .write_property(P::ALL, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::UNKNOWN_PROPERTY,
            );
        }
    }
}

#[test]
fn property_metadata_multistate_state_text_writes_by_element_or_whole() {
    for mut object in objects() {
        assert!(crate::property_metadata::is_array_property(
            &*object,
            P::STATE_TEXT
        ));
        assert!(!crate::property_metadata::is_array_property(
            &*object,
            P::NUMBER_OF_STATES
        ));
        let mut expected: Vec<_> = (1..=3)
            .map(|i| PropertyValue::CharacterString(format!("State {i}")))
            .collect();
        assert_eq!(
            object.read_property(P::STATE_TEXT, Some(0)).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            object.read_property(P::STATE_TEXT, None).unwrap(),
            PropertyValue::List(expected.clone())
        );
        for index in 1..=3 {
            let value = PropertyValue::CharacterString(format!("Label {index}"));
            object
                .write_property(P::STATE_TEXT, Some(index), value.clone(), None)
                .unwrap();
            assert_eq!(
                object.read_property(P::STATE_TEXT, Some(index)).unwrap(),
                value
            );
            expected[index as usize - 1] = value;
        }
        for (index, value, error) in [
            // Whole, it sets Number_Of_States, which can't be zero (#1443).
            (
                None,
                PropertyValue::List(vec![]),
                ErrorCode::VALUE_OUT_OF_RANGE,
            ),
            // The size takes a count (#1443), not a label.
            (
                Some(0),
                PropertyValue::CharacterString("x".into()),
                ErrorCode::INVALID_DATA_TYPE,
            ),
            (
                Some(4),
                PropertyValue::CharacterString("extra".into()),
                ErrorCode::INVALID_ARRAY_INDEX,
            ),
            (
                Some(1),
                PropertyValue::Unsigned(1),
                ErrorCode::INVALID_DATA_TYPE,
            ),
        ] {
            assert_error(
                object
                    .write_property(P::STATE_TEXT, index, value, None)
                    .unwrap_err(),
                error,
            );
            assert_eq!(
                object.read_property(P::STATE_TEXT, None).unwrap(),
                PropertyValue::List(expected.clone())
            );
        }
        assert_error(
            object.read_property(P::STATE_TEXT, Some(4)).unwrap_err(),
            ErrorCode::INVALID_ARRAY_INDEX,
        );
    }
}

#[test]
fn property_metadata_multistate_alarm_list_cap_and_atomic_rejection() {
    for mut object in objects().into_iter().take(2) {
        assert!(!crate::property_metadata::is_array_property(
            &*object,
            P::ALARM_VALUES
        ));
        let at_cap = PropertyValue::List(vec![
            PropertyValue::Unsigned(2);
            crate::multistate::MAX_ALARM_VALUES
        ]);
        object
            .write_property(P::ALARM_VALUES, None, at_cap.clone(), None)
            .unwrap();
        // A refused element is named by its position (#1048): the
        // out-of-range second value, and the first value past the cap.
        for (index, value, class, code, element) in [
            (
                Some(0),
                PropertyValue::List(vec![]),
                ErrorClass::PROPERTY,
                ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
                None,
            ),
            (
                None,
                PropertyValue::Unsigned(1),
                ErrorClass::PROPERTY,
                ErrorCode::INVALID_DATA_TYPE,
                None,
            ),
            (
                None,
                PropertyValue::List(vec![
                    PropertyValue::Unsigned(1),
                    PropertyValue::Unsigned(u64::from(u32::MAX) + 1),
                ]),
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
                Some(2),
            ),
            (
                None,
                PropertyValue::List(vec![
                    PropertyValue::Unsigned(1);
                    crate::multistate::MAX_ALARM_VALUES + 1
                ]),
                ErrorClass::RESOURCES,
                ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
                Some(crate::multistate::MAX_ALARM_VALUES as u32 + 1),
            ),
        ] {
            let result = object.write_property(P::ALARM_VALUES, index, value, None);
            match element {
                Some(position) => crate::common::assert_list_element_refused(
                    result,
                    class,
                    code,
                    position,
                    &format!("{code:?}"),
                ),
                None => assert!(
                    matches!(result, Err(Error::Remote(RemoteError::Error { class: actual_class, code: actual_code }))
                    if actual_class == class && actual_code == code)
                ),
            }
            assert_eq!(object.read_property(P::ALARM_VALUES, None).unwrap(), at_cap);
        }
        object
            .write_property(P::ALARM_VALUES, None, PropertyValue::List(vec![]), None)
            .unwrap();
        assert_eq!(
            object.read_property(P::ALARM_VALUES, None).unwrap(),
            PropertyValue::List(vec![])
        );
    }
}

#[test]
fn property_metadata_multistate_feedback_stays_independent_of_commands_and_resize() {
    for out_of_service in [false, true] {
        let mut object = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
        let metadata = object.property_metadata().into_owned();
        object
            .write_property(
                P::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(out_of_service),
                None,
            )
            .unwrap();
        object
            .write_property_from(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Unsigned(2),
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
        assert_eq!(
            object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
            PropertyValue::Unsigned(1)
        );
        for feedback in [0, 7, u64::from(u32::MAX)] {
            object
                .write_property(
                    P::FEEDBACK_VALUE,
                    None,
                    PropertyValue::Unsigned(feedback),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Unsigned(2)
            );
            assert_eq!(
                object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
                PropertyValue::Unsigned(feedback)
            );
        }
        assert!(
            matches!(object.write_property(P::FEEDBACK_VALUE, None, PropertyValue::Unsigned(0x1_0000_0002), None),
            Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY
                && code == ErrorCode::VALUE_OUT_OF_RANGE)
        );
        object.set_relinquish_default(3).unwrap();
        object
            .write_property_from(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Null,
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
        object.set_number_of_states(1).unwrap();
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            object.read_property(P::RELINQUISH_DEFAULT, None).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
            PropertyValue::Unsigned(u64::from(u32::MAX))
        );
        assert_eq!(object.property_metadata().as_ref(), metadata);
    }
}

#[test]
fn property_metadata_multistate_command_validation_and_optional_readbacks() {
    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        Box::new(MultiStateOutputObject::new(1, "MSO-1", 3).unwrap()),
    ];
    for mut object in objects {
        let readbacks =
            [P::VALUE_SOURCE, P::LAST_COMMAND_TIME].map(|p| object.read_property(p, None).unwrap());
        for p in [P::PRESENT_VALUE, P::RELINQUISH_DEFAULT] {
            let index = (p == P::PRIORITY_ARRAY).then_some(8);
            for invalid in [0, 4, u64::from(u32::MAX) + 1] {
                let before = object.read_property(p, index).unwrap();
                assert!(
                    matches!(object.write_property_from(p, index, PropertyValue::Unsigned(invalid), Some(8), &crate::command_source::test_origin()),
                    Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY
                        && code == ErrorCode::VALUE_OUT_OF_RANGE)
                );
                assert_eq!(object.read_property(p, index).unwrap(), before);
            }
            object
                .write_property_from(
                    p,
                    index,
                    PropertyValue::Unsigned(3),
                    Some(8),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
        }
        assert!(
            matches!(object.write_property(P::PRIORITY_ARRAY, None, PropertyValue::Null, None),
            Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY
                && code == ErrorCode::WRITE_ACCESS_DENIED)
        );
        object
            .write_property_from(
                P::PRESENT_VALUE,
                None,
                PropertyValue::Null,
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
        assert_eq!(
            object.read_property(P::PRESENT_VALUE, None).unwrap(),
            PropertyValue::Unsigned(3)
        );
        assert_eq!(
            object
                .read_property(P::CURRENT_COMMAND_PRIORITY, None)
                .unwrap(),
            PropertyValue::Null
        );
        assert_eq!(
            object.read_property(P::VALUE_SOURCE, None).unwrap(),
            readbacks[0]
        );
        assert_ne!(
            object.read_property(P::LAST_COMMAND_TIME, None).unwrap(),
            readbacks[1]
        );
        assert!(crate::property_metadata::is_writable_property(
            &*object,
            P::VALUE_SOURCE
        ));
        assert!(!crate::property_metadata::is_writable_property(
            &*object,
            P::LAST_COMMAND_TIME
        ));
        assert!(!crate::property_metadata::is_array_property(
            &*object,
            P::VALUE_SOURCE
        ));
    }
}

#[test]
fn property_metadata_multistate_resize_preserves_retained_configuration() {
    let mut object = MultiStateValueObject::new(1, "MSV-1", 3).unwrap();
    let metadata = object.property_metadata().into_owned();
    object
        .write_property_from(
            P::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(3),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
    object.set_relinquish_default(3).unwrap();
    object.set_alarm_values(vec![3]).unwrap();
    object
        .write_property(
            P::STATE_TEXT,
            Some(1),
            PropertyValue::CharacterString("Retained".into()),
            None,
        )
        .unwrap();
    object.set_number_of_states(1).unwrap();
    assert!(object.set_number_of_states(0).is_err());
    assert_eq!(object.property_metadata().as_ref(), metadata);
    for p in [P::PRESENT_VALUE, P::RELINQUISH_DEFAULT] {
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::Unsigned(3)
        );
    }
    assert_eq!(
        object.read_property(P::PRIORITY_ARRAY, Some(8)).unwrap(),
        PropertyValue::Unsigned(3)
    );
    assert_eq!(
        object.read_property(P::ALARM_VALUES, None).unwrap(),
        PropertyValue::List(vec![PropertyValue::Unsigned(3)])
    );
    assert_eq!(
        object.read_property(P::STATE_TEXT, Some(0)).unwrap(),
        PropertyValue::Unsigned(1)
    );
    object.set_number_of_states(3).unwrap();
    assert_eq!(
        object.read_property(P::STATE_TEXT, None).unwrap(),
        PropertyValue::List(vec![
            PropertyValue::CharacterString("Retained".into()),
            PropertyValue::CharacterString("State 2".into()),
            PropertyValue::CharacterString("State 3".into()),
        ])
    );
}
