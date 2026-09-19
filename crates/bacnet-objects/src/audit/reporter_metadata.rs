use bacnet_types::enums::PropertyIdentifier;
use std::borrow::Cow;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

pub(super) static AUDIT_REPORTER_PROPERTIES: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_NAME,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_TYPE,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(PropertyIdentifier::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::STATUS_FLAGS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::RELIABILITY,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_STATE,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::AUDIT_LEVEL,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::AUDIT_SOURCE_REPORTER,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::AUDITABLE_OPERATIONS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::AUDIT_PRIORITY_FILTER,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PROPERTY_LIST,
        RequiredRead,
        None,
        ReadOnly,
    ),
];

pub(super) fn effective_properties(monitored_objects: bool) -> Cow<'static, [PropertyMetadata]> {
    if !monitored_objects {
        return Cow::Borrowed(AUDIT_REPORTER_PROPERTIES);
    }
    let mut rows = AUDIT_REPORTER_PROPERTIES.to_vec();
    // Only effective rows belong in metadata: absent is not an empty array.
    rows.insert(
        rows.len() - 1,
        PropertyMetadata::new(
            PropertyIdentifier::MONITORED_OBJECTS,
            Optional,
            None,
            ReadOnly,
        ),
    );
    Cow::Owned(rows)
}
