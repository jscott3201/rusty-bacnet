//! BTL Test Plan Section 3.6 — Binary Output Object.
//!
//! BTL references (29 total):
//!   1-3. Relinquish Default, Command Prioritization, OOS/SF/Reliability
//!   4. Polarity  5-6. Change of State, State Count  7-8. Elapsed Active Time
//!   9-22. Minimum Off/On Time (14 tests)
//!   23-26. Value Source (4 tests)  27. REI
//!   (2 duplicates: 7.3.1.6.1 appears twice, 7.3.1.6.10 appears twice)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

const OT: u32 = 4;
const T: ObjectType = ObjectType::BINARY_OUTPUT;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.6.1",
        name: "BO: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "bo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_relinquish_default(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.2",
        name: "BO: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "bo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_command_prioritization(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.3",
        name: "BO: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "bo", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.4",
        name: "BO: Polarity Property",
        reference: "135.1-2025 - 7.3.2.6.3",
        section: Section::Objects,
        tags: &["objects", "bo", "polarity"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_polarity(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.5",
        name: "BO: Change of State",
        reference: "135.1-2025 - 7.3.1.8",
        section: Section::Objects,
        tags: &["objects", "bo", "cos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_change_of_state(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.6",
        name: "BO: Non-zero Writable State Count",
        reference: "135.1-2025 - 7.3.1.24",
        section: Section::Objects,
        tags: &["objects", "bo", "state-count"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_state_count_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.7",
        name: "BO: Elapsed Active Time",
        reference: "135.1-2025 - 7.3.1.9",
        section: Section::Objects,
        tags: &["objects", "bo", "elapsed"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_elapsed_active_time(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.8",
        name: "BO: Writable Elapsed Active Time",
        reference: "135.1-2025 - 7.3.1.25",
        section: Section::Objects,
        tags: &["objects", "bo", "elapsed"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_elapsed_active_time_writable(ctx, T)),
    });
    // Minimum Off Time tests (7.3.1.4, 7.3.1.6.x)
    registry.add(TestDef {
        id: "3.6.9",
        name: "BO: Minimum_Off_Time",
        reference: "135.1-2025 - 7.3.1.4",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_off_time(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.10",
        name: "BO: Min Off Override",
        reference: "135.1-2025 - 7.3.1.6.1",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.11",
        name: "BO: Min Off Priority > 6",
        reference: "135.1-2025 - 7.3.1.6.2",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.12",
        name: "BO: Min Off Priority < 6",
        reference: "135.1-2025 - 7.3.1.6.4",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.13",
        name: "BO: Min Off Clock Unaffected",
        reference: "135.1-2025 - 7.3.1.6.6",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.14",
        name: "BO: Min Off Starts at INACTIVE",
        reference: "135.1-2025 - 7.3.1.6.8",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.15",
        name: "BO: Min Times Not Affected By Time Changes",
        reference: "135.1-2025 - 7.3.1.6.10",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.16",
        name: "BO: Min Off Value Source",
        reference: "135.1-2025 - 7.3.1.6.11",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    // Minimum On Time tests (7.3.1.5, 7.3.1.6.x)
    registry.add(TestDef {
        id: "3.6.17",
        name: "BO: Minimum_On_Time",
        reference: "135.1-2025 - 7.3.1.5",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_on_time(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.18",
        name: "BO: Min On Override",
        reference: "135.1-2025 - 7.3.1.6.1",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.19",
        name: "BO: Min On Priority > 6",
        reference: "135.1-2025 - 7.3.1.6.3",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.20",
        name: "BO: Min On Priority < 6",
        reference: "135.1-2025 - 7.3.1.6.5",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.21",
        name: "BO: Min On Clock Unaffected",
        reference: "135.1-2025 - 7.3.1.6.7",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.22",
        name: "BO: Min On Starts at ACTIVE",
        reference: "135.1-2025 - 7.3.1.6.9",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.23",
        name: "BO: Min On Times Not Affected By Time Changes",
        reference: "135.1-2025 - 7.3.1.6.10",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.24",
        name: "BO: Min On Value Source",
        reference: "135.1-2025 - 7.3.1.6.12",
        section: Section::Objects,
        tags: &["objects", "bo", "min-time", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_minimum_time_behavior(ctx, T)),
    });
    // Value Source
    registry.add(TestDef {
        id: "3.6.25",
        name: "BO: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "bo", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_none(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.26",
        name: "BO: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "bo", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.27",
        name: "BO: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "bo", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_write_by_other(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.6.28",
        name: "BO: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "bo", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_local(ctx, T)),
    });
    // REI
    registry.add(TestDef {
        id: "3.6.29",
        name: "BO: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "bo", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
}
