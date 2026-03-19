//! BTL Test Plan Section 3.1 — Analog Input Object.
//!
//! BTL references (2 total):
//!   1. BTL - 7.3.1.1.1 — Out_Of_Service, Status_Flags, and Reliability Test
//!   2. 135.1-2025 - 7.3.1.21.3 — Reliability_Evaluation_Inhibit Object Test

use bacnet_types::enums::ObjectType;

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;

const OT: u32 = 0; // ANALOG_INPUT

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.1.1",
        name: "AI: Out_Of_Service, Status_Flags, Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ai", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::ANALOG_INPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.1.2",
        name: "AI: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ai", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ANALOG_INPUT,
            ))
        },
    });
}
