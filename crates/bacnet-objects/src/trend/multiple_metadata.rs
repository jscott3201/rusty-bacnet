use super::TrendLogMultipleObject;
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::log_buffer::{BUFFER_SIZE_METADATA, LOG_BUFFER_METADATA, TOTAL_RECORD_COUNT_METADATA};
use crate::log_lifecycle::{LOG_ENABLE_METADATA, RECORD_COUNT_METADATA, STOP_WHEN_FULL_METADATA};
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Unlike single-channel Trend Log, the interval and monitored-reference rows
// have a required base classification. Keep their existing write routes and
// the read-only Out_Of_Service compatibility row unchanged.
const BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    LOG_ENABLE_METADATA,
    PropertyMetadata::new(P::LOG_INTERVAL, RequiredRead, None, Always),
    STOP_WHEN_FULL_METADATA,
    BUFFER_SIZE_METADATA,
    LOG_BUFFER_METADATA,
    RECORD_COUNT_METADATA,
    TOTAL_RECORD_COUNT_METADATA,
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, Optional, None, ReadOnly),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
    PropertyMetadata::new(P::LOGGING_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LOG_DEVICE_OBJECT_PROPERTY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_object(_object: &TrendLogMultipleObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(BASE)
}
