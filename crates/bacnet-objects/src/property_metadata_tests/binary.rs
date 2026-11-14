//! The binary objects' rows in every configuration they have: profile rows,
//! the Binary Value's audit rows, and its Present_Value access with and
//! without a tracked source (#2182).

use super::*;

use crate::audit::{AuditPriorityPolicy, ObjectAuditPolicy};
use crate::object_profile::ObjectProfile;
use crate::present_value_access::PresentValueAccess;
use bacnet_types::bitstring::AuditOperationFlags;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::enums::AuditLevel;

/// The three objects, the Value with `access` and its source tracked when
/// `tracked`, with configuration's bits on: 1 every profile row, 2 the
/// Binary Value's three audit rows.
fn binary_objects(
    configuration: u8,
    access: PresentValueAccess,
    tracked: bool,
) -> [Box<dyn BACnetObject>; 3] {
    let mut bi = BinaryInputObject::new(1, "BI-1").unwrap();
    let mut bo = BinaryOutputObject::new(1, "BO-1").unwrap();
    let mut bv = BinaryValueObject::with_access(1, "BV-1", access).unwrap();
    bv.set_value_source_tracking(tracked);
    if configuration & 1 != 0 {
        let profile = || ObjectProfile {
            tags: Some(vec![BACnetNameValue {
                name: "zone".into(),
                value: None,
            }]),
            profile_location: Some("bacnet://profiles/binary".into()),
            profile_name: Some("555-binary".into()),
        };
        bi.set_profile(profile()).unwrap();
        bo.set_profile(profile()).unwrap();
        bv.set_profile(profile()).unwrap();
    }
    if configuration & 2 != 0 {
        bv.set_audit_policy(ObjectAuditPolicy {
            level: Some(AuditLevel::DEFAULT),
            operations: Some(AuditOperationFlags::empty()),
            priority_filter: Some(AuditPriorityPolicy::Inherit),
        });
    }
    [Box::new(bi), Box::new(bo), Box::new(bv)]
}

