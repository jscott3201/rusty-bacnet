//! The multi-state objects' property rows, in the order their
//! Property_List keeps (the legacy list projection).
//!
//! Base conformance is independent of implemented writability. Commandable
//! and intrinsic rows retain their optional base code; the intrinsic ones
//! carry, as their condition, the footnote of Tables 12-21, 12-22 and
//! 12-23 that requires them of an object reporting intrinsically
//! (IntrinsicReportingRequired), or the one that only permits them
//! (IntrinsicReportingOptional: Event_Message_Texts and Time_Delay_Normal;
//! #1485). The intrinsic rows stay while detection is off.

use bacnet_types::enums::PropertyIdentifier as P;

use crate::io_core::{
    COMMANDABLE_ROWS, COMMANDED_ROWS, DETECTOR_ROWS, EVENT_HISTORY_ROWS, IDENTITY_ROWS,
    OUT_OF_SERVICE_ROW, RELIABILITY_ROWS, VALUE_SOURCE_ROWS, intrinsic,
};
use crate::property_metadata::{
    PROPERTY_LIST_ROW,
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, Through, WhenOutOfService},
    joined_rows,
};

/// Out_Of_Service and Number_Of_States, which has no write of its own:
/// State_Text written whole or at index 0 sets it (#1443).
const STATE_COUNT_ROWS: &[PropertyMetadata] = &[
    OUT_OF_SERVICE_ROW,
    PropertyMetadata::new(
        P::NUMBER_OF_STATES,
        RequiredRead,
        None,
        Through(P::STATE_TEXT),
    ),
];

/// State_Text: element writes, and whole or index-0 writes that set
/// Number_Of_States.
const STATE_TEXT_ROW: PropertyMetadata =
    PropertyMetadata::new(P::STATE_TEXT, Optional, None, Always);

/// Alarm_Values, CHANGE_OF_STATE's parameter on the Input and the Value.
const ALARM_VALUES_ROW: PropertyMetadata = intrinsic(P::ALARM_VALUES);

/// Table 12-21: Present_Value takes writes only out of service.
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
    STATE_COUNT_ROWS,
    RELIABILITY_ROWS,
    &[STATE_TEXT_ROW, ALARM_VALUES_ROW, PROPERTY_LIST_ROW],
];

/// Table 12-22: a Multi-state Output is always commandable, and
/// COMMAND_FAILURE compares its Feedback_Value, whose write route stays
/// apart from command resolution and the state-set bounds.
pub(super) const OUTPUT: &[PropertyMetadata] = joined_rows![
    IDENTITY_ROWS,
    &[
        PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
        intrinsic(P::FEEDBACK_VALUE),
    ],
    DETECTOR_ROWS,
    EVENT_HISTORY_ROWS,
    STATE_COUNT_ROWS,
    COMMANDED_ROWS,
    RELIABILITY_ROWS,
    &[STATE_TEXT_ROW],
    VALUE_SOURCE_ROWS,
    &[PROPERTY_LIST_ROW],
];

/// Table 12-23, for a commandable Multi-state Value; another Present_Value
/// access projects it
/// ([`PresentValueAccess::project`](crate::present_value_access::PresentValueAccess)).
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
    STATE_COUNT_ROWS,
    COMMANDABLE_ROWS,
    RELIABILITY_ROWS,
    &[STATE_TEXT_ROW, ALARM_VALUES_ROW],
    VALUE_SOURCE_ROWS,
    &[PROPERTY_LIST_ROW],
];
