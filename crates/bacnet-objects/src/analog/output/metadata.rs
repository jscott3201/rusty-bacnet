use super::AnalogOutputObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyPresenceCondition::IntrinsicReporting,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Analog Output is always commandable. Base conformance remains independent
// of implemented writability; order preserves the legacy list projection.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::EVENT_DETECTION_ENABLE,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::UNITS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::CURRENT_COMMAND_PRIORITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COV_INCREMENT, Optional, None, Always),
    PropertyMetadata::new(P::HIGH_LIMIT, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::LOW_LIMIT, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::DEADBAND, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::LIMIT_ENABLE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::EVENT_ENABLE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(P::NOTIFY_TYPE, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::NOTIFICATION_CLASS,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::TIME_DELAY, Optional, Some(IntrinsicReporting), Always),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReporting),
        Always,
    ),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::RELIABILITY_EVALUATION_INHIBIT, Optional, None, Always),
    PropertyMetadata::new(
        P::ACKED_TRANSITIONS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_TIME_STAMPS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS,
        Optional,
        Some(IntrinsicReporting),
        ReadOnly,
    ),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(object: &AnalogOutputObject) -> Cow<'_, [PropertyMetadata]> {
    let mut rows = Cow::Borrowed(BASE);
    // Only independently configured engineering bounds: AO has no fault limits.
    if object.min_pres_value.is_some() {
        rows.to_mut().push(PropertyMetadata::new(
            P::MIN_PRES_VALUE,
            Optional,
            None,
            ReadOnly,
        ));
    }
    if object.max_pres_value.is_some() {
        rows.to_mut().push(PropertyMetadata::new(
            P::MAX_PRES_VALUE,
            Optional,
            None,
            ReadOnly,
        ));
    }
    rows
}
