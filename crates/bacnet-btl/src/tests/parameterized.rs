//! Parameterized test registration — generates tests across object types.
//!
//! This module registers the same test logic for every applicable object type,
//! implementing the BTL Test Plan's per-object-type test coverage.

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;

/// Register all parameterized tests across applicable object types.
pub fn register(registry: &mut TestRegistry) {
    register_oos_tests(registry);
    register_command_prioritization_tests(registry);
    register_relinquish_default_tests(registry);
    register_cov_tests(registry);
    register_event_reporting_tests(registry);
    register_rei_tests(registry);
    register_oos_commandable_tests(registry);
    register_value_source_tests(registry);
}

// ═══════════════════════════════════════════════════════════════════════════
// Pattern 1: Out_Of_Service / Status_Flags per object type
// BTL 7.3.1.1.1 — applies to ~32 object types
// ═══════════════════════════════════════════════════════════════════════════

fn register_oos_tests(registry: &mut TestRegistry) {
    // Use a macro to avoid closure capture issues (fn pointers can't capture)
    macro_rules! oos_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("OOS/SF: ", $abbr, " Out_Of_Service and Status_Flags"),
                reference: "BTL 7.3.1.1.1",
                section: Section::Objects,
                tags: &["parameterized", "oos", "status-flags"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, $ot)),
            });
        };
    }

    oos_test!(
        registry,
        "P1.1",
        "AI",
        bacnet_types::enums::ObjectType::ANALOG_INPUT
    );
    oos_test!(
        registry,
        "P1.2",
        "AO",
        bacnet_types::enums::ObjectType::ANALOG_OUTPUT
    );
    oos_test!(
        registry,
        "P1.3",
        "AV",
        bacnet_types::enums::ObjectType::ANALOG_VALUE
    );
    oos_test!(
        registry,
        "P1.4",
        "BI",
        bacnet_types::enums::ObjectType::BINARY_INPUT
    );
    oos_test!(
        registry,
        "P1.5",
        "BO",
        bacnet_types::enums::ObjectType::BINARY_OUTPUT
    );
    oos_test!(
        registry,
        "P1.6",
        "BV",
        bacnet_types::enums::ObjectType::BINARY_VALUE
    );
    oos_test!(
        registry,
        "P1.7",
        "MSI",
        bacnet_types::enums::ObjectType::MULTI_STATE_INPUT
    );
    oos_test!(
        registry,
        "P1.8",
        "MSO",
        bacnet_types::enums::ObjectType::MULTI_STATE_OUTPUT
    );
    oos_test!(
        registry,
        "P1.9",
        "MSV",
        bacnet_types::enums::ObjectType::MULTI_STATE_VALUE
    );
    oos_test!(
        registry,
        "P1.10",
        "LSP",
        bacnet_types::enums::ObjectType::LIFE_SAFETY_POINT
    );
    oos_test!(
        registry,
        "P1.11",
        "LSZ",
        bacnet_types::enums::ObjectType::LIFE_SAFETY_ZONE
    );
    oos_test!(
        registry,
        "P1.12",
        "ACC",
        bacnet_types::enums::ObjectType::ACCUMULATOR
    );
    oos_test!(
        registry,
        "P1.13",
        "PC",
        bacnet_types::enums::ObjectType::PULSE_CONVERTER
    );
    oos_test!(
        registry,
        "P1.14",
        "LP",
        bacnet_types::enums::ObjectType::LOOP
    );
    oos_test!(
        registry,
        "P1.15",
        "AD",
        bacnet_types::enums::ObjectType::ACCESS_DOOR
    );
    oos_test!(
        registry,
        "P1.16",
        "CH",
        bacnet_types::enums::ObjectType::CHANNEL
    );
    oos_test!(
        registry,
        "P1.17",
        "LO",
        bacnet_types::enums::ObjectType::LIGHTING_OUTPUT
    );
    oos_test!(
        registry,
        "P1.18",
        "BLO",
        bacnet_types::enums::ObjectType::BINARY_LIGHTING_OUTPUT
    );
    oos_test!(
        registry,
        "P1.19",
        "STG",
        bacnet_types::enums::ObjectType::STAGING
    );
    oos_test!(
        registry,
        "P1.20",
        "NP",
        bacnet_types::enums::ObjectType::NETWORK_PORT
    );
    oos_test!(
        registry,
        "P1.21",
        "NF",
        bacnet_types::enums::ObjectType::NOTIFICATION_FORWARDER
    );
    oos_test!(
        registry,
        "P1.22",
        "IV",
        bacnet_types::enums::ObjectType::INTEGER_VALUE
    );
    oos_test!(
        registry,
        "P1.23",
        "PIV",
        bacnet_types::enums::ObjectType::POSITIVE_INTEGER_VALUE
    );
    oos_test!(
        registry,
        "P1.24",
        "LAV",
        bacnet_types::enums::ObjectType::LARGE_ANALOG_VALUE
    );
    oos_test!(
        registry,
        "P1.25",
        "CSV",
        bacnet_types::enums::ObjectType::CHARACTERSTRING_VALUE
    );
    oos_test!(
        registry,
        "P1.26",
        "OSV",
        bacnet_types::enums::ObjectType::OCTETSTRING_VALUE
    );
    oos_test!(
        registry,
        "P1.27",
        "BSV",
        bacnet_types::enums::ObjectType::BITSTRING_VALUE
    );
    oos_test!(
        registry,
        "P1.28",
        "DV",
        bacnet_types::enums::ObjectType::DATE_VALUE
    );
    oos_test!(
        registry,
        "P1.29",
        "TV",
        bacnet_types::enums::ObjectType::TIME_VALUE
    );
    oos_test!(
        registry,
        "P1.30",
        "DTV",
        bacnet_types::enums::ObjectType::DATETIME_VALUE
    );
    oos_test!(
        registry,
        "P1.31",
        "DPV",
        bacnet_types::enums::ObjectType::DATEPATTERN_VALUE
    );
    oos_test!(
        registry,
        "P1.32",
        "TPV",
        bacnet_types::enums::ObjectType::TIMEPATTERN_VALUE
    );
    // Color/ColorTemperature OOS tests are in s03_objects/color.rs (3.65.1, 3.65.19)
}

