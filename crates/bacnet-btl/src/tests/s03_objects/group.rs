//! BTL Test Plan Section 3.12 — Group Object.
//! BTL refs: 1 (7.3.2.14 Group Object Test)

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.12.1",
        name: "GRP: Group Object Test",
        reference: "135.1-2025 - 7.3.2.14",
        section: Section::Objects,
        tags: &["objects", "group"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(11)),
        timeout: None,
        run: |ctx| Box::pin(grp_test(ctx)),
    });
}

async fn grp_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::GROUP)?;
    ctx.verify_readable(oid, PropertyIdentifier::LIST_OF_GROUP_MEMBERS)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}
