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
    // Table 12-14 lists the message texts and the reporting options as plain
    // optional rows, with no intrinsic-reporting footnote; the shared
    // `ReportingOptions` serves them (#1507). Its footnote 1 asks for
    // Event_Algorithm_Inhibit wherever the reference row is present.
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        Optional,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
        Optional,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
        Optional,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
        Optional,
        None,
        Always,
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
    // Table 12-14 has no Out_Of_Service, so there is no such row (#1064).
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
    PropertyMetadata::new(
        PropertyIdentifier::NOTIFICATION_CLASS,
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
    PropertyMetadata::new(PropertyIdentifier::NOTIFY_TYPE, RequiredRead, None, Always),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_TIME_STAMPS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    // Table 12-61 lists the message texts and the reporting options as plain
    // optional rows, as Table 12-14 does; the shared `ReportingOptions`
    // serves them here too (#1865).
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS,
        Optional,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_MESSAGE_TEXTS_CONFIG,
        Optional,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT_REF,
        Optional,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::EVENT_ALGORITHM_INHIBIT,
        Optional,
        None,
        Always,
    ),
    PropertyMetadata::new(
        PropertyIdentifier::PROPERTY_LIST,
        RequiredRead,
        None,
        ReadOnly,
    ),
];