// ═══════════════════════════════════════════════════════════════════════════
// Pattern 2: Command Prioritization per commandable type
// 135.1-2025 7.3.1.3 — applies to ~18 commandable types
// ═══════════════════════════════════════════════════════════════════════════

fn register_command_prioritization_tests(registry: &mut TestRegistry) {
    macro_rules! cmd_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("CMD: ", $abbr, " Command Prioritization"),
                reference: "135.1-2025 7.3.1.3",
                section: Section::Objects,
                tags: &["parameterized", "command-priority"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_command_prioritization(ctx, $ot)),
            });
        };
    }

    cmd_test!(
        registry,
        "P2.1",
        "AO",
        bacnet_types::enums::ObjectType::ANALOG_OUTPUT
    );
    cmd_test!(
        registry,
        "P2.2",
        "AV",
        bacnet_types::enums::ObjectType::ANALOG_VALUE
    );
    cmd_test!(
        registry,
        "P2.3",
        "BO",
        bacnet_types::enums::ObjectType::BINARY_OUTPUT
    );
    cmd_test!(
        registry,
        "P2.4",
        "BV",
        bacnet_types::enums::ObjectType::BINARY_VALUE
    );
    cmd_test!(
        registry,
        "P2.5",
        "MSO",
        bacnet_types::enums::ObjectType::MULTI_STATE_OUTPUT
    );
    cmd_test!(
        registry,
        "P2.6",
        "MSV",
        bacnet_types::enums::ObjectType::MULTI_STATE_VALUE
    );
    cmd_test!(
        registry,
        "P2.7",
        "AD",
        bacnet_types::enums::ObjectType::ACCESS_DOOR
    );
    cmd_test!(
        registry,
        "P2.8",
        "LO",
        bacnet_types::enums::ObjectType::LIGHTING_OUTPUT
    );
    cmd_test!(
        registry,
        "P2.9",
        "BLO",
        bacnet_types::enums::ObjectType::BINARY_LIGHTING_OUTPUT
    );
    cmd_test!(
        registry,
        "P2.10",
        "IV",
        bacnet_types::enums::ObjectType::INTEGER_VALUE
    );
    cmd_test!(
        registry,
        "P2.11",
        "PIV",
        bacnet_types::enums::ObjectType::POSITIVE_INTEGER_VALUE
    );
    cmd_test!(
        registry,
        "P2.12",
        "LAV",
        bacnet_types::enums::ObjectType::LARGE_ANALOG_VALUE
    );
    cmd_test!(
        registry,
        "P2.13",
        "CSV",
        bacnet_types::enums::ObjectType::CHARACTERSTRING_VALUE
    );
    cmd_test!(
        registry,
        "P2.14",
        "OSV",
        bacnet_types::enums::ObjectType::OCTETSTRING_VALUE
    );
    cmd_test!(
        registry,
        "P2.15",
        "BSV",
        bacnet_types::enums::ObjectType::BITSTRING_VALUE
    );
    cmd_test!(
        registry,
        "P2.16",
        "DV",
        bacnet_types::enums::ObjectType::DATE_VALUE
    );
    cmd_test!(
        registry,
        "P2.17",
        "TV",
        bacnet_types::enums::ObjectType::TIME_VALUE
    );
    cmd_test!(
        registry,
        "P2.18",
        "DTV",
        bacnet_types::enums::ObjectType::DATETIME_VALUE
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Pattern 3: Relinquish Default per commandable type
// 135.1-2025 7.3.1.2 — applies to same ~18 types
// ═══════════════════════════════════════════════════════════════════════════

fn register_relinquish_default_tests(registry: &mut TestRegistry) {
    macro_rules! rd_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("RD: ", $abbr, " Relinquish Default"),
                reference: "135.1-2025 7.3.1.2",
                section: Section::Objects,
                tags: &["parameterized", "relinquish-default"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_relinquish_default(ctx, $ot)),
            });
        };
    }

    rd_test!(
        registry,
        "P3.1",
        "AO",
        bacnet_types::enums::ObjectType::ANALOG_OUTPUT
    );
    rd_test!(
        registry,
        "P3.2",
        "AV",
        bacnet_types::enums::ObjectType::ANALOG_VALUE
    );
    rd_test!(
        registry,
        "P3.3",
        "BO",
        bacnet_types::enums::ObjectType::BINARY_OUTPUT
    );
    rd_test!(
        registry,
        "P3.4",
        "BV",
        bacnet_types::enums::ObjectType::BINARY_VALUE
    );
    rd_test!(
        registry,
        "P3.5",
        "MSO",
        bacnet_types::enums::ObjectType::MULTI_STATE_OUTPUT
    );
    rd_test!(
        registry,
        "P3.6",
        "MSV",
        bacnet_types::enums::ObjectType::MULTI_STATE_VALUE
    );
    rd_test!(
        registry,
        "P3.7",
        "AD",
        bacnet_types::enums::ObjectType::ACCESS_DOOR
    );
    rd_test!(
        registry,
        "P3.8",
        "LO",
        bacnet_types::enums::ObjectType::LIGHTING_OUTPUT
    );
    rd_test!(
        registry,
        "P3.9",
        "BLO",
        bacnet_types::enums::ObjectType::BINARY_LIGHTING_OUTPUT
    );
    rd_test!(
        registry,
        "P3.10",
        "IV",
        bacnet_types::enums::ObjectType::INTEGER_VALUE
    );
    rd_test!(
        registry,
        "P3.11",
        "PIV",
        bacnet_types::enums::ObjectType::POSITIVE_INTEGER_VALUE
    );
    rd_test!(
        registry,
        "P3.12",
        "LAV",
        bacnet_types::enums::ObjectType::LARGE_ANALOG_VALUE
    );
    rd_test!(
        registry,
        "P3.13",
        "CSV",
        bacnet_types::enums::ObjectType::CHARACTERSTRING_VALUE
    );
    rd_test!(
        registry,
        "P3.14",
        "OSV",
        bacnet_types::enums::ObjectType::OCTETSTRING_VALUE
    );
    rd_test!(
        registry,
        "P3.15",
        "BSV",
        bacnet_types::enums::ObjectType::BITSTRING_VALUE
    );
    rd_test!(
        registry,
        "P3.16",
        "DV",
        bacnet_types::enums::ObjectType::DATE_VALUE
    );
    rd_test!(
        registry,
        "P3.17",
        "TV",
        bacnet_types::enums::ObjectType::TIME_VALUE
    );
    rd_test!(
        registry,
        "P3.18",
        "DTV",
        bacnet_types::enums::ObjectType::DATETIME_VALUE
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Pattern 5: COV Subscription per object type
// 135.1-2025 9.2.1.1 — applies to ~20 COV-capable types
// ═══════════════════════════════════════════════════════════════════════════

fn register_cov_tests(registry: &mut TestRegistry) {
    macro_rules! cov_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("COV: ", $abbr, " Subscribe COV"),
                reference: "135.1-2025 9.2.1.1",
                section: Section::DataSharing,
                tags: &["parameterized", "cov", "subscribe"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_cov_subscribe(ctx, $ot)),
            });
        };
    }

    // Types that currently implement supports_cov() = true in our stack:
    cov_test!(
        registry,
        "P5.1",
        "AI",
        bacnet_types::enums::ObjectType::ANALOG_INPUT
    );
    cov_test!(
        registry,
        "P5.2",
        "AO",
        bacnet_types::enums::ObjectType::ANALOG_OUTPUT
    );
    cov_test!(
        registry,
        "P5.3",
        "AV",
        bacnet_types::enums::ObjectType::ANALOG_VALUE
    );
    cov_test!(
        registry,
        "P5.4",
        "BI",
        bacnet_types::enums::ObjectType::BINARY_INPUT
    );
    cov_test!(
        registry,
        "P5.5",
        "BO",
        bacnet_types::enums::ObjectType::BINARY_OUTPUT
    );
    cov_test!(
        registry,
        "P5.6",
        "BV",
        bacnet_types::enums::ObjectType::BINARY_VALUE
    );
    cov_test!(
        registry,
        "P5.7",
        "MSI",
        bacnet_types::enums::ObjectType::MULTI_STATE_INPUT
    );
    cov_test!(
        registry,
        "P5.8",
        "MSO",
        bacnet_types::enums::ObjectType::MULTI_STATE_OUTPUT
    );
    cov_test!(
        registry,
        "P5.9",
        "MSV",
        bacnet_types::enums::ObjectType::MULTI_STATE_VALUE
    );
    cov_test!(
        registry,
        "P5.10",
        "LSP",
        bacnet_types::enums::ObjectType::LIFE_SAFETY_POINT
    );
    cov_test!(
        registry,
        "P5.11",
        "LSZ",
        bacnet_types::enums::ObjectType::LIFE_SAFETY_ZONE
    );
    cov_test!(
        registry,
        "P5.12",
        "AD",
        bacnet_types::enums::ObjectType::ACCESS_DOOR
    );
    cov_test!(
        registry,
        "P5.13",
        "LP",
        bacnet_types::enums::ObjectType::LOOP
    );
    cov_test!(
        registry,
        "P5.14",
        "ACC",
        bacnet_types::enums::ObjectType::ACCUMULATOR
    );
    cov_test!(
        registry,
        "P5.15",
        "PC",
        bacnet_types::enums::ObjectType::PULSE_CONVERTER
    );
    cov_test!(
        registry,
        "P5.16",
        "LO",
        bacnet_types::enums::ObjectType::LIGHTING_OUTPUT
    );
    cov_test!(
        registry,
        "P5.17",
        "BLO",
        bacnet_types::enums::ObjectType::BINARY_LIGHTING_OUTPUT
    );
    cov_test!(
        registry,
        "P5.18",
        "STG",
        bacnet_types::enums::ObjectType::STAGING
    );
    cov_test!(
        registry,
        "P5.19",
        "IV",
        bacnet_types::enums::ObjectType::INTEGER_VALUE
    );
    cov_test!(
        registry,
        "P5.20",
        "LAV",
        bacnet_types::enums::ObjectType::LARGE_ANALOG_VALUE
    );
    cov_test!(
        registry,
        "P5.21",
        "CLR",
        bacnet_types::enums::ObjectType::COLOR
    );
    cov_test!(
        registry,
        "P5.22",
        "CT",
        bacnet_types::enums::ObjectType::COLOR_TEMPERATURE
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Event Reporting properties per reporting type
// ═══════════════════════════════════════════════════════════════════════════

fn register_event_reporting_tests(registry: &mut TestRegistry) {
    macro_rules! event_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("EVT: ", $abbr, " Event_State Normal"),
                reference: "135.1-2025 12.1",
                section: Section::AlarmAndEvent,
                tags: &["parameterized", "event-state"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_event_state_normal(ctx, $ot)),
            });
        };
    }

    event_test!(
        registry,
        "P6.1",
        "AI",
        bacnet_types::enums::ObjectType::ANALOG_INPUT
    );
    event_test!(
        registry,
        "P6.2",
        "AO",
        bacnet_types::enums::ObjectType::ANALOG_OUTPUT
    );
    event_test!(
        registry,
        "P6.3",
        "AV",
        bacnet_types::enums::ObjectType::ANALOG_VALUE
    );
    event_test!(
        registry,
        "P6.4",
        "BI",
        bacnet_types::enums::ObjectType::BINARY_INPUT
    );
    event_test!(
        registry,
        "P6.5",
        "BO",
        bacnet_types::enums::ObjectType::BINARY_OUTPUT
    );
    event_test!(
        registry,
        "P6.6",
        "BV",
        bacnet_types::enums::ObjectType::BINARY_VALUE
    );
    event_test!(
        registry,
        "P6.7",
        "MSI",
        bacnet_types::enums::ObjectType::MULTI_STATE_INPUT
    );
    event_test!(
        registry,
        "P6.8",
        "MSO",
        bacnet_types::enums::ObjectType::MULTI_STATE_OUTPUT
    );
    event_test!(
        registry,
        "P6.9",
        "MSV",
        bacnet_types::enums::ObjectType::MULTI_STATE_VALUE
    );
    event_test!(
        registry,
        "P6.10",
        "LSP",
        bacnet_types::enums::ObjectType::LIFE_SAFETY_POINT
    );
    event_test!(
        registry,
        "P6.11",
        "LSZ",
        bacnet_types::enums::ObjectType::LIFE_SAFETY_ZONE
    );
    event_test!(
        registry,
        "P6.12",
        "AD",
        bacnet_types::enums::ObjectType::ACCESS_DOOR
    );
    event_test!(
        registry,
        "P6.13",
        "LP",
        bacnet_types::enums::ObjectType::LOOP
    );
    event_test!(
        registry,
        "P6.14",
        "ACC",
        bacnet_types::enums::ObjectType::ACCUMULATOR
    );
    event_test!(
        registry,
        "P6.15",
        "PC",
        bacnet_types::enums::ObjectType::PULSE_CONVERTER
    );
    event_test!(
        registry,
        "P6.16",
        "LC",
        bacnet_types::enums::ObjectType::LOAD_CONTROL
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Reliability_Evaluation_Inhibit (135.1-2025 7.3.1.21.3)
// Applies to ALL object types with Reliability property (~57 types)
// ═══════════════════════════════════════════════════════════════════════════

fn register_rei_tests(registry: &mut TestRegistry) {
    macro_rules! rei_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("REI: ", $abbr, " Reliability_Evaluation_Inhibit"),
                reference: "135.1-2025 - 7.3.1.21.3",
                section: Section::Objects,
                tags: &["parameterized", "rei"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, $ot)),
            });
        };
    }

    use bacnet_types::enums::ObjectType;
    rei_test!(registry, "P7.1", "AI", ObjectType::ANALOG_INPUT);
    rei_test!(registry, "P7.2", "AO", ObjectType::ANALOG_OUTPUT);
    rei_test!(registry, "P7.3", "AV", ObjectType::ANALOG_VALUE);
    rei_test!(registry, "P7.4", "BI", ObjectType::BINARY_INPUT);
    rei_test!(registry, "P7.5", "BO", ObjectType::BINARY_OUTPUT);
    rei_test!(registry, "P7.6", "BV", ObjectType::BINARY_VALUE);
    rei_test!(registry, "P7.7", "MSI", ObjectType::MULTI_STATE_INPUT);
    rei_test!(registry, "P7.8", "MSO", ObjectType::MULTI_STATE_OUTPUT);
    rei_test!(registry, "P7.9", "MSV", ObjectType::MULTI_STATE_VALUE);
    rei_test!(registry, "P7.10", "LSP", ObjectType::LIFE_SAFETY_POINT);
    rei_test!(registry, "P7.11", "LSZ", ObjectType::LIFE_SAFETY_ZONE);
    rei_test!(registry, "P7.12", "ACC", ObjectType::ACCUMULATOR);
    rei_test!(registry, "P7.13", "PC", ObjectType::PULSE_CONVERTER);
    rei_test!(registry, "P7.14", "LP", ObjectType::LOOP);
    rei_test!(registry, "P7.15", "AD", ObjectType::ACCESS_DOOR);
    rei_test!(registry, "P7.16", "LC", ObjectType::LOAD_CONTROL);
    rei_test!(registry, "P7.17", "CH", ObjectType::CHANNEL);
    rei_test!(registry, "P7.18", "LO", ObjectType::LIGHTING_OUTPUT);
    rei_test!(registry, "P7.19", "BLO", ObjectType::BINARY_LIGHTING_OUTPUT);
    rei_test!(registry, "P7.20", "STG", ObjectType::STAGING);
    rei_test!(registry, "P7.21", "NP", ObjectType::NETWORK_PORT);
    rei_test!(registry, "P7.22", "NF", ObjectType::NOTIFICATION_FORWARDER);
    rei_test!(registry, "P7.23", "IV", ObjectType::INTEGER_VALUE);
    rei_test!(registry, "P7.24", "PIV", ObjectType::POSITIVE_INTEGER_VALUE);
    rei_test!(registry, "P7.25", "LAV", ObjectType::LARGE_ANALOG_VALUE);
    rei_test!(registry, "P7.26", "CSV", ObjectType::CHARACTERSTRING_VALUE);
    rei_test!(registry, "P7.27", "OSV", ObjectType::OCTETSTRING_VALUE);
    rei_test!(registry, "P7.28", "BSV", ObjectType::BITSTRING_VALUE);
    rei_test!(registry, "P7.29", "DV", ObjectType::DATE_VALUE);
    rei_test!(registry, "P7.30", "TV", ObjectType::TIME_VALUE);
    rei_test!(registry, "P7.31", "DTV", ObjectType::DATETIME_VALUE);
    rei_test!(registry, "P7.32", "DPV", ObjectType::DATEPATTERN_VALUE);
    rei_test!(registry, "P7.33", "TPV", ObjectType::TIMEPATTERN_VALUE);
    rei_test!(registry, "P7.34", "DTPV", ObjectType::DATETIMEPATTERN_VALUE);
    // Color/ColorTemperature REI tests are in s03_objects/color.rs (3.65.2, 3.65.20)
}

