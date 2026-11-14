//! The binary objects' property tables: their order, their conditions and
//! the write capabilities their dispatch honours.

use crate::binary::{BinaryOutputObject, BinaryValueObject};
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyPresenceCondition::{
        Commandable, CommandableValueSourceTracking, IntrinsicReportingOptional,
        IntrinsicReportingRequired, PairedText, ValueSourceTracking,
    },
    PropertyWriteCapability::{Always, ReadOnly, WhenCommandOwner, WhenOutOfService},
};
use crate::traits::BACnetObject;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::PropertyValue;

#[test]
fn property_metadata_feedback_stays_independent_of_command_resolution() {
    for out_of_service in [false, true] {
        let mut object = BinaryOutputObject::new(1, "BO-1").unwrap();
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
                PropertyValue::Enumerated(1),
                Some(8),
                &crate::command_source::test_origin(),
            )
            .unwrap();
        assert_eq!(
            object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
            PropertyValue::Enumerated(0)
        );
        object
            .write_property(P::FEEDBACK_VALUE, None, PropertyValue::Enumerated(1), None)
            .unwrap();
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
            PropertyValue::Enumerated(0)
        );
        assert_eq!(
            object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
            PropertyValue::Enumerated(1)
        );
        assert!(
            matches!(object.write_property(P::FEEDBACK_VALUE, None, PropertyValue::Enumerated(2), None),
            Err(Error::Remote(RemoteError::Error { class, code }))
                if class == ErrorClass::PROPERTY
                    && code == ErrorCode::VALUE_OUT_OF_RANGE)
        );
        assert_eq!(
            object.read_property(P::FEEDBACK_VALUE, None).unwrap(),
            PropertyValue::Enumerated(1)
        );
    }
}

fn objects() -> [Box<dyn BACnetObject>; 2] {
    [
        Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
    ]
}

fn assert_error(error: Error, expected: ErrorCode) {
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { class, code })
        if class == ErrorClass::PROPERTY && code == expected),
        "expected {expected:?}, got {error:?}"
    );
}

#[test]
fn property_metadata_binary_legacy_order_and_indexed_list() {
    // Independent legacy-list fixture shared by the two commandable types.
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
        P::PRIORITY_ARRAY,
        P::RELINQUISH_DEFAULT,
        P::CURRENT_COMMAND_PRIORITY,
        P::RELIABILITY,
        P::RELIABILITY_EVALUATION_INHIBIT,
        P::ACTIVE_TEXT,
        P::INACTIVE_TEXT,
    ];
    for mut object in objects() {
        let output = object.object_identifier().object_type() == ObjectType::BINARY_OUTPUT;
        let mut expected = base.to_vec();
        if output {
            expected.insert(23, P::POLARITY);
            expected.insert(5, P::FEEDBACK_VALUE);
        } else {
            expected.push(P::ALARM_VALUE);
        }
        expected.extend([P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME]);
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
            assert_eq!(original.len(), expected.len() + 1);
            let required = crate::test_rows::required(&object.property_metadata());
            for row in &original {
                let p = row.property_identifier;
                let condition = match p {
                    P::VALUE_SOURCE => Some(ValueSourceTracking),
                    P::VALUE_SOURCE_ARRAY | P::LAST_COMMAND_TIME => {
                        Some(CommandableValueSourceTracking)
                    }
                    P::PRIORITY_ARRAY | P::RELINQUISH_DEFAULT | P::CURRENT_COMMAND_PRIORITY => {
                        (!output).then_some(Commandable)
                    }
                    P::ACTIVE_TEXT | P::INACTIVE_TEXT => Some(PairedText),
                    // Tables 12-8 and 12-10 require these of an object
                    // that reports intrinsically, and only permit the
                    // next two (#1485).
                    P::EVENT_DETECTION_ENABLE
                    | P::EVENT_ENABLE
                    | P::TIME_DELAY
                    | P::NOTIFY_TYPE
                    | P::NOTIFICATION_CLASS
                    | P::ACKED_TRANSITIONS
                    | P::EVENT_TIME_STAMPS
                    | P::ALARM_VALUE
                    | P::FEEDBACK_VALUE => Some(IntrinsicReportingRequired),
                    P::TIME_DELAY_NORMAL
                    | P::EVENT_MESSAGE_TEXTS
                    | P::EVENT_MESSAGE_TEXTS_CONFIG
                    | P::EVENT_ALGORITHM_INHIBIT_REF
                    | P::EVENT_ALGORITHM_INHIBIT => Some(IntrinsicReportingOptional),
                    _ => None,
                };
                assert_eq!(row.presence_condition, condition, "{p:?}");
                let conformance = if output && p == P::PRESENT_VALUE {
                    RequiredWrite
                } else if condition.is_none() && required.contains(&p) {
                    RequiredRead
                } else {
                    Optional
                };
                assert_eq!(row.conformance, conformance, "{p:?}");
                assert_eq!(
                    required.contains(&p),
                    conformance != Optional
                        || matches!(
                            condition,
                            Some(
                                IntrinsicReportingRequired
                                    | ValueSourceTracking
                                    | CommandableValueSourceTracking
                            )
                        ),
                    "{p:?}"
                );
                assert!(object.read_property(p, None).is_ok(), "{p:?}");
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
                P::MIN_PRES_VALUE,
                P::MAX_PRES_VALUE,
                P::FAULT_LOW_LIMIT,
                P::FAULT_HIGH_LIMIT,
            ] {
                assert!(!expected.contains(&p));
                assert_error(
                    object.read_property(p, None).unwrap_err(),
                    ErrorCode::UNKNOWN_PROPERTY,
                );
            }
        }
    }
}

