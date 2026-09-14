use super::EventLogObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::log_buffer::{BUFFER_SIZE_METADATA, LOG_BUFFER_METADATA, TOTAL_RECORD_COUNT_METADATA};
use crate::log_lifecycle::{LOG_ENABLE_METADATA, RECORD_COUNT_METADATA, STOP_WHEN_FULL_METADATA};
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Preserve legacy order. Log_Interval and Out_Of_Service are implemented
// compatibility rows, not additional required Event Log properties.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    LOG_ENABLE_METADATA,
    PropertyMetadata::new(P::LOG_INTERVAL, Optional, None, Always),
    STOP_WHEN_FULL_METADATA,
    BUFFER_SIZE_METADATA,
    LOG_BUFFER_METADATA,
    RECORD_COUNT_METADATA,
    TOTAL_RECORD_COUNT_METADATA,
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &EventLogObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}
