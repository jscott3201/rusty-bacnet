//! BTL Test Plan Section 3.63 + 3.64 — Audit Reporter + Audit Log.
//! BTL refs: 3.63 (1 REI), 3.64 (1 REI)
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;
pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.63.1",
        name: "AR: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "audit", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::AUDIT_REPORTER,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.64.1",
        name: "AL: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "audit", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(62)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::AUDIT_LOG,
            ))
        },
    });
}
