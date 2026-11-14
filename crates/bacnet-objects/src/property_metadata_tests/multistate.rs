//! The multi-state objects' rows in every configuration they have: each
//! profile row, and the Multi-state Value's Present_Value access with and
//! without a tracked source (#2182). They have no audit rows.

use super::*;

use crate::object_profile::ObjectProfile;
use crate::present_value_access::PresentValueAccess;
use bacnet_types::constructed::BACnetNameValue;

/// The three objects, the Value with `access` and its source tracked when
/// `tracked`, with configuration's bits on: 1 the Tags row, 2 the
/// Profile_Location and Profile_Name rows.
fn multistate_objects(
    configuration: u8,
    access: PresentValueAccess,
    tracked: bool,
) -> [Box<dyn BACnetObject>; 3] {
    let mut msi = MultiStateInputObject::new(1, "MSI-1", 3).unwrap();
    let mut mso = MultiStateOutputObject::new(1, "MSO-1", 3).unwrap();
    let mut msv = MultiStateValueObject::with_access(1, "MSV-1", 3, access).unwrap();
    msv.set_value_source_tracking(tracked);
    if configuration != 0 {
        let profile = || ObjectProfile {
            tags: (configuration & 1 != 0).then(|| {
                vec![BACnetNameValue {
                    name: "zone".into(),
                    value: None,
                }]
            }),
            profile_location: (configuration & 2 != 0)
                .then(|| "bacnet://profiles/multistate".into()),
            profile_name: (configuration & 2 != 0).then(|| "555-multistate".into()),
        };
        msi.set_profile(profile()).unwrap();
        mso.set_profile(profile()).unwrap();
        msv.set_profile(profile()).unwrap();
    }
    [Box::new(msi), Box::new(mso), Box::new(msv)]
}

#[test]
fn property_metadata_multistate_exact_required_and_instance_projections() {
    use PropertyIdentifier as P;

    // Tables 12-21, 12-22 and 12-23 require these of an object that reports
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
        P::ALARM_VALUES,
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
        P::NUMBER_OF_STATES,
    ];
    let reliability = [P::RELIABILITY, P::RELIABILITY_EVALUATION_INHIBIT];
    let sources = [P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME];
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
            for object in multistate_objects(configuration, access, tracked) {
                let kind = object.object_identifier().object_type();
                let commanded = match kind {
                    ObjectType::MULTI_STATE_INPUT => false,
                    ObjectType::MULTI_STATE_OUTPUT => true,
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
                    P::NUMBER_OF_STATES,
                ];
                requires.extend(intrinsic_required);
                if kind == ObjectType::MULTI_STATE_OUTPUT {
                    expected.push(P::FEEDBACK_VALUE);
                    requires.extend(commandable);
                }
                expected.extend(detector);
                if commanded {
                    expected.extend(commandable);
                }
                expected.extend(reliability);
                expected.push(P::STATE_TEXT);
                if kind != ObjectType::MULTI_STATE_OUTPUT {
                    expected.push(P::ALARM_VALUES);
                }
                if commanded {
                    expected.extend(sources);
                    requires.extend(sources);
                } else if kind == ObjectType::MULTI_STATE_VALUE && tracked {
                    expected.push(P::VALUE_SOURCE);
                    requires.push(P::VALUE_SOURCE);
                }
                // The rows a profile adds follow the table's, each on its own.
                if configuration & 1 != 0 {
                    expected.push(P::TAGS);
                }
                if configuration & 2 != 0 {
                    expected.extend([P::PROFILE_LOCATION, P::PROFILE_NAME]);
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
                    } else if commandable.contains(&p) {
                        assert_eq!(
                            row.presence_condition,
                            (kind == ObjectType::MULTI_STATE_VALUE)
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
                            | P::NUMBER_OF_STATES
                            | P::PROPERTY_LIST
                    ) || (p == P::PRESENT_VALUE
                        && kind != ObjectType::MULTI_STATE_OUTPUT)
                        || (kind == ObjectType::MULTI_STATE_OUTPUT && commandable.contains(&p));
                    assert_eq!(
                        row.conformance,
                        if kind == ObjectType::MULTI_STATE_OUTPUT && p == P::PRESENT_VALUE {
                            PropertyConformance::RequiredWrite
                        } else if required_read {
                            PropertyConformance::RequiredRead
                        } else {
                            PropertyConformance::Optional
                        },
                        "{what} {p:?}"
                    );
                    let capability = match p {
                        P::PRESENT_VALUE => {
                            let input = kind == ObjectType::MULTI_STATE_INPUT
                                || (kind == ObjectType::MULTI_STATE_VALUE
                                    && access == PresentValueAccess::ReadOnly);
                            if input {
                                PropertyWriteCapability::WhenOutOfService
                            } else {
                                PropertyWriteCapability::Always
                            }
                        }
                        // Set by a whole State_Text write (#1443).
                        P::NUMBER_OF_STATES => PropertyWriteCapability::Through(P::STATE_TEXT),
                        _ => row.write_capability,
                    };
                    assert_eq!(row.write_capability, capability, "{what} {p:?}");
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
                    .chain(profile)
                    .chain([P::FEEDBACK_VALUE, P::ALARM_VALUES, P::FAULT_VALUES])
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
