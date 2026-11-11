//! Which other properties a successful WriteProperty may change, for each
//! catalogue type (#2234).
//!
//! The WriteProperty proptest (`tests/write_property.rs`) holds every
//! successful write to this table: after a write of P, only P and the
//! properties listed for P on that object type may read differently. A
//! change to the metadata rows counts as a change of Property_List.
//!
//! The entries come from each type's `write_property` and the reads it
//! feeds, for the catalogue objects as `representatives()` builds them, in
//! that minimal state: standalone, with no database, no Device clock, no
//! monotonic clock and default configuration (the one change the proptest
//! makes is seeding the Averaging object with two samples). The table claims
//! no more than that state shows:
//! - nothing reads wall-clock or monotonic time, and a stamp a write takes
//!   (a File's Modification_Date, a Pulse Converter's Count_Change_Time, a
//!   Timer's Update_Time) stays unspecified, which is no change. The two
//!   stamps that do move, Access Point Access_Event_Time and Credential Data
//!   Input Update_Time, count a sequence number, so they are listed below as
//!   ordinary side effects. No property is skipped as time dependent: the
//!   proptest's back-to-back read check would catch one that moved by itself.
//! - no log record is written, because every log-status record needs the
//!   clock and a write that would add one is refused (Enable changes, a
//!   Record_Count purge, a new Log_DeviceObjectProperty). So no log count,
//!   buffer, Status_Flags or Event_State moves on a log type.
//! - writes that only `write_property_from` takes (Present_Value of Analog,
//!   Binary and Multi-state Output and Value, the Lighting commands) are
//!   refused here, so those priority arrays stay empty.
//!
//! Known effects that this state hides, so the table leaves them out:
//! - with a clock: Calendar Date_List moves Present_Value; Access Credential
//!   Activation_Time and Expiration_Time move Credential_Status and
//!   Reason_For_Disable; stamps (Modification_Date, Count_Change_Time, the
//!   Timer's Update_Time and Expiration_Time) move; log writes add records,
//!   moving Record_Count, Total_Record_Count, Log_Buffer and
//!   Records_Since_Notification, and may turn Enable off;
//! - with configured actions, Command Present_Value moves In_Process and
//!   All_Writes_Successful;
//! - with an IP configuration (or a non-Normal command mode), Network Port
//!   writes move Changes_Pending;
//! - with an event raised, Event_Detection_Enable FALSE resets Event_State,
//!   Acked_Transitions and the event time stamps (they are already initial
//!   here);
//! - with value-source commands, the commandable outputs' Value_Source,
//!   Value_Source_Array and Last_Command_Time.
//!
//! An entry says what a write *may* change: whether it does depends on the
//! object's state (most Reliability effects need Out_Of_Service first, and
//! Time_Delay moves Time_Delay_Normal only until that has been written). Each
//! type lists the shared fragments it uses and then its own entries, so a
//! type's row can change without touching the others.

use bacnet_types::enums::{ObjectType as T, PropertyIdentifier as P};

