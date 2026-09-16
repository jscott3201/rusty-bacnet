//! Guard against wrong normative-clause anchors in rustdoc / CHANGELOG (#184).
//!
//! PR-0904 anchor sweep: each cited file is embedded with `include_str!` and
//! checked for its exact known-wrong strings (denylist) plus the corrected
//! anchors (presence). Doc comments only — no behavior is exercised here.

const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");
const ACCUMULATOR: &str = include_str!("../../../crates/bacnet-objects/src/accumulator/mod.rs");
const LOOP_OBJ: &str = include_str!("../../../crates/bacnet-objects/src/loop_obj.rs");
const CONSTRUCTED: &str = include_str!("../../../crates/bacnet-types/src/constructed/mod.rs");
const SCHEDULE_CODECS: &str = include_str!("../../../crates/bacnet-services/src/schedule.rs");
const ACCESS_ENUMS: &str = include_str!("../../../crates/bacnet-types/src/enums/access.rs");
const NOTIFICATION_CLASS: &str =
    include_str!("../../../crates/bacnet-objects/src/notification_class/mod.rs");
const MISC_ENUMS: &str = include_str!("../../../crates/bacnet-types/src/enums/misc.rs");
const COLOR_OBJ: &str = include_str!("../../../crates/bacnet-objects/src/color/mod.rs");
const LIFE_SAFETY_ENUMS: &str =
    include_str!("../../../crates/bacnet-types/src/enums/life_safety.rs");
const TIMER_OBJ: &str = include_str!("../../../crates/bacnet-objects/src/timer.rs");
const LOAD_CONTROL_OBJ: &str = include_str!("../../../crates/bacnet-objects/src/load_control.rs");
const PROGRAM_OBJ: &str = include_str!("../../../crates/bacnet-objects/src/program.rs");
const OBJECT_LEVEL_ENUMS: &str =
    include_str!("../../../crates/bacnet-types/src/enums/object_level.rs");
const FAULT_PARAMETER: &str =
    include_str!("../../../crates/bacnet-types/src/constructed/event_parameter/fault.rs");
const NETWORK_PORT_ENUMS: &str =
    include_str!("../../../crates/bacnet-types/src/enums/network_port.rs");
const ALARM_EVENT: &str = include_str!("../../../crates/bacnet-services/src/alarm_event/mod.rs");

fn assert_anchors(path: &str, src: &str, wrong: &[&str], right: &[&str]) {
    for w in wrong {
        assert!(
            !src.contains(w),
            "{path} still cites wrong anchor {w:?} (see #184)"
        );
    }
    for r in right {
        assert!(
            src.contains(r),
            "{path} missing corrected anchor {r:?} (see #184)"
        );
    }
}

#[test]
fn changelog_anchors_corrected_in_place() {
    assert_anchors(
        "CHANGELOG.md",
        CHANGELOG,
        &["§13.2.1", "§12.15.5", "(ASHRAE 135-2020 Clause 13.5)"],
        &[
            "(ASHRAE 135-2020 Clause 12.21)",
            "Clause 12.21 / Clause 21.6",
            "Clause 12.12 (Event_Parameters) / Clause 21.6 (BACnetEventParameter)",
        ],
    );
}

#[test]
fn accumulator_pulse_converter_tables() {
    assert_anchors(
        "crates/bacnet-objects/src/accumulator/mod.rs",
        ACCUMULATOR,
        &["Clauses 12.1 (Accumulator)", "12.2 (PulseConverter)"],
        &["Table 12-79", "Table 12-27"],
    );
}

#[test]
fn loop_object_clause() {
    assert_anchors(
        "crates/bacnet-objects/src/loop_obj.rs",
        LOOP_OBJ,
        &["Clause 12.19"],
        &["Loop (type 12) object per ASHRAE 135-2020 Clause 12.17"],
    );
}

#[test]
fn constructed_calendar_schedule_anchors() {
    assert_anchors(
        "crates/bacnet-types/src/constructed/mod.rs",
        CONSTRUCTED,
        &["12.6.3", "12.17.4", "12.17.5"],
        &["Clause 12.24", "Clause 12.9", "Clause 21.6"],
    );
}

#[test]
fn constructed_trendlog_anchor() {
    assert_anchors(
        "crates/bacnet-types/src/constructed/mod.rs",
        CONSTRUCTED,
        &["12.20.5"],
        &["Clause 12.25"],
    );
}

