//! The binary objects' property rows, in the order their Property_List
//! keeps (the legacy list projection).
//!
//! Base conformance is independent of implemented writability. Commandable
//! and intrinsic rows retain their optional base code; the intrinsic ones
//! carry, as their condition, the footnote of Tables 12-6, 12-8 and 12-10
//! that requires them of an object reporting intrinsically
//! (IntrinsicReportingRequired), or the one that only permits them
//! (IntrinsicReportingOptional: Event_Message_Texts and Time_Delay_Normal;
//! #1485).

use bacnet_types::enums::PropertyIdentifier as P;

use crate::io_core::{
    COMMANDABLE_ROWS, COMMANDED_ROWS, EVENT_HISTORY_ROWS, IDENTITY_ROWS, OUT_OF_SERVICE_ROW,
    RELIABILITY_ROWS, VALUE_SOURCE_ROWS,
};
use crate::property_metadata::{
    PROPERTY_LIST_ROW,
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyPresenceCondition::{
        IntrinsicReportingOptional, IntrinsicReportingRequired, PairedText,
    },
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
    joined_rows,
};

/// Status_Flags, Event_State and Event_Detection_Enable, then the rows
/// every detector has, in the order the binary tables keep them.
const DETECTOR_ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    intrinsic(P::EVENT_DETECTION_ENABLE),
    intrinsic(P::EVENT_ENABLE),
    intrinsic(P::TIME_DELAY),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
    intrinsic(P::NOTIFY_TYPE),
    intrinsic(P::NOTIFICATION_CLASS),
];

/// A writable row intrinsic reporting requires.
const fn intrinsic(property: P) -> PropertyMetadata {
    PropertyMetadata::new(property, Optional, Some(IntrinsicReportingRequired), Always)
}

/// Polarity, which the Input and the Output keep at normal.
const POLARITY_ROW: PropertyMetadata =
    PropertyMetadata::new(P::POLARITY, RequiredRead, None, ReadOnly);

/// Active_Text and Inactive_Text, which come as a pair.
const TEXT_ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::ACTIVE_TEXT, Optional, Some(PairedText), Always),
    PropertyMetadata::new(P::INACTIVE_TEXT, Optional, Some(PairedText), Always),
];

/// Alarm_Value, CHANGE_OF_STATE's one parameter on the Input and the Value.
const ALARM_VALUE_ROW: PropertyMetadata = intrinsic(P::ALARM_VALUE);

/// Table 12-6: Present_Value takes writes only out of service.
pub(super) const INPUT: &[PropertyMetadata] = joined_rows![
    IDENTITY_ROWS,
    &[PropertyMetadata::new(
        P::PRESENT_VALUE,
        RequiredRead,
        None,
        WhenOutOfService
    )],
    DETECTOR_ROWS,
    EVENT_HISTORY_ROWS,
    &[OUT_OF_SERVICE_ROW, POLARITY_ROW],
    RELIABILITY_ROWS,
    TEXT_ROWS,
    &[ALARM_VALUE_ROW, PROPERTY_LIST_ROW],
];

/// Table 12-8: a Binary Output is always commandable, and COMMAND_FAILURE
/// compares its Feedback_Value.
pub(super) const OUTPUT: &[PropertyMetadata] = joined_rows![
    IDENTITY_ROWS,
    &[
        PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
        intrinsic(P::FEEDBACK_VALUE),
    ],
    DETECTOR_ROWS,
    EVENT_HISTORY_ROWS,
    &[OUT_OF_SERVICE_ROW],
    COMMANDED_ROWS,
    &[POLARITY_ROW],
    RELIABILITY_ROWS,
    TEXT_ROWS,
    VALUE_SOURCE_ROWS,
    &[PROPERTY_LIST_ROW],
];

/// Table 12-10, for a commandable Binary Value; another Present_Value access
/// projects it ([`PresentValueAccess::project`](crate::present_value_access::PresentValueAccess)).
pub(super) const VALUE: &[PropertyMetadata] = joined_rows![
    IDENTITY_ROWS,
    &[PropertyMetadata::new(
        P::PRESENT_VALUE,
        RequiredRead,
        None,
        Always
    )],
    DETECTOR_ROWS,
    EVENT_HISTORY_ROWS,
    &[OUT_OF_SERVICE_ROW],
    COMMANDABLE_ROWS,
    RELIABILITY_ROWS,
    TEXT_ROWS,
    &[ALARM_VALUE_ROW],
    VALUE_SOURCE_ROWS,
    &[PROPERTY_LIST_ROW],
];