#[test]
fn property_metadata_binary_exact_required_and_instance_projections() {
    use PropertyIdentifier as P;

    // Tables 12-6, 12-8 and 12-10 require these of an object that reports
    // intrinsically; Time_Delay_Normal and Event_Message_Texts they only
    // permit (#1485).
    let intrinsic_required = [
        P::EVENT_DETECTION_ENABLE,
        P::EVENT_ENABLE,
        P::TIME_DELAY,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::ALARM_VALUE,
        P::FEEDBACK_VALUE,
    ];
    let intrinsic_permitted = [
        P::TIME_DELAY_NORMAL,
        P::EVENT_MESSAGE_TEXTS,
        P::EVENT_MESSAGE_TEXTS_CONFIG,
        P::EVENT_ALGORITHM_INHIBIT_REF,
        P::EVENT_ALGORITHM_INHIBIT,
    ];
    let commandable = [
        P::PRIORITY_ARRAY,
        P::RELINQUISH_DEFAULT,
        P::CURRENT_COMMAND_PRIORITY,
    ];
    let detector = [
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
    ];
    let reliability = [P::RELIABILITY, P::RELIABILITY_EVALUATION_INHIBIT];
    let texts = [P::ACTIVE_TEXT, P::INACTIVE_TEXT];
    let sources = [P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME];
    let audit = [
        P::AUDIT_LEVEL,
        P::AUDITABLE_OPERATIONS,
        P::AUDIT_PRIORITY_FILTER,
    ];
    let profile = [P::TAGS, P::PROFILE_LOCATION, P::PROFILE_NAME];
    let accesses = [
        (PresentValueAccess::Commandable, false),
        (PresentValueAccess::Writable, false),
        (PresentValueAccess::Writable, true),
        (PresentValueAccess::ReadOnly, false),
        (PresentValueAccess::ReadOnly, true),
    ];
    for configuration in 0..4 {
        for (access, tracked) in accesses {
            for object in binary_objects(configuration, access, tracked) {
                let kind = object.object_identifier().object_type();
                let commanded = match kind {
                    ObjectType::BINARY_INPUT => false,
                    ObjectType::BINARY_OUTPUT => true,
                    _ => access == PresentValueAccess::Commandable,
                };
                let mut expected = vec![
                    P::OBJECT_IDENTIFIER,
                    P::OBJECT_NAME,
                    P::DESCRIPTION,
                    P::OBJECT_TYPE,
                    P::PRESENT_VALUE,
                ];
                let mut requires = vec![
                    P::OBJECT_IDENTIFIER,
                    P::OBJECT_NAME,
                    P::OBJECT_TYPE,
                    P::PRESENT_VALUE,
                    P::STATUS_FLAGS,
                    P::EVENT_STATE,
                    P::OUT_OF_SERVICE,
                    P::POLARITY,
                ];
                requires.extend(intrinsic_required);
                if kind == ObjectType::BINARY_OUTPUT {
                    expected.push(P::FEEDBACK_VALUE);
                    requires.extend(commandable);
                }
                expected.extend(detector);
                if commanded {
                    expected.extend(commandable);
                }
                if kind != ObjectType::BINARY_VALUE {
                    expected.push(P::POLARITY);
                }
                expected.extend(reliability);
                expected.extend(texts);
                if kind != ObjectType::BINARY_OUTPUT {
                    expected.push(P::ALARM_VALUE);
                }
                if commanded {
                    expected.extend(sources);
                    requires.extend(sources);
                } else if kind == ObjectType::BINARY_VALUE && tracked {
                    expected.push(P::VALUE_SOURCE);
                    requires.push(P::VALUE_SOURCE);
                }
                // The rows an instance adds follow the table's, audit rows
                // first and profile rows last; Audit_Priority_Filter only
                // while commandable.
                if configuration & 2 != 0 && kind == ObjectType::BINARY_VALUE {
                    let filter = commanded as usize;
                    expected.extend(&audit[..2 + filter]);
                }
                if configuration & 1 != 0 {
                    expected.extend(profile);
                }
                let what = format!("{kind:?} {configuration} {access:?} {tracked}");
                // Metadata order is the list's.
                let mut required: Vec<_> = expected
                    .iter()
                    .copied()
                    .filter(|p| requires.contains(p))
                    .collect();
                required.push(P::PROPERTY_LIST);
                assert_unique_and_canonical(object.as_ref());
                assert_eq!(
                    crate::test_rows::required(&object.property_metadata()),
                    required,
                    "{what}"
                );
                assert_eq!(
                    crate::test_rows::listed(&object.property_metadata()),
                    crate::test_rows::listed_from(&expected),
                    "{what}"
                );
                let metadata = object.property_metadata();
                assert_eq!(metadata.len(), expected.len() + 1, "{what}");
                for row in metadata.iter() {
                    let p = row.property_identifier;
                    if intrinsic_required.contains(&p) {
                        assert_eq!(
                            row.presence_condition,
                            Some(PropertyPresenceCondition::IntrinsicReportingRequired),
                            "{what} {p:?}"
                        );
                    } else if intrinsic_permitted.contains(&p) {
                        assert_eq!(
                            row.presence_condition,
                            Some(PropertyPresenceCondition::IntrinsicReportingOptional),
                            "{what} {p:?}"
                        );
                    } else if texts.contains(&p) {
                        assert_eq!(
                            row.presence_condition,
                            Some(PropertyPresenceCondition::PairedText),
                            "{what} {p:?}"
                        );
                    } else if commandable.contains(&p) {
                        assert_eq!(
                            row.presence_condition,
                            (kind == ObjectType::BINARY_VALUE)
                                .then_some(PropertyPresenceCondition::Commandable),
                            "{what} {p:?}"
                        );
                    }
                    let required_read = matches!(
                        p,
                        P::OBJECT_IDENTIFIER
                            | P::OBJECT_NAME
                            | P::OBJECT_TYPE
                            | P::STATUS_FLAGS
                            | P::EVENT_STATE
                            | P::OUT_OF_SERVICE
                            | P::POLARITY
                            | P::PROPERTY_LIST
                    ) || (p == P::PRESENT_VALUE
                        && kind != ObjectType::BINARY_OUTPUT)
                        || (kind == ObjectType::BINARY_OUTPUT && commandable.contains(&p));
                    assert_eq!(
                        row.conformance,
                        if kind == ObjectType::BINARY_OUTPUT && p == P::PRESENT_VALUE {
                            PropertyConformance::RequiredWrite
                        } else if required_read {
                            PropertyConformance::RequiredRead
                        } else {
                            PropertyConformance::Optional
                        },
                        "{what} {p:?}"
                    );
                    if p == P::PRESENT_VALUE {
                        let input = kind == ObjectType::BINARY_INPUT
                            || (kind == ObjectType::BINARY_VALUE
                                && access == PresentValueAccess::ReadOnly);
                        assert_eq!(
                            row.write_capability,
                            if input {
                                PropertyWriteCapability::WhenOutOfService
                            } else {
                                PropertyWriteCapability::Always
                            },
                            "{what}"
                        );
                    }
                    assert!(object.read_property(p, None).is_ok(), "{what} {p:?}");
                }
                let wire: Vec<_> = expected
                    .iter()
                    .filter(|&&p| {
                        !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE)
                    })
                    .map(|p| PropertyValue::Enumerated(p.to_raw()))
                    .collect();
                assert_eq!(
                    object.read_property(P::PROPERTY_LIST, None).unwrap(),
                    PropertyValue::List(wire.clone()),
                    "{what}"
                );
                for (index, value) in wire.iter().enumerate() {
                    assert_eq!(
                        object
                            .read_property(P::PROPERTY_LIST, Some(index as u32 + 1))
                            .unwrap(),
                        *value,
                        "{what}"
                    );
                }
                // What a configuration leaves out isn't served either.
                for p in commandable
                    .into_iter()
                    .chain(sources)
                    .chain(audit)
                    .chain(profile)
                    .chain([P::POLARITY, P::FEEDBACK_VALUE, P::ALARM_VALUE])
                {
                    assert_eq!(
                        object.read_property(p, None).is_ok(),
                        expected.contains(&p),
                        "{what} {p:?}"
                    );
                }
            }
        }
    }
}
