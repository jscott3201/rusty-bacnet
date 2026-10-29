use super::EventLogObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::log_buffer::{BUFFER_SIZE_METADATA, LOG_BUFFER_METADATA, TOTAL_RECORD_COUNT_METADATA};
use crate::log_lifecycle::{LOG_ENABLE_METADATA, RECORD_COUNT_METADATA, STOP_WHEN_FULL_METADATA};
use crate::log_reporting::BUFFER_READY_METADATA;
use crate::log_window::{START_TIME_METADATA, STOP_TIME_METADATA};
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};

// The legacy rows keep their order, then the window and the intrinsic
// reporting rows follow. Table 12-31 defines neither Log_Interval nor
// Out_Of_Service, so there are no such rows (#1064). Start_Time and Stop_Time
// come together and are writable (footnotes 1 and 2); no footnote requires
// them. The log reports BUFFER_READY (#1347), so it has the rows footnote 3
// asks for, each marked as present for intrinsic reporting.
const BASE: [PropertyMetadata; 29] = {
    let r = BUFFER_READY_METADATA;
    [
        PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
        PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
        LOG_ENABLE_METADATA,
        STOP_WHEN_FULL_METADATA,
        BUFFER_SIZE_METADATA,
        LOG_BUFFER_METADATA,
        RECORD_COUNT_METADATA,
        TOTAL_RECORD_COUNT_METADATA,
        PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::RELIABILITY, Optional, None, ReadOnly),
        START_TIME_METADATA,
        STOP_TIME_METADATA,
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
};

pub(super) fn for_object(_object: &EventLogObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(&BASE)
}