// ═══════════════════════════════════════════════════════════════════════════
// Out_Of_Service for Commandable Value Objects (135.1-2025 7.3.1.1.2)
// ═══════════════════════════════════════════════════════════════════════════

fn register_oos_commandable_tests(registry: &mut TestRegistry) {
    macro_rules! oosc_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("OOS-CMD: ", $abbr, " OOS for Commandable Objects"),
                reference: "135.1-2025 - 7.3.1.1.2",
                section: Section::Objects,
                tags: &["parameterized", "oos", "commandable"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_oos_commandable(ctx, $ot)),
            });
        };
    }

    use bacnet_types::enums::ObjectType;
    oosc_test!(registry, "P8.1", "AV", ObjectType::ANALOG_VALUE);
    oosc_test!(registry, "P8.2", "BV", ObjectType::BINARY_VALUE);
    oosc_test!(registry, "P8.3", "MSV", ObjectType::MULTI_STATE_VALUE);
    oosc_test!(registry, "P8.4", "IV", ObjectType::INTEGER_VALUE);
    oosc_test!(registry, "P8.5", "PIV", ObjectType::POSITIVE_INTEGER_VALUE);
    oosc_test!(registry, "P8.6", "LAV", ObjectType::LARGE_ANALOG_VALUE);
    oosc_test!(registry, "P8.7", "CSV", ObjectType::CHARACTERSTRING_VALUE);
    oosc_test!(registry, "P8.8", "OSV", ObjectType::OCTETSTRING_VALUE);
    oosc_test!(registry, "P8.9", "BSV", ObjectType::BITSTRING_VALUE);
    oosc_test!(registry, "P8.10", "DV", ObjectType::DATE_VALUE);
    oosc_test!(registry, "P8.11", "TV", ObjectType::TIME_VALUE);
    oosc_test!(registry, "P8.12", "DTV", ObjectType::DATETIME_VALUE);
}

