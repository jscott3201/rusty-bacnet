use std::borrow::Cow;
use std::collections::HashSet;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use crate::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use crate::multistate::{MultiStateInputObject, MultiStateOutputObject, MultiStateValueObject};
use crate::property_metadata::{
    PropertyConformance, PropertyMetadata, PropertyPresenceCondition, PropertyWriteCapability,
};
use crate::traits::BACnetObject;
use crate::value_types::{DateValueObject, TimeValueObject};

mod analog;
mod intrinsic;

/// Whether `property` reads the way its metadata row promises: with a value,
/// or, for a log's Log_Buffer, with the PROPERTY / READ_ACCESS_DENIED that
/// marks a present row only ReadRange (and AuditLogQuery) serve
/// (Clauses 12.25.14, 12.27.13, 12.30.19 and 12.64.10).
pub(crate) fn metadata_row_reads(object: &dyn BACnetObject, property: PropertyIdentifier) -> bool {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let result = object.read_property(property, None);
    if matches!(
        object.object_identifier().object_type(),
        ObjectType::AUDIT_LOG
            | ObjectType::EVENT_LOG
            | ObjectType::TREND_LOG
            | ObjectType::TREND_LOG_MULTIPLE
    ) && property == PropertyIdentifier::LOG_BUFFER
    {
        return matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::READ_ACCESS_DENIED);
    }
    result.is_ok()
}

struct InstanceMetadataObject {
    oid: ObjectIdentifier,
    include_description: bool,
}

impl InstanceMetadataObject {
    fn new(instance: u32, include_description: bool) -> Self {
        Self {
            oid: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
            include_description,
        }
    }
}

