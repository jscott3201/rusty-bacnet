use super::PulseConverterObject;
use alloc::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Canonical effective rows for the Pulse Converter (type 24, ASHRAE 135-2020
// §12.23 Table 12-27; printed pp. 301-307 / PDF pp. 303-309).
// Order preserves the legacy projection, with the count rows #1092 added
// (Count, Update_Time, Count_Change_Time, Count_Before_Change) after
// Adjust_Value and COV_Period after COV_Increment, as the table orders them;
// PROPERTY_LIST is appended so the projection helper omits it while
// required_properties keeps it. Only implemented rows are described: the
// table rows the object does not serve (intrinsic-reporting, event-message,
// Reliability_Evaluation_Inhibit, audit, tag and profile rows) are all
// optional and stay absent until dispatch exists.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly; a rename falls
// through to WRITE_ACCESS_DENIED (the object has no write_object_name arm).
// Description carries the table O code with a routed CharacterString arm, so
// Optional/Always. Out_Of_Service carries the table R code with the routed
// Boolean arm, so RequiredRead/Always.
// Present_Value carries the table R code with footnote 1, and §12.23 requires
// it to accept writes while Out_Of_Service is TRUE; dispatch gates it behind
// Out_Of_Service (in-service writes are denied before value validation), so
// RequiredRead/WhenOutOfService. In service it reads as Count times
// Scale_Factor. Adjust_Value carries the table W code with a routed Real arm
// that adjusts Count (§12.23.13), so RequiredWrite/Always. Scale_Factor is
// table R with an arm (RequiredRead/Always); Units, Status_Flags and
// Event_State are table R without one (RequiredRead/ReadOnly).
// Count, Update_Time, Count_Change_Time and Count_Before_Change are table R
// and read-only by their descriptions (§12.23.14-§12.23.17): Count changes
// through `add_pulses` and Adjust_Value writes, the other three only as side
// effects of those, so RequiredRead/ReadOnly.
// Input_Reference and COV_Increment are table O with arms (Optional/Always).
// COV_Increment and COV_Period are footnote 2 rows, present because the
// object reports COV (supports_cov and cov_increment). COV_Period is served
// as a constant 0 with no write arm (Optional/ReadOnly): zero means no
// periodic notifications (Clause 13.1), which the server does not send.
// Reliability is table O, and since it can report the Input_Reference
// CONFIGURATION_ERROR its arm takes a client's value while Out_Of_Service is
// TRUE (Clause 12.23.10), so Optional/WhenOutOfService.
// Presence is None throughout: the implementation models no commandable,
// intrinsic-reporting, or paired-text gating. The object is not createable at
// runtime (the network factory builds only the eight analog/binary/
// multi-state input/output/value types) and remains deleteable (delete denies
// only Device and NetworkPort); neither needs an override. Array gating keeps
// the default: Property_List admits an index (BACnetARRAY per Table 12-27)
// while every other served row rejects one. COV keeps its overrides:
// supports_cov=true plus cov_increment()=Some, and the COV gating path
// (read_property plus supports_cov_property to supports_cov) never consults
// metadata.
const PULSE_CONVERTER_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::UNITS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::SCALE_FACTOR, RequiredRead, None, Always),
    PropertyMetadata::new(P::ADJUST_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::COUNT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::UPDATE_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COUNT_CHANGE_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COUNT_BEFORE_CHANGE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::COV_INCREMENT, Optional, None, Always),
    PropertyMetadata::new(P::COV_PERIOD, Optional, None, ReadOnly),
    PropertyMetadata::new(P::INPUT_REFERENCE, Optional, None, Always),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_pulse_converter_object(
    _object: &PulseConverterObject,
) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(PULSE_CONVERTER_BASE)
}

#[cfg(test)]
#[path = "metadata_tests.rs"]
mod tests;
