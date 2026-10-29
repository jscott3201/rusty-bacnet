//! Lighting Output's warn commands (#1384; Table 12-67, Clauses 12.54.6.2 and
//! 12.54.13), written to Lighting_Command or as the Present_Value special
//! values -1.0, -2.0 and -3.0 (Table 12-65): both ways behave alike.

use super::engine_tests::{Fixture, op};
use super::*;
use bacnet_types::enums::LightingOperation as Op;

/// The two ways to ask for a warn command.
#[derive(Debug, Clone, Copy)]
enum Path {
    LightingCommand,
    PresentValue,
}

const PATHS: [Path; 2] = [Path::LightingCommand, Path::PresentValue];

impl Fixture {
    fn warn(&mut self, path: Path, operation: Op, priority: u8) {
        match path {
            Path::LightingCommand => self.command(op(operation, Some(priority))),
            Path::PresentValue => {
                let value = match operation {
                    Op::WARN => -1.0,
                    Op::WARN_RELINQUISH => -2.0,
                    Op::WARN_OFF => -3.0,
                    other => panic!("{other:?} has no Present_Value form"),
                };
                self.present(PropertyValue::Real(value), priority);
            }
        }
    }

    fn blinks(&self) -> u64 {
        self.lo.lighting_blink_count_internal()
    }

    /// Blink_Warn_Enable TRUE with `seconds` of Egress_Time.
    fn blink_warn(&mut self, seconds: u64) {
        self.write(
            PropertyIdentifier::BLINK_WARN_ENABLE,
            PropertyValue::Boolean(true),
        );
        self.write(
            PropertyIdentifier::EGRESS_TIME,
            PropertyValue::Unsigned(seconds),
        );
    }

    fn lit_at_8() -> Self {
        let mut f = Self::new();
        f.present(PropertyValue::Real(80.0), 8);
        f
    }
}

#[test]
fn with_blink_warn_disabled_warn_commands_take_effect_at_once() {
    for path in PATHS {
        let mut f = Fixture::lit_at_8();
        // Egress_Time is set, but Blink_Warn_Enable is FALSE by default.
        f.write(PropertyIdentifier::EGRESS_TIME, PropertyValue::Unsigned(30));
        f.warn(path, Op::WARN, 8);
        assert_eq!((f.slot(8), f.pv()), (Some(80.0), 80.0), "{path:?}");
        f.warn(path, Op::WARN_OFF, 8);
        assert_eq!((f.slot(8), f.pv()), (Some(0.0), 0.0), "{path:?}");
        f.present(PropertyValue::Real(80.0), 8);
        f.warn(path, Op::WARN_RELINQUISH, 8);
        assert_eq!((f.slot(8), f.pv()), (None, 0.0), "{path:?}");
        assert_eq!(
            (f.blinks(), f.egress_active(), f.deadline()),
            (0, false, None)
        );
    }
}

#[test]
fn warn_off_blinks_and_turns_the_slot_off_once_egress_time_runs_out() {
    for path in PATHS {
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.at(1_000);
        f.warn(path, Op::WARN_OFF, 8);
        assert_eq!((f.blinks(), f.egress_active()), (1, true), "{path:?}");
        assert_eq!((f.slot(8), f.pv(), f.tv()), (Some(80.0), 80.0, 80.0));
        assert_eq!(f.deadline(), Some(Duration::from_secs(11)));
        f.at(10_999);
        assert!(!f.advance());
        f.at(11_000);
        assert!(f.advance());
        assert_eq!(
            (f.slot(8), f.pv(), f.egress_active()),
            (Some(0.0), 0.0, false)
        );
        assert_eq!(f.deadline(), None);
    }
}

#[test]
fn warn_relinquish_blinks_and_relinquishes_once_egress_time_runs_out() {
    for path in PATHS {
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.warn(path, Op::WARN_RELINQUISH, 8);
        assert_eq!((f.blinks(), f.egress_active()), (1, true), "{path:?}");
        assert_eq!(f.slot(8), Some(80.0));
        f.at(10_000);
        assert!(f.advance());
        assert_eq!((f.slot(8), f.pv(), f.egress_active()), (None, 0.0, false));
    }
}

#[test]
fn warn_blinks_only_at_the_highest_priority_when_it_is_lit() {
    for path in PATHS {
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.warn(path, Op::WARN, 8);
        assert_eq!(f.blinks(), 1, "{path:?}");
        // Priority 10 isn't the highest in use, and an off slot isn't lit.
        f.warn(path, Op::WARN, 10);
        f.present(PropertyValue::Real(0.0), 8);
        f.warn(path, Op::WARN, 8);
        assert_eq!(f.blinks(), 1, "{path:?}");
        assert_eq!((f.slot(8), f.slot(10)), (Some(0.0), None));
        assert_eq!((f.egress_active(), f.deadline()), (false, None));
    }
}

