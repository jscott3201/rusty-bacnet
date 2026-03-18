//! BTL Test Plan Section 3.15 — Multi-State Output Object.
//! BTL references (12): Relinquish, Cmd Pri, OOS, Number_Of_States, State_Text,
//! Value Source (5), REI, X73.1

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

const OT: u32 = 14;
const T: ObjectType = ObjectType::MULTI_STATE_OUTPUT;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.15.1",
        name: "MSO: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "mso"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_relinquish_default(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.2",
        name: "MSO: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "mso"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_command_prioritization(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.3",
        name: "MSO: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "mso", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.4",
        name: "MSO: Number_Of_States Range",
        reference: "135.1-2025 - 7.3.1.15",
        section: Section::Objects,
        tags: &["objects", "mso"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_number_of_states_range(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.5",
        name: "MSO: Number_Of_States and State_Text",
        reference: "135.1-2025 - 7.3.2.19.2",
        section: Section::Objects,
        tags: &["objects", "mso"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_state_text_consistency(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.6",
        name: "MSO: Resizable State_Text",
        reference: "135.1-2025 - 7.3.1.38.1",
        section: Section::Objects,
        tags: &["objects", "mso", "state-text"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_state_text_consistency(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.7",
        name: "MSO: Writable Number_Of_States",
        reference: "BTL - 7.3.1.X73.1",
        section: Section::Objects,
        tags: &["objects", "mso"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_number_of_states_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.8",
        name: "MSO: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "mso", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_none(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.9",
        name: "MSO: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "mso", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.10",
        name: "MSO: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "mso", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_write_by_other(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.11",
        name: "MSO: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "mso", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_local(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.15.12",
        name: "MSO: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "mso", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
}
