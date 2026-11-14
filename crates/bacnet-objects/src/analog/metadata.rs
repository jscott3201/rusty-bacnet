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

use crate::io_core::{EVENT_HISTORY_ROWS, IDENTITY_ROWS, RELIABILITY_ROWS};
use crate::property_metadata::{
    COV_INCREMENT_ROW, PROPERTY_LIST_ROW,
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyPresenceCondition::{
        Commandable, CommandableValueSourceTracking, IntrinsicReportingOptional,
        IntrinsicReportingRequired, ValueSourceTracking,
    },
    PropertyWriteCapability::{Always, ReadOnly, WhenCommandOwner, WhenOutOfService},
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
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
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

/// Value_Source, which its tracking makes present, and the two rows a
/// commandable object's tracking adds.
const VALUE_SOURCE_ROWS: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        P::VALUE_SOURCE,
        Optional,
        Some(ValueSourceTracking),
        WhenCommandOwner,
    ),
    PropertyMetadata::new(
        P::VALUE_SOURCE_ARRAY,
        Optional,
        Some(CommandableValueSourceTracking),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::LAST_COMMAND_TIME,
        Optional,
        Some(CommandableValueSourceTracking),
        ReadOnly,
    ),
];

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
    &[
        PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, ReadOnly),
        PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
        PropertyMetadata::new(P::CURRENT_COMMAND_PRIORITY, RequiredRead, None, ReadOnly),
        COV_INCREMENT_ROW,
    ],
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
    &[
        PropertyMetadata::new(P::PRIORITY_ARRAY, Optional, Some(Commandable), ReadOnly),
        PropertyMetadata::new(P::RELINQUISH_DEFAULT, Optional, Some(Commandable), Always),
        PropertyMetadata::new(
            P::CURRENT_COMMAND_PRIORITY,
            Optional,
            Some(Commandable),
            ReadOnly,
        ),
        COV_INCREMENT_ROW,
    ],
    OUT_OF_RANGE_ROWS,
    RELIABILITY_ROWS,
    EVENT_HISTORY_ROWS,
    VALUE_SOURCE_ROWS,
    &[PROPERTY_LIST_ROW],
];
