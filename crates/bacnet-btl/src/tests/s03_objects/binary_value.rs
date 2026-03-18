//! BTL Test Plan Section 3.7 — Binary Value Object.
//!
//! BTL references (30 total): Same structure as BO but with OOS-Commandable (7.3.1.1.2)
//! and Non-commandable Value_Source (7.3.1.28.2).

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

const OT: u32 = 5;
const T: ObjectType = ObjectType::BINARY_VALUE;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.7.1",
        name: "BV: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "bv", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.2",
        name: "BV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "bv", "oos-cmd"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.3",
        name: "BV: Change of State",
        reference: "135.1-2025 - 7.3.1.8",
        section: Section::Objects,
        tags: &["objects", "bv", "cos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_change_of_state(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.4",
        name: "BV: Non-zero Writable State Count",
        reference: "135.1-2025 - 7.3.1.24",
        section: Section::Objects,
        tags: &["objects", "bv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_state_count_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.5",
        name: "BV: Elapsed Active Time",
        reference: "135.1-2025 - 7.3.1.9",
        section: Section::Objects,
        tags: &["objects", "bv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_elapsed_active_time(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.6",
        name: "BV: Writable Elapsed Active Time",
        reference: "135.1-2025 - 7.3.1.25",
        section: Section::Objects,
        tags: &["objects", "bv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_elapsed_active_time_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.7",
        name: "BV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "bv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_relinquish_default(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.8",
        name: "BV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "bv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_command_prioritization(ctx, T)),
    });
    // Minimum Off Time
    registry.add(TestDef {
        id: "3.7.9",
        name: "BV: Minimum_Off_Time",
        reference: "135.1-2025 - 7.3.1.4",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_off_time(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.10",
        name: "BV: Min Off Override",
        reference: "135.1-2025 - 7.3.1.6.1",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.11",
        name: "BV: Min Off Priority > 6",
        reference: "135.1-2025 - 7.3.1.6.2",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.12",
        name: "BV: Min Off Priority < 6",
        reference: "135.1-2025 - 7.3.1.6.4",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.13",
        name: "BV: Min Off Clock Unaffected",
        reference: "135.1-2025 - 7.3.1.6.6",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.14",
        name: "BV: Min Off Starts at INACTIVE",
        reference: "135.1-2025 - 7.3.1.6.8",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.15",
        name: "BV: Min Times Not Affected By Time Changes",
        reference: "135.1-2025 - 7.3.1.6.10",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.16",
        name: "BV: Min Off Value Source",
        reference: "135.1-2025 - 7.3.1.6.11",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    // Minimum On Time
    registry.add(TestDef {
        id: "3.7.17",
        name: "BV: Minimum_On_Time",
        reference: "135.1-2025 - 7.3.1.5",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_on_time(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.18",
        name: "BV: Min On Override",
        reference: "135.1-2025 - 7.3.1.6.1",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.19",
        name: "BV: Min On Priority > 6",
        reference: "135.1-2025 - 7.3.1.6.3",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.20",
        name: "BV: Min On Priority < 6",
        reference: "135.1-2025 - 7.3.1.6.5",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.21",
        name: "BV: Min On Clock Unaffected",
        reference: "135.1-2025 - 7.3.1.6.7",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.22",
        name: "BV: Min On Starts at ACTIVE",
        reference: "135.1-2025 - 7.3.1.6.9",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.23",
        name: "BV: Min On Times Not Affected By Time Changes",
        reference: "135.1-2025 - 7.3.1.6.10",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.24",
        name: "BV: Min On Value Source",
        reference: "135.1-2025 - 7.3.1.6.12",
        section: Section::Objects,
        tags: &["objects", "bv", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    // Value Source
    registry.add(TestDef {
        id: "3.7.25",
        name: "BV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "bv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_non_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.26",
        name: "BV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "bv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_none(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.27",
        name: "BV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "bv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.28",
        name: "BV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "bv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_write_by_other(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.7.29",
        name: "BV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "bv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_local(ctx, T)),
    });
    // REI
    registry.add(TestDef {
        id: "3.7.30",
        name: "BV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "bv", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
}
