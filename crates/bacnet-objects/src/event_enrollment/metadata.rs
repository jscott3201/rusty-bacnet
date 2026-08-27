use crate::property_metadata::{PropertyConformance, PropertyMetadata, PropertyWriteCapability};
use bacnet_types::enums::PropertyIdentifier;

use PropertyConformance::{Optional, RequiredRead};
use PropertyWriteCapability::{Always, ReadOnly};

pub(super) static EVENT_ENROLLMENT_PROPERTIES: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_IDENTIFIER,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(PropertyIdentifier::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(PropertyIdentifier::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_TYPE,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(PropertyIdentifier::EVENT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(PropertyIdentifier::NOTIFY_TYPE, RequiredRead, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_PARAMETERS,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_PROPERTY_REFERENCE,
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
    PropertyMetadata::new(PropertyIdentifier::EVENT_ENABLE, RequiredRead, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::ACKED_TRANSITIONS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::NOTIFICATION_CLASS,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_TIME_STAMPS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    // Table 12-14 marks these O2, whose condition is not yet representable in
    // the shared metadata enum; keep the implemented rows optional.
    PropertyMetadata::new(PropertyIdentifier::FAULT_TYPE, Optional, None, ReadOnly),
    PropertyMetadata::new(PropertyIdentifier::FAULT_PARAMETERS, Optional, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::TIME_DELAY_NORMAL,
        Optional,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::STATUS_FLAGS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    // Out_Of_Service is an existing compatibility projection, not a Table
    // 12-14 row. Retain it so metadata remains complete for the readable API.
    PropertyMetadata::new(PropertyIdentifier::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::RELIABILITY,
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

pub(super) static ALERT_ENROLLMENT_PROPERTIES: &[PropertyMetadata] = &[
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
    PropertyMetadata::new(PropertyIdentifier::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::OBJECT_TYPE,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PRESENT_VALUE,
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
        PropertyIdentifier::EVENT_DETECTION_ENABLE,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(PropertyIdentifier::EVENT_ENABLE, RequiredRead, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::ACKED_TRANSITIONS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::NOTIFICATION_CLASS,
        RequiredRead,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_TIME_STAMPS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    // These three are existing compatibility projections, not Table 12-61
    // rows. Retain them as optional so metadata covers the readable API.
    PropertyMetadata::new(PropertyIdentifier::STATUS_FLAGS, Optional, None, ReadOnly),
    PropertyMetadata::new(PropertyIdentifier::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(PropertyIdentifier::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(
        PropertyIdentifier::PROPERTY_LIST,
        RequiredRead,
        None,
        ReadOnly,
    ),
];
