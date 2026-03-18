//! BTL Test Plan Section 3.16 — Multi-State Value Object.
//! BTL references (14): OOS, OOS-Cmd, Relinquish, Cmd Pri, Number_Of_States,
//! State_Text, Value Source (5), REI, X73.1, Alarm_Values

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

const OT: u32 = 19;
const T: ObjectType = ObjectType::MULTI_STATE_VALUE;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.16.1",
        name: "MSV: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "msv", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.2",
        name: "MSV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "msv", "oos-cmd"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.3",
        name: "MSV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "msv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_relinquish_default(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.4",
        name: "MSV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "msv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_command_prioritization(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.5",
        name: "MSV: Number_Of_States Range",
        reference: "135.1-2025 - 7.3.1.15",
        section: Section::Objects,
        tags: &["objects", "msv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_number_of_states_range(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.6",
        name: "MSV: Number_Of_States and State_Text",
        reference: "135.1-2025 - 7.3.2.20.2",
        section: Section::Objects,
        tags: &["objects", "msv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_state_text_consistency(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.7",
        name: "MSV: Writable Number_Of_States",
        reference: "BTL - 7.3.1.X73.1",
        section: Section::Objects,
        tags: &["objects", "msv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_number_of_states_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.8",
        name: "MSV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "msv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_non_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.9",
        name: "MSV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "msv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_none(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.10",
        name: "MSV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "msv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.11",
        name: "MSV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "msv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_write_by_other(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.12",
        name: "MSV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "msv", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_local(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.13",
        name: "MSV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "msv", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.16.14",
        name: "MSV: Alarm_Values/Change of State",
        reference: "135.1-2025 - 7.3.1.8",
        section: Section::Objects,
        tags: &["objects", "msv", "alarm"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_change_of_state(ctx, T)),
    });
}
