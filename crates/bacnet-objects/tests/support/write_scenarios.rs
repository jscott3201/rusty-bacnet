//! The clock-bound proptest's coverage data (#2262): the write sequences
//! that reach the clocked table's entries a single write can't, the rows
//! that take only the value they hold, and the entries nothing here can
//! reach.

use bacnet_encoding::constructed::{
    encode_device_object_property_reference, encode_lighting_command,
};
use bacnet_types::constructed::BACnetLightingCommand;
use bacnet_types::enums::{LightingOperation, ObjectType as T, PropertyIdentifier as P};
use bacnet_types::primitives::{Date, PropertyValue, Time};

use crate::write_clocks::DATE;
use crate::write_values::{av_present_value, encoded};

/// One write of a scenario, or of the targeted pass.
#[derive(Debug, Clone)]
pub struct Step {
    /// The property written.
    pub property: P,
    /// The array index.
    pub array_index: Option<u32>,
    /// The value.
    pub value: PropertyValue,
    /// The priority.
    pub priority: Option<u8>,
    /// Through `write_property_from` with the fixture's origin.
    pub from: bool,
    /// Extra clock steps before the write, beyond the usual one.
    pub wait: u32,
}

impl Step {
    /// A whole-property write of `value` through `write_property_from`, at
    /// no priority, after one step.
    pub fn new(property: P, value: PropertyValue) -> Self {
        Self {
            property,
            array_index: None,
            value,
            priority: None,
            from: true,
            wait: 0,
        }
    }
}

/// A sequence of writes on a fresh fixture of one type, each of which must
/// be accepted.
pub struct Scenario {
    /// What it shows.
    pub name: &'static str,
    /// The catalogue type.
    pub object_type: T,
    /// The writes, in order.
    pub steps: Vec<Step>,
}

impl Step {
    /// This write at `priority`.
    fn at(self, priority: u8) -> Self {
        Self {
            priority: Some(priority),
            ..self
        }
    }

    /// This write after `steps` more clock steps.
    fn after(self, steps: u32) -> Self {
        Self {
            wait: steps,
            ..self
        }
    }
}

fn boolean(value: bool) -> PropertyValue {
    PropertyValue::Boolean(value)
}