#[test]
fn property_metadata_binary_write_capabilities_match_dispatch() {
    for out_of_service in [false, true] {
        for mut object in objects() {
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
            let metadata = object.property_metadata().into_owned();
            for row in metadata {
                let p = row.property_identifier;
                let capability = match p {
                    P::VALUE_SOURCE => WhenCommandOwner,
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
                    | P::ACTIVE_TEXT
                    | P::INACTIVE_TEXT
                    | P::ALARM_VALUE
                    | P::FEEDBACK_VALUE
                    | P::EVENT_MESSAGE_TEXTS_CONFIG
                    | P::EVENT_ALGORITHM_INHIBIT_REF
                    | P::EVENT_ALGORITHM_INHIBIT => Always,
                    P::RELIABILITY => WhenOutOfService,
                    _ => ReadOnly,
                };
                assert_eq!(row.write_capability, capability, "{p:?}");
                assert_eq!(
                    crate::property_metadata::is_writable_property(&*object, p),
                    capability.is_writable(),
                    "{p:?}"
                );
                let index = (p == P::PRIORITY_ARRAY).then_some(8);
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
            for p in [P::PRESENT_VALUE, P::RELINQUISH_DEFAULT] {
                let index = (p == P::PRIORITY_ARRAY).then_some(8);
                let before = object.read_property(p, index).unwrap();
                assert_error(
                    object
                        .write_property_from(
                            p,
                            index,
                            PropertyValue::Enumerated(2),
                            Some(8),
                            &crate::command_source::test_origin(),
                        )
                        .unwrap_err(),
                    ErrorCode::VALUE_OUT_OF_RANGE,
                );
                assert_eq!(object.read_property(p, index).unwrap(), before);
                object
                    .write_property_from(
                        p,
                        index,
                        PropertyValue::Enumerated(1),
                        Some(8),
                        &crate::command_source::test_origin(),
                    )
                    .unwrap();
                assert_eq!(
                    object.read_property(P::PRESENT_VALUE, None).unwrap(),
                    PropertyValue::Enumerated(1)
                );
            }
            assert_error(
                object
                    .write_property(P::PRIORITY_ARRAY, None, PropertyValue::Null, None)
                    .unwrap_err(),
                ErrorCode::WRITE_ACCESS_DENIED,
            );
            // Relinquish both written slots: the configured default takes over.
            object
                .write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Null,
                    Some(8),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
            object
                .write_property_from(
                    P::PRESENT_VALUE,
                    None,
                    PropertyValue::Null,
                    Some(16),
                    &crate::command_source::test_origin(),
                )
                .unwrap();
            object
                .write_property(
                    P::RELINQUISH_DEFAULT,
                    None,
                    PropertyValue::Enumerated(0),
                    None,
                )
                .unwrap();
            assert_eq!(
                object.read_property(P::PRESENT_VALUE, None).unwrap(),
                PropertyValue::Enumerated(0)
            );
        }
    }
}
