//! What a successful WriteProperty may change on top of
//! `write_side_effects.rs`, once the catalogue objects have clocks and the
//! seeded state `write_clocks.rs` gives them (#2262).
//!
//! The clock-bound proptest (`tests/write_property_clocked.rs`) holds every
//! accepted write to both tables: a write of P may change P, what the
//! clock-free table lists for P, what [`clocked_effects`] adds here, what
//! [`routed_effects`] adds for a write through `write_property_from` only,
//! and the stamps [`stamps`] names, which may only move to the current
//! step's date and time. As there, each entry comes from the type's write
//! path and the reads it feeds. The coverage pass fails on any entry no
//! write of its sweep or scenarios uses (`write_scenarios.rs`).
//!
//! What the setup reveals:
//! - **Commands with a writer** (routed). `write_property_from` takes Present_Value
//!   on the commandable outputs and values: it moves the priority array, the
//!   priority in control, the published Value_Source, the slot's
//!   Value_Source_Array entry and Last_Command_Time (a sequence number that
//!   steps when the effective value or source changes). A Value_Source
//!   correction rewrites its slot only and leaves Last_Command_Time.
//! - **Lighting** (routed). A Lighting Output's Present_Value and Lighting_Command
//!   run the lighting engine as well: Tracking_Value, In_Progress and
//!   Egress_Active (a warn) move with the slot.
//! - **Raised events.** With an event raised, Event_Detection_Enable FALSE
//!   puts Event_State back to NORMAL and resets Acked_Transitions and the
//!   event time stamps, which clears Status_Flags' IN_ALARM (and, on an
//!   Access Door, can secure it again).
//! - **Logs.** With a clock, a Enable change, a purge (Record_Count 0,
//!   or a new Log_DeviceObjectProperty), a Stop_When_Full that finds the
//!   buffer full and a window edge (Start_Time, Stop_Time) each add a
//!   log-status record, moving Record_Count, Total_Record_Count and
//!   Records_Since_Notification, and a full buffer may turn Enable off.
//!   Turning detection back on restarts the notification count
//!   (Last_Notify_Record, Records_Since_Notification).
//! - **Clock-derived reads.** Calendar Present_Value is whether today is in
//!   Date_List; Access Credential Credential_Status and Reason_For_Disable
//!   compare Activation_Time and Expiration_Time with today; a running
//!   Timer's Expiration_Time is now plus the time left.
//! - **Command actions.** A Command's Present_Value starts its list:
//!   In_Process turns TRUE until the host finishes the run, and
//!   All_Writes_Successful starts again.
//! - **Pending activation.** On a BBMD Network Port whose owner activates
//!   changes, the BDT, Accept_FD_Registrations and multicast group rows
//!   wait for activation, so Changes_Pending turns TRUE.
//!
//! Timestamps: the stamps a write takes from the Device clock (a File's
//! Modification_Date, a Pulse Converter's Count_Change_Time, a Timer's
//! Update_Time) are in [`stamps`], so the check compares them with the
//! current step. A Timer's Expiration_Time is now plus a duration, not the
//! current step, so it is an ordinary side effect.
//!
//! What this setup still leaves out, so no entry claims it: the logs'
//! event reporting (their event commits are unsupported, so their
//! Event_State never leaves NORMAL), and an Audit Log's records from
//! audited operations.

use bacnet_types::enums::{ObjectType as T, PropertyIdentifier as P};

use crate::write_side_effects::Effect;

/// A command with its writer, on an object that tracks Value_Source per
/// priority.
const SOURCED_COMMAND: &[Effect] = &[
    (
        P::PRESENT_VALUE,
        &[
            P::PRIORITY_ARRAY,
            P::CURRENT_COMMAND_PRIORITY,
            P::VALUE_SOURCE,
            P::VALUE_SOURCE_ARRAY,
            P::LAST_COMMAND_TIME,
        ],
    ),
    (P::VALUE_SOURCE, &[P::VALUE_SOURCE_ARRAY]),
];

/// Event_Detection_Enable FALSE clears a raised event.
const EVENT_RESET: &[Effect] = &[(
    P::EVENT_DETECTION_ENABLE,
    &[
        P::EVENT_STATE,
        P::ACKED_TRANSITIONS,
        P::EVENT_TIME_STAMPS,
        P::STATUS_FLAGS,
    ],
)];

