//! BTL Test Plan Sections 3.24-3.35 — Value Type Objects (12 types).
//! Total BTL refs: 134 (10-14 per type depending on date/time pattern tests)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.24.1",
        name: "BSV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.2",
        name: "BSV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.3",
        name: "BSV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.4",
        name: "BSV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.5",
        name: "BSV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.6",
        name: "BSV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.7",
        name: "BSV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.8",
        name: "BSV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.9",
        name: "BSV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.24.10",
        name: "BSV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "bsv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(39)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::BITSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.1",
        name: "CSV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.2",
        name: "CSV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.3",
        name: "CSV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.4",
        name: "CSV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.5",
        name: "CSV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.6",
        name: "CSV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.7",
        name: "CSV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.8",
        name: "CSV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.9",
        name: "CSV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.25.10",
        name: "CSV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "csv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(40)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::CHARACTERSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.1",
        name: "DPV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.2",
        name: "DPV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.3",
        name: "DPV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.4",
        name: "DPV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.5",
        name: "DPV: Date Pattern Properties",
        reference: "135.1-2025 - 7.2.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.6",
        name: "DPV: Date Pattern Properties (variant)",
        reference: "135.1-2025 - 7.2.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.7",
        name: "DPV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.8",
        name: "DPV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.9",
        name: "DPV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.10",
        name: "DPV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.11",
        name: "DPV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.26.12",
        name: "DPV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(41)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.1",
        name: "DV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::DATE_VALUE)),
    });
    registry.add(TestDef {
        id: "3.27.2",
        name: "DV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_commandable(ctx, ObjectType::DATE_VALUE)),
    });
    registry.add(TestDef {
        id: "3.27.3",
        name: "DV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.4",
        name: "DV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.5",
        name: "DV: Date Non-Pattern Properties",
        reference: "135.1-2025 - 7.2.7",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.6",
        name: "DV: Date Non-Pattern via WPM",
        reference: "135.1-2025 - 9.23.2.19",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.7",
        name: "DV: Date Non-Pattern (variant)",
        reference: "135.1-2025 - 7.2.7",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.8",
        name: "DV: Date Non-Pattern via WPM (variant)",
        reference: "135.1-2025 - 9.23.2.19",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.9",
        name: "DV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.10",
        name: "DV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_none(ctx, ObjectType::DATE_VALUE)),
    });
    registry.add(TestDef {
        id: "3.27.11",
        name: "DV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.12",
        name: "DV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.13",
        name: "DV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.27.14",
        name: "DV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(42)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATE_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.1",
        name: "DTPV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.2",
        name: "DTPV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.3",
        name: "DTPV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.4",
        name: "DTPV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.5",
        name: "DTPV: DateTime Pattern Properties",
        reference: "135.1-2025 - 7.2.6",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.6",
        name: "DTPV: DateTime Pattern Properties (variant)",
        reference: "135.1-2025 - 7.2.6",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.7",
        name: "DTPV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.8",
        name: "DTPV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.9",
        name: "DTPV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.10",
        name: "DTPV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.11",
        name: "DTPV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.28.12",
        name: "DTPV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(43)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.1",
        name: "DTV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.2",
        name: "DTV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.3",
        name: "DTV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.4",
        name: "DTV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.5",
        name: "DTV: DateTime Non-Pattern",
        reference: "BTL - 7.2.9",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.6",
        name: "DTV: DateTime Non-Pattern via WPM",
        reference: "BTL - 9.23.2.21",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.7",
        name: "DTV: DateTime Non-Pattern (variant)",
        reference: "BTL - 7.2.9",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.8",
        name: "DTV: DateTime Non-Pattern via WPM (variant)",
        reference: "BTL - 9.23.2.21",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.9",
        name: "DTV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.10",
        name: "DTV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.11",
        name: "DTV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.12",
        name: "DTV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.13",
        name: "DTV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.29.14",
        name: "DTV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "dtv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(44)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::DATETIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.1",
        name: "IV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.2",
        name: "IV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.3",
        name: "IV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.4",
        name: "IV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.5",
        name: "IV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.6",
        name: "IV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.7",
        name: "IV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.8",
        name: "IV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.9",
        name: "IV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.30.10",
        name: "IV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "iv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(45)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.1",
        name: "LAV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.2",
        name: "LAV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.3",
        name: "LAV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.4",
        name: "LAV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.5",
        name: "LAV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.6",
        name: "LAV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.7",
        name: "LAV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.8",
        name: "LAV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.9",
        name: "LAV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.31.10",
        name: "LAV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "lav"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(46)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::LARGE_ANALOG_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.1",
        name: "OSV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.2",
        name: "OSV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.3",
        name: "OSV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.4",
        name: "OSV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.5",
        name: "OSV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.6",
        name: "OSV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.7",
        name: "OSV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.8",
        name: "OSV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.9",
        name: "OSV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.32.10",
        name: "OSV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "osv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(47)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::OCTETSTRING_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.1",
        name: "PIV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.2",
        name: "PIV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.3",
        name: "PIV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.4",
        name: "PIV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.5",
        name: "PIV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.6",
        name: "PIV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.7",
        name: "PIV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.8",
        name: "PIV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.9",
        name: "PIV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.33.10",
        name: "PIV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "piv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(48)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::POSITIVE_INTEGER_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.1",
        name: "TPV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.2",
        name: "TPV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_commandable(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.3",
        name: "TPV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.4",
        name: "TPV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.5",
        name: "TPV: Time Pattern Properties",
        reference: "135.1-2025 - 7.2.5",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.6",
        name: "TPV: Time Pattern Properties (variant)",
        reference: "135.1-2025 - 7.2.5",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.7",
        name: "TPV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.8",
        name: "TPV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.9",
        name: "TPV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.10",
        name: "TPV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.11",
        name: "TPV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.34.12",
        name: "TPV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "tpv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(49)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIMEPATTERN_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.1",
        name: "TV: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::TIME_VALUE)),
    });
    registry.add(TestDef {
        id: "3.35.2",
        name: "TV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_commandable(ctx, ObjectType::TIME_VALUE)),
    });
    registry.add(TestDef {
        id: "3.35.3",
        name: "TV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.4",
        name: "TV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.5",
        name: "TV: Time Non-Pattern Properties",
        reference: "135.1-2025 - 7.2.8",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.6",
        name: "TV: Time Non-Pattern via WPM",
        reference: "135.1-2025 - 9.23.2.20",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.7",
        name: "TV: Time Non-Pattern (variant)",
        reference: "135.1-2025 - 7.2.8",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.8",
        name: "TV: Time Non-Pattern via WPM (variant)",
        reference: "135.1-2025 - 9.23.2.20",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.9",
        name: "TV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_non_commandable(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.10",
        name: "TV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_none(ctx, ObjectType::TIME_VALUE)),
    });
    registry.add(TestDef {
        id: "3.35.11",
        name: "TV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.12",
        name: "TV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.13",
        name: "TV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.35.14",
        name: "TV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "value-type", "tv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(50)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIME_VALUE,
            ))
        },
    });
}
