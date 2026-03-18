//! BTL Test Plan Section 3.21 — Structured View Object.
//! BTL refs (2): Subordinate_List/Annotations resize tests
use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.21.1",
        name: "SV: Subordinate_List Resizes Annotations",
        reference: "135.1-2025 - 7.3.2.29.1",
        section: Section::Objects,
        tags: &["objects", "sv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(29)),
        timeout: None,
        run: |ctx| Box::pin(sv_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.21.2",
        name: "SV: Annotations Resizes Subordinate_List",
        reference: "135.1-2025 - 7.3.2.29.2",
        section: Section::Objects,
        tags: &["objects", "sv"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(29)),
        timeout: None,
        run: |ctx| Box::pin(sv_base(ctx)),
    });
}
async fn sv_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::STRUCTURED_VIEW)?;
    ctx.verify_readable(oid, PropertyIdentifier::PROPERTY_LIST)
        .await?;
    ctx.pass()
}
