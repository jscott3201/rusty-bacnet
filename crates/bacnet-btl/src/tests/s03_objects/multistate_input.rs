//! BTL Test Plan Section 3.14 — Multi-State Input Object.
//! BTL references (6): OOS, Number_Of_States range, State_Text, REI, Alarm_Values, X73.1

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

const OT: u32 = 13;
const T: ObjectType = ObjectType::MULTI_STATE_INPUT;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.14.1",
        name: "MSI: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "msi", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.14.2",
        name: "MSI: Number_Of_States Range",
        reference: "135.1-2025 - 7.3.1.15",
        section: Section::Objects,
        tags: &["objects", "msi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_number_of_states_range(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.14.3",
        name: "MSI: Number_Of_States and State_Text",
        reference: "135.1-2025 - 7.3.2.18.2",
        section: Section::Objects,
        tags: &["objects", "msi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_state_text_consistency(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.14.4",
        name: "MSI: Writable Number_Of_States",
        reference: "BTL - 7.3.1.X73.1",
        section: Section::Objects,
        tags: &["objects", "msi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_number_of_states_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.14.5",
        name: "MSI: Alarm_Values",
        reference: "135.1-2025 - 7.3.1.8",
        section: Section::Objects,
        tags: &["objects", "msi", "alarm"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_change_of_state(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.14.6",
        name: "MSI: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "msi", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
}