impl BACnetObject for InstanceMetadataObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "instance-metadata"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.object_name().into()))
            }
            PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                self.object_identifier().object_type().to_raw(),
            )),
            PropertyIdentifier::DESCRIPTION if self.include_description => {
                Ok(PropertyValue::CharacterString(String::new()))
            }
            PropertyIdentifier::PROPERTY_LIST => {
                crate::common::read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(crate::common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(crate::common::write_access_denied_error())
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        let mut rows = vec![
            PropertyMetadata::new(
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
            PropertyMetadata::new(
                PropertyIdentifier::OBJECT_NAME,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
            PropertyMetadata::new(
                PropertyIdentifier::OBJECT_TYPE,
                PropertyConformance::RequiredRead,
                None,
                PropertyWriteCapability::ReadOnly,
            ),
        ];
        if self.include_description {
            rows.push(PropertyMetadata::new(
                PropertyIdentifier::DESCRIPTION,
                PropertyConformance::Optional,
                None,
                PropertyWriteCapability::ReadOnly,
            ));
        }
        rows.push(PropertyMetadata::new(
            PropertyIdentifier::PROPERTY_LIST,
            PropertyConformance::RequiredRead,
            None,
            PropertyWriteCapability::ReadOnly,
        ));
        Cow::Owned(rows)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let metadata = self.property_metadata();
        crate::property_metadata::property_list_from_metadata(metadata.as_ref())
    }
}

fn metadata_row(
    object: &dyn BACnetObject,
    property_identifier: PropertyIdentifier,
) -> PropertyMetadata {
    *object
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == property_identifier)
        .expect("property should have a metadata row")
}

fn assert_unique_and_canonical(object: &dyn BACnetObject) {
    let metadata = object.property_metadata();
    assert!(!metadata.is_empty());
    assert!(
        metadata
            .iter()
            .any(|row| row.property_identifier == PropertyIdentifier::PROPERTY_LIST)
    );
    let unique: HashSet<_> = metadata.iter().map(|row| row.property_identifier).collect();
    assert_eq!(
        unique.len(),
        metadata.len(),
        "duplicate metadata identifier"
    );
}

#[test]
fn property_metadata_contract_time_value() {
    let object = TimeValueObject::new(1, "TV-1").unwrap();
    assert_unique_and_canonical(&object);
    assert_eq!(object.property_metadata().len(), 12);

    let present_value = metadata_row(&object, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(present_value.conformance, PropertyConformance::RequiredRead);
    assert_eq!(present_value.presence_condition, None);
    assert_eq!(
        present_value.write_capability,
        PropertyWriteCapability::Always
    );

    let priority_array = metadata_row(&object, PropertyIdentifier::PRIORITY_ARRAY);
    assert_eq!(priority_array.conformance, PropertyConformance::Optional);
    assert_eq!(
        priority_array.presence_condition,
        Some(PropertyPresenceCondition::Commandable)
    );
    assert_eq!(
        priority_array.write_capability,
        PropertyWriteCapability::ReadOnly
    );

    let status_flags = metadata_row(&object, PropertyIdentifier::STATUS_FLAGS);
    assert_eq!(status_flags.conformance, PropertyConformance::RequiredRead);
    assert_eq!(
        status_flags.write_capability,
        PropertyWriteCapability::ReadOnly
    );
}

#[test]
fn property_metadata_contract_binary_input() {
    let object = BinaryInputObject::new(1, "BI-1").unwrap();
    assert_unique_and_canonical(&object);
    assert_eq!(object.property_metadata().len(), 27);

    let present_value = metadata_row(&object, PropertyIdentifier::PRESENT_VALUE);
    assert_eq!(present_value.conformance, PropertyConformance::RequiredRead);
    assert_eq!(
        present_value.write_capability,
        PropertyWriteCapability::WhenOutOfService
    );

    let reliability = metadata_row(&object, PropertyIdentifier::RELIABILITY);
    assert_eq!(reliability.conformance, PropertyConformance::Optional);
    assert_eq!(
        reliability.write_capability,
        PropertyWriteCapability::WhenOutOfService
    );

    let inhibit = metadata_row(&object, PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT);
    assert_eq!(inhibit.conformance, PropertyConformance::Optional);
    assert_eq!(inhibit.presence_condition, None);
    assert_eq!(inhibit.write_capability, PropertyWriteCapability::Always);

    let event_enable = metadata_row(&object, PropertyIdentifier::EVENT_ENABLE);
    assert_eq!(event_enable.conformance, PropertyConformance::Optional);
    assert_eq!(
        event_enable.presence_condition,
        Some(PropertyPresenceCondition::IntrinsicReportingRequired)
    );
    assert_eq!(
        event_enable.write_capability,
        PropertyWriteCapability::Always
    );

    for property_identifier in [
        PropertyIdentifier::ACTIVE_TEXT,
        PropertyIdentifier::INACTIVE_TEXT,
    ] {
        assert_eq!(
            metadata_row(&object, property_identifier).presence_condition,
            Some(PropertyPresenceCondition::PairedText)
        );
    }
}

/// The ASHRAE-assigned property identifiers the served-without-a-row sweep
/// tries: 0 to 511, and from 4,194,304 up to sixteen past the highest one
/// this stack names (Clause 23 leaves the range between to vendors). The
/// audit checks that every built-in row falls inside it.
pub(crate) fn swept_property_identifiers() -> impl Iterator<Item = PropertyIdentifier> {
    (0..512)
        .chain(4_194_304..=swept_upper_bound())
        .map(PropertyIdentifier::from_raw)
}

/// The last identifier of the sweep's upper range.
pub(crate) fn swept_upper_bound() -> u32 {
    PropertyIdentifier::ALL_NAMED
        .iter()
        .map(|(_, property)| property.to_raw())
        .max()
        .expect("named property identifiers")
        + 16
}

/// Whether [`swept_property_identifiers`] tries `property`.
pub(crate) fn is_swept(property: PropertyIdentifier) -> bool {
    swept_property_identifiers().any(|swept| swept == property)
}

/// The other half of the audit's readability check: a property the object
/// serves, read without an array index, has a row. Every identifier of
/// [`swept_property_identifiers`] is tried on one object of each catalogue
/// type; a property without a row must answer PROPERTY / UNKNOWN_PROPERTY.
#[test]
fn property_metadata_contract_every_served_property_has_a_row() {
    use bacnet_types::enums::{ErrorClass, ErrorCode};
    let mut objects = crate::catalogue::representatives();
    // Reach the Staging object's started state, as the audit does.
    for object in objects
        .iter_mut()
        .filter(|object| object.object_identifier().object_type() == ObjectType::STAGING)
    {
        object
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(5.0),
                None,
            )
            .unwrap();
    }
    let mut failures = Vec::new();
    let mut visited = std::collections::BTreeSet::new();
    for object in &objects {
        visited.insert(object.object_identifier().object_type().to_raw());
        let metadata = object.property_metadata();
        for property in swept_property_identifiers() {
            if matches!(
                property,
                PropertyIdentifier::ALL
                    | PropertyIdentifier::REQUIRED
                    | PropertyIdentifier::OPTIONAL
            ) || metadata
                .iter()
                .any(|row| row.property_identifier == property)
            {
                continue;
            }
            match object.read_property(property, None) {
                Err(Error::Remote(RemoteError::Error { class, code }))
                    if class == ErrorClass::PROPERTY && code == ErrorCode::UNKNOWN_PROPERTY => {}
                other => failures.push(format!(
                    "{:?} serves {property:?} without a row: {other:?}",
                    object.object_identifier().object_type()
                )),
            }
        }
    }
    assert_eq!(
        visited.len(),
        64,
        "the sweep must visit every catalogue type"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn property_metadata_contract_property_list_projection_excludes_property_list() {
    let cases: [(Box<dyn BACnetObject>, &[PropertyIdentifier]); 2] = [
        (
            Box::new(TimeValueObject::new(1, "TV-1").unwrap()),
            &[
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::OBJECT_TYPE,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyIdentifier::RELIABILITY,
                PropertyIdentifier::PRIORITY_ARRAY,
                PropertyIdentifier::RELINQUISH_DEFAULT,
                PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
            ],
        ),
        (
            Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
            &[
                PropertyIdentifier::OBJECT_IDENTIFIER,
                PropertyIdentifier::OBJECT_NAME,
                PropertyIdentifier::DESCRIPTION,
                PropertyIdentifier::OBJECT_TYPE,
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
                PropertyIdentifier::EVENT_STATE,
                PropertyIdentifier::EVENT_DETECTION_ENABLE,
                PropertyIdentifier::EVENT_ENABLE,
                PropertyIdentifier::TIME_DELAY,
                PropertyIdentifier::TIME_DELAY_NORMAL,
                PropertyIdentifier::NOTIFY_TYPE,
                PropertyIdentifier::NOTIFICATION_CLASS,
                PropertyIdentifier::ACKED_TRANSITIONS,
                PropertyIdentifier::EVENT_TIME_STAMPS,
                PropertyIdentifier::EVENT_MESSAGE_TEXTS,
                PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
                PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
                PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
                PropertyIdentifier::OUT_OF_SERVICE,
                PropertyIdentifier::POLARITY,
                PropertyIdentifier::RELIABILITY,
                PropertyIdentifier::RELIABILITY_EVALUATION_INHIBIT,
                PropertyIdentifier::ACTIVE_TEXT,
                PropertyIdentifier::INACTIVE_TEXT,
                PropertyIdentifier::ALARM_VALUE,
            ],
        ),
    ];

    for (object, expected_projection) in cases {
        let projected: Vec<_> = object
            .property_metadata()
            .iter()
            .filter_map(|row| {
                (row.property_identifier != PropertyIdentifier::PROPERTY_LIST)
                    .then_some(row.property_identifier)
            })
            .collect();
        assert_eq!(projected, expected_projection);
        assert_eq!(object.property_list().as_ref(), expected_projection);
        assert!(
            !object
                .property_list()
                .contains(&PropertyIdentifier::PROPERTY_LIST)
        );

        let wire_list = object
            .read_property(PropertyIdentifier::PROPERTY_LIST, None)
            .unwrap();
        let expected_wire = PropertyValue::List(
            expected_projection
                .iter()
                .filter(|property_identifier| {
                    !matches!(
                        **property_identifier,
                        PropertyIdentifier::OBJECT_IDENTIFIER
                            | PropertyIdentifier::OBJECT_NAME
                            | PropertyIdentifier::OBJECT_TYPE
                            | PropertyIdentifier::PROPERTY_LIST
                    )
                })
                .map(|property_identifier| PropertyValue::Enumerated(property_identifier.to_raw()))
                .collect(),
        );
        assert_eq!(wire_list, expected_wire);
    }
}

#[test]
fn property_metadata_contract_write_capabilities_match_dispatch() {
    let mut time_value = TimeValueObject::new(1, "TV-1").unwrap();
    assert_eq!(
        metadata_row(&time_value, PropertyIdentifier::OBJECT_NAME).write_capability,
        PropertyWriteCapability::Always
    );
    assert!(
        time_value
            .write_property(
                PropertyIdentifier::OBJECT_NAME,
                None,
                PropertyValue::CharacterString("TV-2".into()),
                None,
            )
            .is_ok()
    );
    assert_eq!(
        metadata_row(&time_value, PropertyIdentifier::STATUS_FLAGS).write_capability,
        PropertyWriteCapability::ReadOnly
    );
    assert!(
        time_value
            .write_property(
                PropertyIdentifier::STATUS_FLAGS,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .is_err()
    );

    let mut binary_input = BinaryInputObject::new(1, "BI-1").unwrap();
    for property_identifier in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::RELIABILITY,
    ] {
        assert_eq!(
            metadata_row(&binary_input, property_identifier).write_capability,
            PropertyWriteCapability::WhenOutOfService
        );
    }
    assert!(
        binary_input
            .write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(1),
                None,
                &crate::command_source::test_origin(),
            )
            .is_err()
    );
    assert!(
        binary_input
            .write_property(
                PropertyIdentifier::RELIABILITY,
                None,
                PropertyValue::Enumerated(1),
                None,
            )
            .is_err()
    );
    binary_input
        .write_property(
            PropertyIdentifier::OUT_OF_SERVICE,
            None,
            PropertyValue::Boolean(true),
            None,
        )
        .unwrap();
    binary_input
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            None,
            &crate::command_source::test_origin(),
        )
        .unwrap();
    binary_input
        .write_property(
            PropertyIdentifier::RELIABILITY,
            None,
            PropertyValue::Enumerated(1),
            None,
        )
        .unwrap();
}

#[test]
fn property_metadata_contract_models_required_write_code() {
    let row = PropertyMetadata::new(
        PropertyIdentifier::PRESENT_VALUE,
        PropertyConformance::RequiredWrite,
        None,
        PropertyWriteCapability::Always,
    );
    assert!(row.conformance.is_required());
    assert!(row.write_capability.is_writable());
}

#[test]
fn property_metadata_contract_dyn_object_can_return_owned_instance_rows() {
    let without_description = InstanceMetadataObject::new(1, false);
    let with_description = InstanceMetadataObject::new(2, true);
    let cases: [(&dyn BACnetObject, bool); 2] =
        [(&without_description, false), (&with_description, true)];

    for (object, expect_description) in cases {
        let metadata = object.property_metadata();
        assert!(matches!(&metadata, Cow::Owned(_)));
        assert_eq!(
            metadata
                .iter()
                .any(|row| row.property_identifier == PropertyIdentifier::DESCRIPTION),
            expect_description
        );
    }
}

#[test]
fn property_metadata_migrated_date_value_exact_required_set() {
    use PropertyIdentifier as P;

    let object = DateValueObject::new(1, "DV-1").unwrap();
    let metadata = object.property_metadata();
    assert!(matches!(metadata, Cow::Borrowed(_)));
    assert_eq!(metadata.len(), 12);
    let required = object.required_properties();
    assert_eq!(
        required.as_ref(),
        [
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
            P::STATUS_FLAGS,
            P::PROPERTY_LIST
        ]
    );
}

#[test]
fn property_metadata_binary_commandable_exact_required_sets() {
    use PropertyIdentifier as P;

    let objects: [Box<dyn BACnetObject>; 2] = [
        Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
    ];
    // Tables 12-8 and 12-10 require these of an object that reports
    // intrinsically (#1485), in metadata order.
    let intrinsic = [
        P::EVENT_DETECTION_ENABLE,
        P::EVENT_ENABLE,
        P::TIME_DELAY,
        P::NOTIFY_TYPE,
        P::NOTIFICATION_CLASS,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
    ];
    for object in objects {
        let output = object.object_identifier().object_type() == ObjectType::BINARY_OUTPUT;
        let mut required = vec![
            P::OBJECT_IDENTIFIER,
            P::OBJECT_NAME,
            P::OBJECT_TYPE,
            P::PRESENT_VALUE,
        ];
        if output {
            // Table 12-8 footnote 4: the COMMAND_FAILURE feedback.
            required.push(P::FEEDBACK_VALUE);
        }
        required.extend([P::STATUS_FLAGS, P::EVENT_STATE]);
        required.extend(intrinsic);
        required.push(P::OUT_OF_SERVICE);
        if output {
            required.extend([
                P::PRIORITY_ARRAY,
                P::RELINQUISH_DEFAULT,
                P::CURRENT_COMMAND_PRIORITY,
                P::POLARITY,
            ]);
        } else {
            // Table 12-10 footnote 6: the CHANGE_OF_STATE alarm value.
            required.push(P::ALARM_VALUE);
        }
        required.extend([P::VALUE_SOURCE, P::VALUE_SOURCE_ARRAY, P::LAST_COMMAND_TIME]);
        required.push(P::PROPERTY_LIST);
        assert_unique_and_canonical(object.as_ref());
        assert!(matches!(object.property_metadata(), Cow::Borrowed(_)));
        assert_eq!(object.required_properties().as_ref(), required);
        assert!(object.is_createable());
    }
}