/// What a log-status record moves.
const LOG_RECORD: &[P] = &[
    P::RECORD_COUNT,
    P::TOTAL_RECORD_COUNT,
    P::RECORDS_SINCE_NOTIFICATION,
];

/// A log-status record, and Enable turned off when it fills a buffer
/// that stops when full.
const LOG_RECORD_MAY_STOP: &[P] = &[
    P::RECORD_COUNT,
    P::TOTAL_RECORD_COUNT,
    P::RECORDS_SINCE_NOTIFICATION,
    P::ENABLE,
];

/// The writes that record log status on a Trend Log, Trend Log Multiple or
/// Event Log (`log_lifecycle.rs`), and turning detection back on.
const LOG_LIFECYCLE: &[Effect] = &[
    (P::ENABLE, LOG_RECORD),
    // A purge empties the buffer and records it. It turns logging off
    // only in a buffer of one record, and Buffer_Size is read-only here.
    (
        P::RECORD_COUNT,
        &[P::TOTAL_RECORD_COUNT, P::RECORDS_SINCE_NOTIFICATION],
    ),
    // Only a full buffer records it, so the count stays at capacity.
    (
        P::STOP_WHEN_FULL,
        &[
            P::TOTAL_RECORD_COUNT,
            P::RECORDS_SINCE_NOTIFICATION,
            P::ENABLE,
        ],
    ),
    (P::START_TIME, LOG_RECORD_MAY_STOP),
    (P::STOP_TIME, LOG_RECORD_MAY_STOP),
    (
        P::EVENT_DETECTION_ENABLE,
        &[P::LAST_NOTIFY_RECORD, P::RECORDS_SINCE_NOTIFICATION],
    ),
];

/// A Trend Log's new monitored property purges the buffer.
const LOG_PURGE_ON_REFERENCE: &[Effect] = &[(P::LOG_DEVICE_OBJECT_PROPERTY, LOG_RECORD)];

/// What the lighting engine moves on a Lighting Output, besides
/// Present_Value: the slot and its source, and the operation it starts or
/// halts (a level written over a running fade halts it).
const LIGHTING_ENGINE: &[P] = &[
    P::PRIORITY_ARRAY,
    P::CURRENT_COMMAND_PRIORITY,
    P::VALUE_SOURCE_ARRAY,
    P::LAST_COMMAND_TIME,
    P::TRACKING_VALUE,
    P::IN_PROGRESS,
    P::EGRESS_ACTIVE,
];

/// Each type's added fragments; empty for a type the setup adds nothing to.
pub fn clocked_effects(object_type: T) -> &'static [&'static [Effect]] {
    match object_type {
        T::ANALOG_OUTPUT
        | T::ANALOG_VALUE
        | T::BINARY_OUTPUT
        | T::BINARY_VALUE
        | T::MULTI_STATE_OUTPUT
        | T::MULTI_STATE_VALUE => &[EVENT_RESET],
        T::ANALOG_INPUT
        | T::BINARY_INPUT
        | T::MULTI_STATE_INPUT
        | T::ACCESS_ZONE
        | T::EVENT_ENROLLMENT => &[EVENT_RESET],
        // Secured_Status reads the door's alarm state.
        T::ACCESS_DOOR => &[
            EVENT_RESET,
            &[(P::EVENT_DETECTION_ENABLE, &[P::SECURED_STATUS])],
        ],
        T::TREND_LOG | T::TREND_LOG_MULTIPLE => &[LOG_LIFECYCLE, LOG_PURGE_ON_REFERENCE],
        T::EVENT_LOG => &[LOG_LIFECYCLE],
        // A Enable change appends a status record; a smaller
        // Buffer_Size drops the oldest records.
        T::AUDIT_LOG => &[&[
            (P::ENABLE, &[P::RECORD_COUNT, P::TOTAL_RECORD_COUNT]),
            (P::BUFFER_SIZE, &[P::RECORD_COUNT]),
        ]],
        T::CALENDAR => &[&[(P::DATE_LIST, &[P::PRESENT_VALUE])]],
        T::ACCESS_CREDENTIAL => &[&[
            (
                P::ACTIVATION_TIME,
                &[P::CREDENTIAL_STATUS, P::REASON_FOR_DISABLE],
            ),
            (
                P::EXPIRATION_TIME,
                &[P::CREDENTIAL_STATUS, P::REASON_FOR_DISABLE],
            ),
        ]],
        T::COMMAND => &[&[(P::PRESENT_VALUE, &[P::IN_PROCESS, P::ALL_WRITES_SUCCESSFUL])]],
        T::NETWORK_PORT => &[&[
            (P::BBMD_BROADCAST_DISTRIBUTION_TABLE, &[P::CHANGES_PENDING]),
            (P::BBMD_ACCEPT_FD_REGISTRATIONS, &[P::CHANGES_PENDING]),
            (P::BACNET_IP_MULTICAST_ADDRESS, &[P::CHANGES_PENDING]),
        ]],
        // Starting, stopping and expiring the timer.
        T::TIMER => &[&[
            (P::PRESENT_VALUE, &[P::EXPIRATION_TIME]),
            (P::TIMER_RUNNING, &[P::EXPIRATION_TIME]),
            (P::TIMER_STATE, &[P::EXPIRATION_TIME]),
        ]],
        _ => &[],
    }
}

