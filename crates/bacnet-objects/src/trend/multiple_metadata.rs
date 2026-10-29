use super::TrendLogMultipleObject;
use alloc::borrow::Cow;

use bacnet_types::enums::{LoggingType, PropertyIdentifier as P};

use crate::log_buffer::{BUFFER_SIZE_METADATA, LOG_BUFFER_METADATA, TOTAL_RECORD_COUNT_METADATA};
use crate::log_lifecycle::{LOG_ENABLE_METADATA, RECORD_COUNT_METADATA, STOP_WHEN_FULL_METADATA};
use crate::log_reporting::BUFFER_READY_METADATA;
use crate::log_window::{START_TIME_METADATA, STOP_TIME_METADATA};
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata, PropertyWriteCapability,
    PropertyWriteCapability::{Always, ReadOnly},
};

// Unlike single-channel Trend Log, the interval and monitored-reference rows
// have a required base classification. Table 12-35 defines no Out_Of_Service,
// so there is no such row (#985). Log_DeviceObjectProperty is a writable
// array held to this device (#1234). Logging_Type takes POLLED or TRIGGERED;
// Log_Interval is writable while POLLED and read-only while TRIGGERED
// (footnote 2). Start_Time and Stop_Time are writable as footnote 1 asks;
// the device supports clock-aligned logging, so Align_Intervals and
// Interval_Offset are present (footnote 3), writable like Log_Interval, and
// Trigger is writable to ask for an acquisition. No footnote of Table 12-35
// requires the window the way Table 12-29's does a Trend Log's (#1481). The
// log reports BUFFER_READY (#1347), so the intrinsic reporting rows footnote
// 4 asks for come last, each marked as present for that reason.
const fn rows(log_interval: PropertyWriteCapability) -> [PropertyMetadata; 35] {
    let r = BUFFER_READY_METADATA;
    [
        PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
        PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
        LOG_ENABLE_METADATA,
        PropertyMetadata::new(P::LOG_INTERVAL, RequiredRead, None, log_interval),
        STOP_WHEN_FULL_METADATA,
        BUFFER_SIZE_METADATA,
        LOG_BUFFER_METADATA,
        RECORD_COUNT_METADATA,
        TOTAL_RECORD_COUNT_METADATA,
        PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
        PropertyMetadata::new(P::LOGGING_TYPE, RequiredRead, None, Always),
        PropertyMetadata::new(P::LOG_DEVICE_OBJECT_PROPERTY, RequiredRead, None, Always),
        START_TIME_METADATA,
        STOP_TIME_METADATA,
        PropertyMetadata::new(P::ALIGN_INTERVALS, Optional, None, Always),
        PropertyMetadata::new(P::INTERVAL_OFFSET, Optional, None, Always),
        PropertyMetadata::new(P::TRIGGER, Optional, None, Always),
        r[0],
        r[1],
        r[2],
        r[3],
        r[4],
        r[5],
        r[6],
        r[7],
        r[8],
        r[9],
        r[10],
        r[11],
        r[12],
        PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
    ]
}

const POLLED: [PropertyMetadata; 35] = rows(Always);
const TRIGGERED: [PropertyMetadata; 35] = rows(ReadOnly);

pub(super) fn for_object(object: &TrendLogMultipleObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(if object.logging_type() == LoggingType::TRIGGERED {
        &TRIGGERED
    } else {
        &POLLED
    })
}
