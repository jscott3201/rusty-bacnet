use super::{AccessDoorObject, AccessPointObject, AccessZoneObject};
use std::borrow::Cow;

use bacnet_types::enums::PropertyIdentifier as P;

use crate::event::options::REPORTING_OPTION_METADATA;
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead, RequiredWrite},
    PropertyMetadata,
    PropertyPresenceCondition::{IntrinsicReportingOptional, IntrinsicReportingRequired},
    PropertyWriteCapability::{Always, ReadOnly, WhenOutOfService},
};

// Canonical effective rows for the Access Topology trio (ASHRAE 135-2020; PDF = printed + 2):
// - Access Door (type 30, §12.26 Table 12-30; printed p. 325 / PDF p. 327)
// - Access Point (type 33, §12.31 Table 12-36; printed p. 365 / PDF p. 367)
// - Access Zone (type 36, §12.32 Table 12-37; printed p. 382 / PDF p. 384)
// Order preserves each legacy projection; Door Event_State, Priority_Array,
// and Relinquish_Default (readable but unlisted) are appended after the
// legacy rows (Lift FLOOR_NUMBER precedent) so the served projection gains
// exactly three rows, and Property_List is appended so the projection helper
// omits it while required_properties keeps it. Only implemented rows are
// described: table rows the objects do not serve (Door_Unlock_Delay_Time,
// Maintenance_Required, the point's event rows, audit/tag/profile rows)
// stay absent until dispatch exists. The door and the zone serve
// Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1329)
// after Event_Message_Texts. The required door rows #1073
// added follow Relinquish_Default: Door_Pulse_Time, Door_Extended_Pulse_Time
// and Door_Open_Too_Long_Time carry the table R code with routed Unsigned32
// arms, so RequiredRead/Always, and Current_Command_Priority, derived from
// the priority array, is RequiredRead/ReadOnly.
// Object_Identifier, Object_Name, and Object_Type carry the table R code and
// have no network write route, so RequiredRead/ReadOnly. Object_Name
// explicitly documents the denial: a rename falls through to
// WRITE_ACCESS_DENIED (no object has a write_object_name arm). Description
// carries the table O code with a routed CharacterString write arm, so
// Optional/Always. Out_Of_Service carries the table R code with the routed
// Boolean arm, so RequiredRead/Always.
// The door's event rows (#1149) are the zone's ten below, with the same
// codes, conditions and capabilities (Table 12-30 footnotes 3 and 5).
// Door_Alarm_State, the value the door's CHANGE_OF_STATE algorithm watches,
// also carries footnote 3, so intrinsic reporting requires it too (#1485).
// Fault_Values and
// Masked_Alarm_Values carry the plain O code and routed list arms, so
// Optional/Always with no presence reason. The door's Reliability, the
// table R code, is writable only out of service, since the FAULT_STATE
// check can move it off NO_FAULT_DETECTED (Clause 12.26.9), so
// RequiredRead/WhenOutOfService.
// Door Present_Value carries the table W code (commandable) with the
// priority-slot write arm, so RequiredWrite/Always. Relinquish_Default
// carries the table R code with the LOCK/UNLOCK setter arm, so
// RequiredRead/Always. Priority_Array and Event_State are served readable
// rows with no write arm, so RequiredRead/ReadOnly. Door_Status, Lock_Status
// and Door_Alarm_State carry the table O code with footnote 1, writable while
// Out_Of_Service is TRUE, and dispatch takes their writes only then (#1131),
// so Optional/WhenOutOfService. Secured_Status and Door_Members carry the
// table O code with no write arm, so Optional/ReadOnly.
// Table 12-36 has no Present_Value row, so the point serves none (#1064
// removed the implementation-extra row the 0.1.0 import carried).
// Access_Event, Access_Event_Tag, Access_Event_Time, Access_Doors, and
// Event_State carry the table R code with no write arm, so
// RequiredRead/ReadOnly; an Out_Of_Service edge moves the three event rows
// (#1248) without opening them to writes. Authentication_Status and
// Access_Event_Credential, appended for #1284, carry the table R code with
// no write arm, so RequiredRead/ReadOnly too.
// The point rows #1307 appended carry the table R code.
// Active_Authentication_Policy and Authorization_Mode have routed write arms,
// so RequiredRead/Always; the application sets
// Number_Of_Authentication_Policies and Priority_For_Writing, which have no
// write arm, so RequiredRead/ReadOnly. Authentication_Policy_List and
// Authentication_Policy_Names (#1325) carry the table O code with footnote 1
// and no write arm, so Optional/ReadOnly; they are per-instance rows,
// present once the application sets the policies, before Property_List.
// The point's Reliability can read CONFIGURATION_ERROR since #1325, so
// Clause 12.31.8 opens it to writes while out of service:
// RequiredRead/WhenOutOfService. Access_Event_Authentication_Factor (#1132)
// carries the table O code with no write arm, so Optional/ReadOnly, after
// Priority_For_Writing.
// Zone Global_Identifier carries the table W code with the routed Unsigned
// arm, so RequiredWrite/Always. Table 12-37 has neither Present_Value nor
// Access_Doors, so the zone serves neither (#1064 removed the
// implementation-extra rows the 0.1.0 import carried).
// Occupancy_Count carries the table O code and Reliability the table R code,
// both with footnote 1, and dispatch takes their writes only while
// Out_Of_Service is TRUE (#1247), so Optional/WhenOutOfService and
// RequiredRead/WhenOutOfService. Occupancy_Count, Occupancy_Count_Enable
// and Adjust_Value also carry footnote 3, which requires them of a zone
// that reports intrinsically, as this one does (#1485). Entry_Points and Exit_Points carry the
// table R code with no arm, so RequiredRead/ReadOnly, and Status_Flags the
// table R code with no network write route, so RequiredRead/ReadOnly.
// The rows #1284 appended: Occupancy_State and Event_State carry the table R
// code and are derived, so RequiredRead/ReadOnly; Adjust_Value carries the O
// code with footnote 5 and a routed Integer arm, so Optional/Always; and
// Occupancy_Count_Enable and the two limits carry the O code with no write
// arm (the application sets them), so Optional/ReadOnly.
// The zone's event rows (#1305) follow the Multi-state Input's: the Table
// 12-37 O code with footnote 3, which requires the row of a zone that
// reports intrinsically (IntrinsicReportingRequired), or footnote 7 alone,
// which only permits it (IntrinsicReportingOptional: Event_Message_Texts and
// Time_Delay_Normal; #1485). Time_Delay, Notification_Class, Alarm_Values,
// Event_Enable, Notify_Type, Event_Detection_Enable and Time_Delay_Normal
// have routed write arms, so Always; Acked_Transitions, Event_Time_Stamps
// and Event_Message_Texts are kept by the event machinery, so ReadOnly.
// Apart from the door's and the zone's footnote-1 rows and the door's
// Reliability, every write arm is routed unconditionally and the suites pin in-service writes, so those rows
// are Always and the metadata mirrors dispatch. Presence is None on every
// other row: the family models no commandable or paired-text gating.
// The trio is not createable at runtime (the network factory builds only the
// eight analog/binary/multi-state input/output/value types, so the
// is_createable=false default holds) and remains deleteable (delete denies
// only Device and NetworkPort, so the is_deleteable=true default holds);
// neither needs an override. Array gating keeps the default: Priority_Array
// and Door_Members (BACnetARRAYs per Table 12-30), Access_Doors (Table 12-36)
// and Property_List admit an index while every other served row rejects one. COV keeps its override: supports_cov=true on
// Access Door and Access Point (Table 13-1 lists both; #1061 added the
// point) and false on Access Zone, and the COV gating path (read_property
// plus supports_cov_property to supports_cov) never consults metadata.
const ACCESS_DOOR_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRESENT_VALUE, RequiredWrite, None, Always),
    PropertyMetadata::new(P::DOOR_STATUS, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::LOCK_STATUS, Optional, None, WhenOutOfService),
    PropertyMetadata::new(P::SECURED_STATUS, Optional, None, ReadOnly),
    PropertyMetadata::new(
        P::DOOR_ALARM_STATE,
        Optional,
        Some(IntrinsicReportingRequired),
        WhenOutOfService,
    ),
    PropertyMetadata::new(P::DOOR_MEMBERS, Optional, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PRIORITY_ARRAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::RELINQUISH_DEFAULT, RequiredRead, None, Always),
    PropertyMetadata::new(P::DOOR_PULSE_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DOOR_EXTENDED_PULSE_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::DOOR_OPEN_TOO_LONG_TIME, RequiredRead, None, Always),
    PropertyMetadata::new(P::CURRENT_COMMAND_PRIORITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MASKED_ALARM_VALUES, Optional, None, Always),
    PropertyMetadata::new(
        P::TIME_DELAY,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::NOTIFICATION_CLASS,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::ALARM_VALUES,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(P::FAULT_VALUES, Optional, None, Always),
    PropertyMetadata::new(
        P::EVENT_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::ACKED_TRANSITIONS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::NOTIFY_TYPE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::EVENT_TIME_STAMPS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS,
        Optional,
        Some(IntrinsicReportingOptional),
        ReadOnly,
    ),
    // Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1329).
    REPORTING_OPTION_METADATA[0],
    REPORTING_OPTION_METADATA[1],
    REPORTING_OPTION_METADATA[2],
    PropertyMetadata::new(
        P::EVENT_DETECTION_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_POINT_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_EVENT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_EVENT_TAG, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_EVENT_TIME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_DOORS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::AUTHENTICATION_STATUS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACCESS_EVENT_CREDENTIAL, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::ACTIVE_AUTHENTICATION_POLICY, RequiredRead, None, Always),
    PropertyMetadata::new(
        P::NUMBER_OF_AUTHENTICATION_POLICIES,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(P::AUTHORIZATION_MODE, RequiredRead, None, Always),
    PropertyMetadata::new(P::PRIORITY_FOR_WRITING, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::ACCESS_EVENT_AUTHENTICATION_FACTOR,
        Optional,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

const ACCESS_ZONE_BASE: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::GLOBAL_IDENTIFIER, RequiredWrite, None, Always),
    PropertyMetadata::new(
        P::OCCUPANCY_COUNT,
        Optional,
        Some(IntrinsicReportingRequired),
        WhenOutOfService,
    ),
    PropertyMetadata::new(P::ENTRY_POINTS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EXIT_POINTS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, WhenOutOfService),
    PropertyMetadata::new(P::OCCUPANCY_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::EVENT_STATE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::OCCUPANCY_COUNT_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::ADJUST_VALUE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(P::OCCUPANCY_UPPER_LIMIT, Optional, None, ReadOnly),
    PropertyMetadata::new(P::OCCUPANCY_LOWER_LIMIT, Optional, None, ReadOnly),
    PropertyMetadata::new(
        P::TIME_DELAY,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::NOTIFICATION_CLASS,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::ALARM_VALUES,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::EVENT_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::ACKED_TRANSITIONS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::NOTIFY_TYPE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::EVENT_TIME_STAMPS,
        Optional,
        Some(IntrinsicReportingRequired),
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::EVENT_MESSAGE_TEXTS,
        Optional,
        Some(IntrinsicReportingOptional),
        ReadOnly,
    ),
    // Event_Message_Texts_Config and the Event_Algorithm_Inhibit pair (#1329).
    REPORTING_OPTION_METADATA[0],
    REPORTING_OPTION_METADATA[1],
    REPORTING_OPTION_METADATA[2],
    PropertyMetadata::new(
        P::EVENT_DETECTION_ENABLE,
        Optional,
        Some(IntrinsicReportingRequired),
        Always,
    ),
    PropertyMetadata::new(
        P::TIME_DELAY_NORMAL,
        Optional,
        Some(IntrinsicReportingOptional),
        Always,
    ),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];

pub(super) fn for_access_door_object(_object: &AccessDoorObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_DOOR_BASE)
}

pub(super) fn for_access_point_object(object: &AccessPointObject) -> Cow<'_, [PropertyMetadata]> {
    if !object.serves_policy_list() {
        return Cow::Borrowed(ACCESS_POINT_BASE);
    }
    let mut rows = ACCESS_POINT_BASE.to_vec();
    let before_property_list = rows
        .iter()
        .position(|row| row.property_identifier == P::PROPERTY_LIST)
        .expect("Property_List row");
    rows.splice(
        before_property_list..before_property_list,
        [
            PropertyMetadata::new(P::AUTHENTICATION_POLICY_LIST, Optional, None, ReadOnly),
            PropertyMetadata::new(P::AUTHENTICATION_POLICY_NAMES, Optional, None, ReadOnly),
        ],
    );
    Cow::Owned(rows)
}

pub(super) fn for_access_zone_object(_object: &AccessZoneObject) -> Cow<'_, [PropertyMetadata]> {
    Cow::Borrowed(ACCESS_ZONE_BASE)
}

#[cfg(test)]
#[path = "metadata_topology_tests.rs"]
mod tests;
