//! The analog objects' property rows, in the order their Property_List
//! keeps (the legacy list projection).
//!
//! Base conformance codes stay independent of the write routes implemented.
//! Intrinsic, commandable and COV rows keep their optional base code even
//! when a capability requires their presence, and carry the condition as
//! their presence condition. Footnote 3 of Tables 12-2 and 12-4 requires an
//! intrinsic row of an object that reports intrinsically
//! (IntrinsicReportingRequired); footnote 5 of Table 12-2 and footnote 6 of
//! Table 12-4 alone only permit one (IntrinsicReportingOptional:
//! Event_Message_Texts and Time_Delay_Normal; #1485).

use bacnet_types::enums::PropertyIdentifier as P;

use crate::io_core::{
    COMMANDABLE_ROWS, COMMANDED_ROWS, EVENT_HISTORY_ROWS, IDENTITY_ROWS, OUT_OF_SERVICE_ROW,
    RELIABILITY_ROWS, VALUE_SOURCE_ROWS,
};
use crate::property_metadata::{
    COV_INCREMENT_ROW, PROPERTY_LIST_ROW,
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyPresenceCondition::{IntrinsicReportingOptional, IntrinsicReportingRequired},
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
    joined_rows,
};

/// Status_Flags to Units, between Present_Value and the commandable rows.
const STATUS_ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::EVENT_DETECTION_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    OUT_OF_SERVICE_ROW,
    PropertyMetadata::new(P::UNITS, RequiredRead, None, ReadOnly),
];

/// OUT_OF_RANGE's parameters and the reporting rows every detector has.
const OUT_OF_RANGE_ROWS: &[PropertyMetadata] = &[
    intrinsic(P::HIGH_LIMIT),
    intrinsic(P::LOW_LIMIT),
    intrinsic(P::DEADBAND),
    intrinsic(P::LIMIT_ENABLE),
    intrinsic(P::EVENT_ENABLE),
    intrinsic(P::NOTIFY_TYPE),
    intrinsic(P::NOTIFICATION_CLASS),
    intrinsic(P::TIME_DELAY),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
];

/// A writable row intrinsic reporting requires.
const fn intrinsic(property: P) -> PropertyMetadata {
    PropertyMetadata::new(property, Optional, Some(IntrinsicReportingRequired), Always)
}

/// Table 12-2: Present_Value takes writes only out of service.
pub(super) const INPUT: &[PropertyMetadata] = joined_rows![
    IDENTITY_ROWS,
    &[PropertyMetadata::new(
        P::PRESENT_VALUE,
        RequiredRead,
        None,
        WhenOutOfService
    )],
    STATUS_ROWS,
    &[COV_INCREMENT_ROW],
    OUT_OF_RANGE_ROWS,
    RELIABILITY_ROWS,
    EVENT_HISTORY_ROWS,
    &[PROPERTY_LIST_ROW],
];

/// Table 12-3: an Analog Output is always commandable.
pub(super) const OUTPUT: &[PropertyMetadata] = joined_rows![
    IDENTITY_ROWS,
    &[PropertyMetadata::new(
        P::PRESENT_VALUE,
        RequiredWrite,
        None,
        Always
    )],
    STATUS_ROWS,
    COMMANDED_ROWS,
    &[COV_INCREMENT_ROW],
    OUT_OF_RANGE_ROWS,
    RELIABILITY_ROWS,
    EVENT_HISTORY_ROWS,
    VALUE_SOURCE_ROWS,
    &[PROPERTY_LIST_ROW],
];

/// Table 12-4, for a commandable Analog Value; another Present_Value access
/// projects it ([`PresentValueAccess::project`](crate::present_value_access::PresentValueAccess)).
pub(super) const VALUE: &[PropertyMetadata] = joined_rows![
    IDENTITY_ROWS,
    &[PropertyMetadata::new(
        P::PRESENT_VALUE,
        RequiredRead,
        None,
        Always
    )],
    STATUS_ROWS,
    COMMANDABLE_ROWS,
    &[COV_INCREMENT_ROW],
    OUT_OF_RANGE_ROWS,
    RELIABILITY_ROWS,
    EVENT_HISTORY_ROWS,
    VALUE_SOURCE_ROWS,
    &[PROPERTY_LIST_ROW],
];