/// A write of `.0` may also change each property in `.1`.
pub type Effect = (P, &'static [P]);

/// Out_Of_Service is one of Status_Flags' inputs.
const OUT_OF_SERVICE_FLAGS: &[Effect] = &[(P::OUT_OF_SERVICE, &[P::STATUS_FLAGS])];

/// Reliability, writable while out of service, is another input; leaving
/// out of service restores (or re-evaluates) the Reliability that was set
/// aside, overwriting one written meanwhile.
const RELIABILITY_SIMULATION: &[Effect] = &[
    (P::OUT_OF_SERVICE, &[P::STATUS_FLAGS, P::RELIABILITY]),
    (P::RELIABILITY, &[P::STATUS_FLAGS]),
];

/// Time_Delay_Normal reads as Time_Delay until it is written itself.
const TIME_DELAY_NORMAL_FALLBACK: &[Effect] = &[(P::TIME_DELAY, &[P::TIME_DELAY_NORMAL])];

/// Clearing Event_Algorithm_Inhibit_Ref while a reference is set puts
/// Event_Algorithm_Inhibit back to FALSE (`event/options.rs`).
const INHIBIT_REFERENCE: &[Effect] = &[(
    P::EVENT_ALGORITHM_INHIBIT_REF,
    &[P::EVENT_ALGORITHM_INHIBIT],
)];

/// Present_Value reads Relinquish_Default while no priority is commanded.
const RELINQUISH_DEFAULT: &[Effect] = &[(P::RELINQUISH_DEFAULT, &[P::PRESENT_VALUE])];

/// A Present_Value command, or a NULL relinquishing one, moves the priority
/// array and the priority that is in control.
const COMMANDED: &[Effect] = &[(
    P::PRESENT_VALUE,
    &[P::PRIORITY_ARRAY, P::CURRENT_COMMAND_PRIORITY],
)];

/// The intrinsic-reporting input types: Analog, Binary and Multi-state
/// Input, Output and Value.
const REPORTING_POINT: &[&[Effect]] = &[
    RELIABILITY_SIMULATION,
    TIME_DELAY_NORMAL_FALLBACK,
    INHIBIT_REFERENCE,
];

/// What a reset of the Averaging window moves, once it holds samples.
const AVERAGING_STATISTICS: &[P] = &[
    P::MINIMUM_VALUE,
    P::AVERAGE_VALUE,
    P::MAXIMUM_VALUE,
    P::ATTEMPTED_SAMPLES,
    P::VALID_SAMPLES,
];

/// Each catalogue type's fragments, or `None` for a type the table doesn't
/// know (so a new catalogue type fails until it has a row).
pub fn effects(object_type: T) -> Option<&'static [&'static [Effect]]> {
    Some(match object_type {
        T::ANALOG_INPUT | T::BINARY_INPUT => REPORTING_POINT,
        T::ANALOG_OUTPUT | T::ANALOG_VALUE | T::BINARY_OUTPUT | T::BINARY_VALUE => &[
            RELIABILITY_SIMULATION,
            TIME_DELAY_NORMAL_FALLBACK,
            INHIBIT_REFERENCE,
            RELINQUISH_DEFAULT,
        ],
        T::MULTI_STATE_INPUT => &[
            RELIABILITY_SIMULATION,
            TIME_DELAY_NORMAL_FALLBACK,
            INHIBIT_REFERENCE,
            // Number_Of_States is State_Text's length.
            &[(P::STATE_TEXT, &[P::NUMBER_OF_STATES])],
        ],
        T::MULTI_STATE_VALUE => &[
            RELIABILITY_SIMULATION,
            TIME_DELAY_NORMAL_FALLBACK,
            INHIBIT_REFERENCE,
            RELINQUISH_DEFAULT,
            &[(P::STATE_TEXT, &[P::NUMBER_OF_STATES])],
        ],
        T::MULTI_STATE_OUTPUT => &[
            RELIABILITY_SIMULATION,
            TIME_DELAY_NORMAL_FALLBACK,
            INHIBIT_REFERENCE,
            RELINQUISH_DEFAULT,
            // A Feedback_Value outside Number_Of_States is a configuration
            // error; the evaluator reruns on these writes and on the
            // inhibit, and leaving out of service reruns it too.
            &[
                (
                    P::STATE_TEXT,
                    &[P::NUMBER_OF_STATES, P::RELIABILITY, P::STATUS_FLAGS],
                ),
                (P::FEEDBACK_VALUE, &[P::RELIABILITY, P::STATUS_FLAGS]),
                (
                    P::RELIABILITY_EVALUATION_INHIBIT,
                    &[P::RELIABILITY, P::STATUS_FLAGS],
                ),
            ],
        ],
        T::BITSTRING_VALUE
        | T::CHARACTERSTRING_VALUE
        | T::DATEPATTERN_VALUE
        | T::DATE_VALUE
        | T::DATETIMEPATTERN_VALUE
        | T::DATETIME_VALUE
        | T::LARGE_ANALOG_VALUE
        | T::OCTETSTRING_VALUE
        | T::POSITIVE_INTEGER_VALUE
        | T::TIMEPATTERN_VALUE
        | T::TIME_VALUE => &[OUT_OF_SERVICE_FLAGS, COMMANDED, RELINQUISH_DEFAULT],
        // Reliability simulated out of service, as on the Accumulator (#2446).
        T::INTEGER_VALUE => &[RELIABILITY_SIMULATION, COMMANDED, RELINQUISH_DEFAULT],
        T::GLOBAL_GROUP
        | T::PROGRAM
        | T::NOTIFICATION_FORWARDER
        | T::NETWORK_PORT
        | T::ESCALATOR
        | T::LIFT => &[OUT_OF_SERVICE_FLAGS],
        T::BINARY_LIGHTING_OUTPUT => &[OUT_OF_SERVICE_FLAGS, RELINQUISH_DEFAULT],
        T::PULSE_CONVERTER => &[
            RELIABILITY_SIMULATION,
            &[
                // Back in service, Present_Value is Count x Scale_Factor again.
                (P::OUT_OF_SERVICE, &[P::PRESENT_VALUE]),
                (
                    P::ADJUST_VALUE,
                    &[P::COUNT, P::COUNT_BEFORE_CHANGE, P::PRESENT_VALUE],
                ),
                (P::SCALE_FACTOR, &[P::PRESENT_VALUE]),
            ],
        ],
        T::LOOP | T::ACCUMULATOR => &[RELIABILITY_SIMULATION],
        T::ACCESS_DOOR => &[
            RELIABILITY_SIMULATION,
            TIME_DELAY_NORMAL_FALLBACK,
            INHIBIT_REFERENCE,
            COMMANDED,
            &[
                // Leaving out of service puts back the door state set aside;
                // Secured_Status derives from the door and lock status,
                // Present_Value and the masked alarm values.
                (
                    P::OUT_OF_SERVICE,
                    &[
                        P::DOOR_STATUS,
                        P::LOCK_STATUS,
                        P::DOOR_ALARM_STATE,
                        P::SECURED_STATUS,
                    ],
                ),
                (P::PRESENT_VALUE, &[P::SECURED_STATUS]),
                (
                    P::RELINQUISH_DEFAULT,
                    &[P::PRESENT_VALUE, P::SECURED_STATUS],
                ),
                (P::DOOR_STATUS, &[P::SECURED_STATUS]),
                (P::LOCK_STATUS, &[P::SECURED_STATUS]),
                // A fault value held in Door_Alarm_State reads as a fault.
                (P::DOOR_ALARM_STATE, &[P::RELIABILITY, P::STATUS_FLAGS]),
                // A held state the lists no longer admit settles to NORMAL.
                (
                    // Fault values stay admitted, so Reliability can't move.
                    P::ALARM_VALUES,
                    &[P::DOOR_ALARM_STATE],
                ),
                (
                    P::FAULT_VALUES,
                    &[P::DOOR_ALARM_STATE, P::RELIABILITY, P::STATUS_FLAGS],
                ),
                (
                    P::MASKED_ALARM_VALUES,
                    &[
                        P::DOOR_ALARM_STATE,
                        P::RELIABILITY,
                        P::STATUS_FLAGS,
                        P::SECURED_STATUS,
                    ],
                ),
            ],
        ],
        T::ACCESS_CREDENTIAL => &[&[(
            P::CREDENTIAL_DISABLE,
            &[P::REASON_FOR_DISABLE, P::CREDENTIAL_STATUS],
        )]],
        T::ACCESS_POINT => &[
            RELIABILITY_SIMULATION,
            // Each Out_Of_Service edge records an access event, and
            // authentication reads DISABLED while out of service.
            &[(
                P::OUT_OF_SERVICE,
                &[
                    P::ACCESS_EVENT,
                    P::ACCESS_EVENT_TAG,
                    P::ACCESS_EVENT_TIME,
                    P::AUTHENTICATION_STATUS,
                ],
            )],
        ],
        T::ACCESS_ZONE => &[
            RELIABILITY_SIMULATION,
            TIME_DELAY_NORMAL_FALLBACK,
            INHIBIT_REFERENCE,
            &[
                (P::ADJUST_VALUE, &[P::OCCUPANCY_COUNT]),
                // Leaving out of service puts back the count set aside.
                (P::OUT_OF_SERVICE, &[P::OCCUPANCY_COUNT]),
            ],
        ],
        T::CREDENTIAL_DATA_INPUT => &[
            RELIABILITY_SIMULATION,
            &[
                (P::OUT_OF_SERVICE, &[P::PRESENT_VALUE, P::UPDATE_TIME]),
                (P::PRESENT_VALUE, &[P::UPDATE_TIME]),
            ],
        ],
        T::LIFE_SAFETY_POINT | T::LIFE_SAFETY_ZONE => &[
            RELIABILITY_SIMULATION,
            // Leaving out of service puts back the Tracking_Value set aside.
            &[(P::OUT_OF_SERVICE, &[P::TRACKING_VALUE])],
        ],
        T::LIGHTING_OUTPUT => &[
            OUT_OF_SERVICE_FLAGS,
            // Tracking_Value follows Present_Value when no fade runs.
            &[(
                P::RELINQUISH_DEFAULT,
                &[P::PRESENT_VALUE, P::TRACKING_VALUE],
            )],
        ],
        T::COLOR | T::COLOR_TEMPERATURE => &[&[
            // Both start the transition engine, which never advances here.
            (P::PRESENT_VALUE, &[P::TRACKING_VALUE, P::IN_PROGRESS]),
            (
                P::COLOR_COMMAND,
                &[P::PRESENT_VALUE, P::TRACKING_VALUE, P::IN_PROGRESS],
            ),
        ]],
        T::CHANNEL => &[
            RELIABILITY_SIMULATION,
            &[
                (P::PRESENT_VALUE, &[P::LAST_PRIORITY, P::WRITE_STATUS]),
                // The two arrays are resized together.
                (P::LIST_OF_OBJECT_PROPERTY_REFERENCES, &[P::EXECUTION_DELAY]),
                (P::EXECUTION_DELAY, &[P::LIST_OF_OBJECT_PROPERTY_REFERENCES]),
            ],
        ],
        T::TIMER => &[
            RELIABILITY_SIMULATION,
            // Starting, expiring and clearing the timer move its state.
            &[
                (
                    P::PRESENT_VALUE,
                    &[
                        P::TIMER_STATE,
                        P::TIMER_RUNNING,
                        P::INITIAL_TIMEOUT,
                        P::LAST_STATE_CHANGE,
                    ],
                ),
                (
                    P::TIMER_RUNNING,
                    &[
                        P::PRESENT_VALUE,
                        P::TIMER_STATE,
                        P::INITIAL_TIMEOUT,
                        P::LAST_STATE_CHANGE,
                    ],
                ),
                (
                    P::TIMER_STATE,
                    &[P::PRESENT_VALUE, P::TIMER_RUNNING, P::LAST_STATE_CHANGE],
                ),
            ],
        ],
        T::STAGING => &[
            // Leaving out of service puts back the Reliability set aside,
            // ending a simulated one (#2258).
            RELIABILITY_SIMULATION,
            // Present_Stage is evaluated again; Present_Value is clamped to
            // the range, whose top is the last stage's limit.
            &[
                (P::PRESENT_VALUE, &[P::PRESENT_STAGE]),
                (P::MIN_PRES_VALUE, &[P::PRESENT_VALUE, P::PRESENT_STAGE]),
                (
                    P::STAGES,
                    &[P::MAX_PRES_VALUE, P::PRESENT_VALUE, P::PRESENT_STAGE],
                ),
                (P::TARGET_REFERENCES, &[P::PRESENT_STAGE]),
                (P::PRIORITY_FOR_WRITING, &[P::PRESENT_STAGE]),
                (P::OUT_OF_SERVICE, &[P::PRESENT_STAGE]),
            ],
        ],
        T::LOAD_CONTROL => &[&[(
            P::REQUESTED_SHED_LEVEL,
            &[P::EXPECTED_SHED_LEVEL, P::ACTUAL_SHED_LEVEL],
        )]],
        T::FILE => &[&[
            // Changing the data clears Archive.
            (P::FILE_SIZE, &[P::ARCHIVE]),
        ]],
        T::SCHEDULE => &[
            RELIABILITY_SIMULATION,
            // Scheduled values of mixed datatypes are a configuration error,
            // judged again when any of them changes.
            &[
                (P::SCHEDULE_DEFAULT, &[P::RELIABILITY, P::STATUS_FLAGS]),
                (P::WEEKLY_SCHEDULE, &[P::RELIABILITY, P::STATUS_FLAGS]),
                (P::EXCEPTION_SCHEDULE, &[P::RELIABILITY, P::STATUS_FLAGS]),
            ],
        ],
        T::EVENT_ENROLLMENT => &[
            INHIBIT_REFERENCE,
            // Event_Type and Fault_Type are the parameters' choices, and
            // Time_Delay_Normal falls back to the parameters' time delay.
            &[
                (P::EVENT_PARAMETERS, &[P::EVENT_TYPE, P::TIME_DELAY_NORMAL]),
                (P::FAULT_PARAMETERS, &[P::FAULT_TYPE]),
            ],
        ],
        T::ALERT_ENROLLMENT | T::EVENT_LOG => &[INHIBIT_REFERENCE],
        T::TREND_LOG => &[
            INHIBIT_REFERENCE,
            // The acquisition mode and the interval set each other, and
            // Log_Interval's row is read-only while TRIGGERED.
            &[
                (
                    P::LOGGING_TYPE,
                    &[P::LOG_INTERVAL, P::TRIGGER, P::PROPERTY_LIST],
                ),
                (P::LOG_INTERVAL, &[P::LOGGING_TYPE]),
            ],
        ],
        T::TREND_LOG_MULTIPLE => &[
            INHIBIT_REFERENCE,
            &[(
                P::LOGGING_TYPE,
                &[P::LOG_INTERVAL, P::TRIGGER, P::PROPERTY_LIST],
            )],
        ],
        // In the catalogue's minimal state, each write changes only itself
        // (see the module notes for what a clock or more state would add).
        T::AVERAGING => &[&[
            // Each accepted write of these discards the samples (the
            // catalogue object is seeded with two, see write_property.rs).
            (P::OBJECT_PROPERTY_REFERENCE, AVERAGING_STATISTICS),
            (P::WINDOW_INTERVAL, AVERAGING_STATISTICS),
            (P::WINDOW_SAMPLES, AVERAGING_STATISTICS),
            // Attempted_Samples is the one statistic written: it takes 0.
            (
                P::ATTEMPTED_SAMPLES,
                &[
                    P::MINIMUM_VALUE,
                    P::AVERAGE_VALUE,
                    P::MAXIMUM_VALUE,
                    P::VALID_SAMPLES,
                ],
            ),
        ]],
        T::DEVICE
        | T::CALENDAR
        | T::COMMAND
        | T::GROUP
        | T::STRUCTURED_VIEW
        | T::NOTIFICATION_CLASS
        | T::ACCESS_RIGHTS
        | T::ACCESS_USER
        | T::ELEVATOR_GROUP
        | T::AUDIT_LOG
        | T::AUDIT_REPORTER => &[],
        _ => return None,
    })
}

/// The properties other than `property` that a successful write of it may
/// change on the catalogue object of `object_type`.
pub fn may_also_change(object_type: T, property: P) -> impl Iterator<Item = P> {
    effects(object_type)
        .unwrap_or_default()
        .iter()
        .flat_map(|fragment| fragment.iter())
        .filter(move |(written, _)| *written == property)
        .flat_map(|(_, changed)| changed.iter().copied())
}
