//! BTL Test Plan Section 3.20 + 3.23 — TrendLog + TrendLogMultiple.
//! BTL refs: 3.20 (1 REI), 3.23 (1 REI)
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;
pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.20.1",
        name: "TL: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "tl", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TREND_LOG,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.23.1",
        name: "TLM: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "tlm", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TREND_LOG_MULTIPLE,
            ))
        },
    });
}
