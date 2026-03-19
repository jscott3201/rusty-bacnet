//! BTL Test Plan Section 3.38 — Program Object.
//! BTL refs (2): Program_Change Property, REI
use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use crate::tests::helpers;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.38.1",
        name: "PROG: Program_Change Property",
        reference: "135.1-2025 - 7.3.2.22.1",
        section: Section::Objects,
        tags: &["objects", "program"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(16)),
        timeout: None,
        run: |ctx| Box::pin(prog_change(ctx)),
    });
    registry.add(TestDef {
        id: "3.38.2",
        name: "PROG: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "program", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(16)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::PROGRAM,
            ))
        },
    });
}
async fn prog_change(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::PROGRAM)?;
    ctx.verify_readable(oid, PropertyIdentifier::PROGRAM_STATE)
        .await?;
    ctx.pass()
}