// ═══════════════════════════════════════════════════════════════════════════
// Value Source Mechanism (BTL 7.3.1.28.x)
// 5 tests × ~29 object types = ~145 test instances
// ═══════════════════════════════════════════════════════════════════════════

fn register_value_source_tests(registry: &mut TestRegistry) {
    macro_rules! vs_test {
        ($registry:expr, $id:expr, $abbr:expr, $ot:expr) => {
            $registry.add(TestDef {
                id: $id,
                name: concat!("VS: ", $abbr, " Value_Source Mechanism"),
                reference: "BTL - 7.3.1.28.1",
                section: Section::Objects,
                tags: &["parameterized", "value-source"],
                conditionality: Conditionality::RequiresCapability(Capability::ObjectType(
                    $ot.to_raw(),
                )),
                timeout: None,
                run: |ctx| Box::pin(helpers::test_value_source_write_by_other(ctx, $ot)),
            });
        };
    }

    use bacnet_types::enums::ObjectType;
    vs_test!(registry, "P9.1", "AO", ObjectType::ANALOG_OUTPUT);
    vs_test!(registry, "P9.2", "AV", ObjectType::ANALOG_VALUE);
    vs_test!(registry, "P9.3", "BO", ObjectType::BINARY_OUTPUT);
    vs_test!(registry, "P9.4", "BV", ObjectType::BINARY_VALUE);
    vs_test!(registry, "P9.5", "MSO", ObjectType::MULTI_STATE_OUTPUT);
    vs_test!(registry, "P9.6", "MSV", ObjectType::MULTI_STATE_VALUE);
    vs_test!(registry, "P9.7", "IV", ObjectType::INTEGER_VALUE);
    vs_test!(registry, "P9.8", "PIV", ObjectType::POSITIVE_INTEGER_VALUE);
    vs_test!(registry, "P9.9", "LAV", ObjectType::LARGE_ANALOG_VALUE);
    vs_test!(registry, "P9.10", "CSV", ObjectType::CHARACTERSTRING_VALUE);
    vs_test!(registry, "P9.11", "OSV", ObjectType::OCTETSTRING_VALUE);
    vs_test!(registry, "P9.12", "BSV", ObjectType::BITSTRING_VALUE);
    vs_test!(registry, "P9.13", "DV", ObjectType::DATE_VALUE);
    vs_test!(registry, "P9.14", "TV", ObjectType::TIME_VALUE);
    vs_test!(registry, "P9.15", "DTV", ObjectType::DATETIME_VALUE);
    vs_test!(registry, "P9.16", "LO", ObjectType::LIGHTING_OUTPUT);
    vs_test!(registry, "P9.17", "BLO", ObjectType::BINARY_LIGHTING_OUTPUT);
    vs_test!(registry, "P9.18", "CLR", ObjectType::COLOR);
    vs_test!(registry, "P9.19", "CT", ObjectType::COLOR_TEMPERATURE);
}