/// Each type's entries for a write through `write_property_from` only: the
/// commands that need their writer. A plain `write_property` of these
/// properties is refused on these types, so it may move none of them.
pub fn routed_effects(object_type: T) -> &'static [&'static [Effect]] {
    match object_type {
        T::ANALOG_OUTPUT
        | T::ANALOG_VALUE
        | T::BINARY_OUTPUT
        | T::BINARY_VALUE
        | T::MULTI_STATE_OUTPUT
        | T::MULTI_STATE_VALUE => &[SOURCED_COMMAND],
        T::LIGHTING_OUTPUT => &[&[
            (P::PRESENT_VALUE, LIGHTING_ENGINE),
            (P::LIGHTING_COMMAND, LIGHTING_ENGINE),
            (P::LIGHTING_COMMAND, &[P::PRESENT_VALUE]),
            // Only a NULL can empty every slot, so only Present_Value moves
            // the published source with one writer.
            (P::PRESENT_VALUE, &[P::VALUE_SOURCE]),
            (P::VALUE_SOURCE, &[P::VALUE_SOURCE_ARRAY]),
        ]],
        // A warn arms an egress.
        T::BINARY_LIGHTING_OUTPUT => &[SOURCED_COMMAND, &[(P::PRESENT_VALUE, &[P::EGRESS_ACTIVE])]],
        _ => &[],
    }
}

/// Each type's stamps: a write of `.0` may set each property in `.1` to the
/// current step's date and time, and to nothing else.
pub fn stamps(object_type: T) -> &'static [Effect] {
    match object_type {
        // Changing the data stamps it (`FileObject::mark_modified`).
        T::FILE => &[(P::FILE_SIZE, &[P::MODIFICATION_DATE])],
        // An Adjust_Value write moves Count, stamped.
        T::PULSE_CONVERTER => &[(P::ADJUST_VALUE, &[P::COUNT_CHANGE_TIME])],
        // Each state change the write makes is stamped.
        T::TIMER => &[
            (P::PRESENT_VALUE, &[P::UPDATE_TIME]),
            (P::TIMER_RUNNING, &[P::UPDATE_TIME]),
            (P::TIMER_STATE, &[P::UPDATE_TIME]),
        ],
        _ => &[],
    }
}

/// Every entry this table holds for `object_type`: the clocked entries,
/// the routed ones when `from` is set, and the stamps when `stamped` is.
pub fn entries(object_type: T, from: bool, stamped: bool) -> impl Iterator<Item = &'static Effect> {
    let routed: &[&[Effect]] = if from {
        routed_effects(object_type)
    } else {
        &[]
    };
    let stamps: &[Effect] = if stamped { stamps(object_type) } else { &[] };
    clocked_effects(object_type)
        .iter()
        .chain(routed)
        .flat_map(|fragment| fragment.iter())
        .chain(stamps)
}

/// Whether this table (not the clock-free one) lets a write of `property`,
/// through `write_property_from` when `from` is set, change `other`.
pub fn listed(object_type: T, property: P, other: P, from: bool) -> bool {
    entries(object_type, from, false)
        .any(|(written, changed)| *written == property && changed.contains(&other))
}