#[test]
fn warn_relinquish_acts_at_once_where_the_table_says_so() {
    for path in PATHS {
        // A lit slot below would keep the light on: no warning needed.
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.present(PropertyValue::Real(20.0), 12);
        f.warn(path, Op::WARN_RELINQUISH, 8);
        assert_eq!((f.slot(8), f.pv()), (None, 20.0), "{path:?}");
        // So would a lit Relinquish_Default.
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.lo.set_relinquish_default(30.0).unwrap();
        f.warn(path, Op::WARN_RELINQUISH, 8);
        assert_eq!((f.slot(8), f.pv()), (None, 30.0), "{path:?}");
        // Not the highest priority in use.
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.present(PropertyValue::Real(50.0), 4);
        f.warn(path, Op::WARN_RELINQUISH, 8);
        assert_eq!((f.slot(8), f.pv()), (None, 50.0), "{path:?}");
        // An off slot.
        let mut f = Fixture::new();
        f.blink_warn(10);
        f.present(PropertyValue::Real(0.0), 8);
        f.warn(path, Op::WARN_RELINQUISH, 8);
        assert_eq!(f.slot(8), None, "{path:?}");
        assert_eq!((f.blinks(), f.egress_active()), (0, false), "{path:?}");
    }
}

#[test]
fn warn_off_acts_at_once_where_the_table_says_so() {
    for path in PATHS {
        // Already off.
        let mut f = Fixture::new();
        f.blink_warn(10);
        f.present(PropertyValue::Real(0.0), 8);
        f.warn(path, Op::WARN_OFF, 8);
        assert_eq!((f.slot(8), f.pv()), (Some(0.0), 0.0), "{path:?}");
        // Not the highest priority in use.
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.present(PropertyValue::Real(50.0), 4);
        f.warn(path, Op::WARN_OFF, 8);
        assert_eq!((f.slot(8), f.pv()), (Some(0.0), 50.0), "{path:?}");
        assert_eq!((f.blinks(), f.egress_active()), (0, false), "{path:?}");
    }
}

#[test]
fn with_no_egress_time_a_warn_blinks_and_acts_at_once() {
    for path in PATHS {
        let mut f = Fixture::lit_at_8();
        f.blink_warn(0);
        f.warn(path, Op::WARN_OFF, 8);
        assert_eq!((f.blinks(), f.egress_active()), (1, false), "{path:?}");
        assert_eq!((f.slot(8), f.deadline()), (Some(0.0), None));
    }
}

#[test]
fn a_halted_egress_gives_its_slot_its_final_value_at_once() {
    for path in PATHS {
        // A higher-priority write cuts a WARN_RELINQUISH short.
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.warn(path, Op::WARN_RELINQUISH, 8);
        f.present(PropertyValue::Real(50.0), 4);
        assert_eq!((f.slot(8), f.pv()), (None, 50.0), "{path:?}");
        assert_eq!((f.egress_active(), f.deadline()), (false, None));

        // A same-priority command cuts a WARN_OFF short: 0.0 goes in, then
        // the new command acts, fading from the level that was held.
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.warn(path, Op::WARN_OFF, 8);
        f.command(BACnetLightingCommand {
            target_level: Some(60.0),
            fade_time: Some(1_000),
            ..op(Op::FADE_TO, Some(8))
        });
        assert_eq!(
            (f.slot(8), f.tv(), f.egress_active()),
            (Some(60.0), 80.0, false)
        );

        // A lower-priority write doesn't: the egress runs out as before, and
        // the slot below shows once priority 8 is relinquished.
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.warn(path, Op::WARN_RELINQUISH, 8);
        f.present(PropertyValue::Real(30.0), 12);
        assert!(f.egress_active(), "{path:?}");
        f.at(10_000);
        assert!(f.advance());
        assert_eq!((f.slot(8), f.pv()), (None, 30.0), "{path:?}");
    }
}

#[test]
fn a_step_during_an_egress_steps_from_the_held_level() {
    for operation in [Op::WARN_OFF, Op::WARN_RELINQUISH] {
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.warn(Path::LightingCommand, operation, 8);
        f.command(BACnetLightingCommand {
            step_increment: Some(10.0),
            ..op(Op::STEP_UP, Some(8))
        });
        assert_eq!((f.slot(8), f.pv()), (Some(90.0), 90.0), "{operation:?}");
        assert_eq!((f.egress_active(), f.deadline()), (false, None));
    }
}

#[test]
fn stop_cancels_an_egress_at_its_priority_and_leaves_the_slot() {
    for path in PATHS {
        let mut f = Fixture::lit_at_8();
        f.blink_warn(10);
        f.warn(path, Op::WARN_OFF, 8);
        // A STOP at another priority is ignored.
        f.command(op(Op::STOP, Some(4)));
        assert!(f.egress_active(), "{path:?}");
        f.command(op(Op::STOP, Some(8)));
        assert_eq!((f.egress_active(), f.deadline()), (false, None), "{path:?}");
        f.at(60_000);
        assert!(!f.advance());
        assert_eq!((f.slot(8), f.pv()), (Some(80.0), 80.0), "{path:?}");
    }
}

#[test]
fn present_value_warn_values_stay_out_of_the_priority_array_and_lighting_command() {
    let mut f = Fixture::new();
    f.blink_warn(10);
    for value in [-1.0, -2.0, -3.0] {
        // No priority: 16, as for any Present_Value write.
        f.lo.write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(value),
            None,
            &crate::command_source::test_origin(),
        )
        .unwrap();
        for priority in 1..=16 {
            assert_ne!(f.slot(priority), Some(value), "{value} at {priority}");
        }
    }
    assert_eq!(f.slot(16), Some(0.0));
    assert_eq!(
        f.lo.lighting_command(),
        BACnetLightingCommand::new(Op::NONE)
    );
}