/// A BACnetDateTime of `DATE` at `hour`, or every field unspecified.
fn datetime(hour: Option<u8>) -> PropertyValue {
    let (date, time) = match hour {
        Some(hour) => (
            DATE,
            Time {
                hour,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
        ),
        None => (
            Date {
                year: 255,
                month: 255,
                day: 255,
                day_of_week: 255,
            },
            Time {
                hour: 255,
                minute: 255,
                second: 255,
                hundredths: 255,
            },
        ),
    };
    PropertyValue::List(vec![PropertyValue::Date(date), PropertyValue::Time(time)])
}

/// Analog Value 1's Present_Value, encoded.
fn reference() -> Vec<u8> {
    encoded(|buf| encode_device_object_property_reference(buf, &av_present_value()))
}

/// Two Enable toggles: with the seeded record, a buffer of three is
/// full.
fn fill_log() -> Vec<Step> {
    vec![
        Step::new(P::ENABLE, boolean(false)),
        Step::new(P::ENABLE, boolean(true)),
    ]
}

/// The scenarios: each reaches table entries or row changes that need
/// state an earlier write in it sets up.
#[allow(non_snake_case)]
pub fn SCENARIOS() -> Vec<Scenario> {
    let mut scenarios = Vec::new();
    for log in [T::TREND_LOG, T::TREND_LOG_MULTIPLE, T::EVENT_LOG] {
        let mut purge = fill_log();
        purge.push(Step::new(P::RECORD_COUNT, PropertyValue::Unsigned(0)));
        let mut stop = fill_log();
        stop.push(Step::new(P::STOP_WHEN_FULL, boolean(true)));
        scenarios.extend([
            Scenario {
                name: "a purge after records restarts the count",
                object_type: log,
                steps: purge,
            },
            Scenario {
                name: "Stop_When_Full on a full buffer stops logging",
                object_type: log,
                steps: stop,
            },
            Scenario {
                name: "detection back on restarts the notification count",
                object_type: log,
                steps: vec![
                    Step::new(P::EVENT_DETECTION_ENABLE, boolean(false)),
                    Step::new(P::EVENT_DETECTION_ENABLE, boolean(true)),
                ],
            },
        ]);
        // A window that closes (one more record) and reopens into a buffer
        // the next record fills, so logging stops.
        for (edge, closed) in [(P::STOP_TIME, 7), (P::START_TIME, 9)] {
            scenarios.push(Scenario {
                name: "a window reopening into a nearly full buffer stops logging",
                object_type: log,
                steps: vec![
                    Step::new(P::STOP_WHEN_FULL, boolean(true)),
                    Step::new(edge, datetime(Some(closed))),
                    Step::new(edge, datetime(None)),
                ],
            });
        }
    }
    for (log, value) in [
        (T::TREND_LOG, PropertyValue::ApplicationData(reference())),
        (
            T::TREND_LOG_MULTIPLE,
            PropertyValue::List(vec![PropertyValue::ApplicationData(reference())]),
        ),
    ] {
        let mut purge = fill_log();
        purge.push(Step::new(P::LOG_DEVICE_OBJECT_PROPERTY, value));
        scenarios.extend([
            Scenario {
                name: "a new monitored property purges a full buffer",
                object_type: log,
                steps: purge,
            },
            Scenario {
                name: "Trigger in TRIGGERED mode",
                object_type: log,
                steps: vec![
                    Step::new(P::LOGGING_TYPE, PropertyValue::Enumerated(2)),
                    Step::new(P::TRIGGER, boolean(true)),
                ],
            },
        ]);
    }
    let fade = BACnetLightingCommand {
        target_level: Some(10.0),
        fade_time: Some(5000),
        priority: Some(8),
        ..BACnetLightingCommand::new(LightingOperation::FADE_TO)
    };
    scenarios.extend([
        Scenario {
            name: "Timer_State IDLE stops a running timer",
            object_type: T::TIMER,
            steps: vec![
                Step::new(P::PRESENT_VALUE, PropertyValue::Unsigned(60_000)),
                Step::new(P::TIMER_STATE, PropertyValue::Enumerated(0)).after(2),
            ],
        },
        Scenario {
            name: "a level written over a running fade halts it",
            object_type: T::LIGHTING_OUTPUT,
            steps: vec![
                Step::new(
                    P::LIGHTING_COMMAND,
                    PropertyValue::ApplicationData(encoded(|buf| {
                        encode_lighting_command(buf, &fade)
                    })),
                ),
                Step::new(P::PRESENT_VALUE, PropertyValue::Real(30.0))
                    .at(8)
                    .after(1),
            ],
        },
        Scenario {
            name: "a door alarm state the lists admit",
            object_type: T::ACCESS_DOOR,
            steps: vec![
                Step::new(P::OUT_OF_SERVICE, boolean(true)),
                Step::new(
                    P::ALARM_VALUES,
                    PropertyValue::List(vec![PropertyValue::Enumerated(1)]),
                ),
                Step::new(P::DOOR_ALARM_STATE, PropertyValue::Enumerated(1)),
            ],
        },
        Scenario {
            name: "a Start_Time behind a level to shed starts a request",
            object_type: T::LOAD_CONTROL,
            steps: vec![
                // level [1] 1
                Step::new(
                    P::REQUESTED_SHED_LEVEL,
                    PropertyValue::ApplicationData(vec![0x19, 0x01]),
                ),
                Step::new(P::SHED_DURATION, PropertyValue::Unsigned(60)),
                Step::new(P::START_TIME, datetime(Some(9))),
            ],
        },
        Scenario {
            name: "a shorter Shed_Duration ends a request that has run its course",
            object_type: T::LOAD_CONTROL,
            steps: vec![
                Step::new(
                    P::REQUESTED_SHED_LEVEL,
                    PropertyValue::ApplicationData(vec![0x19, 0x01]),
                ),
                Step::new(P::SHED_DURATION, PropertyValue::Unsigned(90)),
                Step::new(P::DUTY_WINDOW, PropertyValue::Unsigned(15)),
                // Shedding since 07:00, until 08:30.
                Step::new(P::START_TIME, datetime(Some(7))),
                // Now it ended at 07:10.
                Step::new(P::SHED_DURATION, PropertyValue::Unsigned(10)),
            ],
        },
        Scenario {
            name: "execution delays for the Channel's one member",
            object_type: T::CHANNEL,
            steps: vec![
                Step::new(
                    P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
                    PropertyValue::List(vec![PropertyValue::ApplicationData(reference())]),
                ),
                Step::new(
                    P::EXECUTION_DELAY,
                    PropertyValue::List(vec![PropertyValue::Unsigned(5)]),
                ),
            ],
        },
    ]);
    scenarios
}

/// Writable rows that take only the value they already hold here, each
/// with the reason.
pub const IDENTITY_ONLY: &[(T, P, &str)] = &[
    (
        T::ACCESS_POINT,
        P::ACTIVE_AUTHENTICATION_POLICY,
        "the catalogue point has one authentication policy, so 1 is the only usable one",
    ),
    (
        T::ACCESS_POINT,
        P::AUTHORIZATION_MODE,
        "the catalogue point supports AUTHORIZE alone",
    ),
    (
        T::LOAD_CONTROL,
        P::SHED_LEVELS,
        "the catalogue Load Control has no shed levels, and a client can't change their number",
    ),
];

/// Clocked table entries no write here can reach, each with the reason.
/// Empty: the sweep and the scenarios use every entry.
pub const UNREACHABLE: &[(T, P, P, &str)] = &[];