#[test]
fn constructed_fault_parameters_anchor() {
    assert_anchors(
        "crates/bacnet-types/src/constructed/mod.rs",
        CONSTRUCTED,
        &["12.12.50"],
        &["Clause 12.12 -- Fault_Parameters"],
    );
}

#[test]
fn schedule_service_codecs_clause() {
    assert_anchors(
        "crates/bacnet-services/src/schedule.rs",
        SCHEDULE_CODECS,
        &["Clauses 12.17, 21"],
        &["Clauses 12.24, 21"],
    );
}

#[test]
fn access_user_type_clause() {
    assert_anchors(
        "crates/bacnet-types/src/enums/access.rs",
        ACCESS_ENUMS,
        &["Clause 12.35"],
        &["access user type (Clause 12.33"],
    );
}

#[test]
fn notification_class_priority_ack_clause() {
    assert_anchors(
        "crates/bacnet-objects/src/notification_class/mod.rs",
        NOTIFICATION_CLASS,
        &["Clause 13.2.1"],
        &["Per ASHRAE 135-2020 Clause 12.21"],
    );
}

#[test]
fn event_transition_bits_production() {
    assert_anchors(
        "crates/bacnet-types/src/enums/misc.rs",
        MISC_ENUMS,
        &["(Clause 12.11)"],
        &["transition bit positions (Clause 21.6"],
    );
}

#[test]
fn color_addendum_claims_no_base_numbers() {
    assert_anchors(
        "crates/bacnet-objects/src/color/mod.rs",
        COLOR_OBJ,
        &["12.55-12.56"],
        &["Addendum bj"],
    );
}

#[test]
fn life_safety_operation_service_anchor() {
    assert_anchors(
        "crates/bacnet-types/src/enums/life_safety.rs",
        LIFE_SAFETY_ENUMS,
        &["12.15.13", "Table 12-54"],
        &["Clauses 13.13, 12.15"],
    );
}

#[test]
fn life_safety_mode_silenced_clauses() {
    assert_anchors(
        "crates/bacnet-types/src/enums/life_safety.rs",
        LIFE_SAFETY_ENUMS,
        &["12.15.12", "12.15.14"],
        &["Clause 12.15 (Mode)", "Clause 12.15 (Silenced)"],
    );
}

#[test]
fn timer_object_clause() {
    assert_anchors(
        "crates/bacnet-objects/src/timer.rs",
        TIMER_OBJ,
        &["Clause 12.\n"],
        &["Clause 12.57"],
    );
}

#[test]
fn load_control_object_clause() {
    assert_anchors(
        "crates/bacnet-objects/src/load_control.rs",
        LOAD_CONTROL_OBJ,
        &["Clause 12.\n"],
        &["Clause 12.28"],
    );
}

#[test]
fn program_object_clause() {
    assert_anchors(
        "crates/bacnet-objects/src/program.rs",
        PROGRAM_OBJ,
        &["Clause 12.\n"],
        &["Clause 12.22"],
    );
}

#[test]
fn device_status_event_logging_type_clauses() {
    assert_anchors(
        "crates/bacnet-types/src/enums/object_level.rs",
        OBJECT_LEVEL_ENUMS,
        &["12.11.9", "12.12.6", "12.25.14"],
        &[
            "Clause 12.11 (System_Status)",
            "Clause 12.12 (Event_Type)",
            "Clause 12.25 (Logging_Type)",
        ],
    );
}

#[test]
fn fault_parameter_roundtrip_anchor() {
    assert_anchors(
        "crates/bacnet-types/src/constructed/event_parameter/fault.rs",
        FAULT_PARAMETER,
        &["12.12.50"],
        &["Clause 12.12 -- Fault_Parameters"],
    );
}

#[test]
fn network_port_enum_clauses() {
    assert_anchors(
        "crates/bacnet-types/src/enums/network_port.rs",
        NETWORK_PORT_ENUMS,
        &["12.56.44", "12.56.40", "12.56.42"],
        &[
            "Clause 12.56 (Network_Type)",
            "Clause 12.56 (Command)",
            "Clause 12.56 (Network_Number_Quality)",
        ],
    );
}

#[test]
fn alarm_event_service_anchors_present() {
    assert_anchors(
        "crates/bacnet-services/src/alarm_event/mod.rs",
        ALARM_EVENT,
        &[],
        &[
            "AcknowledgeAlarm acknowledges an event transition (Clause 13.5)",
            "Clauses 13.8, 13.9",
            "GetEventInformation retrieves event summaries (Clause 13.12)",
        ],
    );
}
