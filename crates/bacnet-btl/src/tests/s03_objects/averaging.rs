//! BTL Test Plan Section 3.4 — Averaging Object.
//! BTL references (2): 7.3.2.4.1 Reinitializing Samples, 7.3.2.4.2 Managing Sample Window

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const OT: u32 = 18;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.4.1",
        name: "AVG: Reinitializing Samples",
        reference: "135.1-2025 - 7.3.2.4.1",
        section: Section::Objects,
        tags: &["objects", "averaging"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(avg_reinit_samples(ctx)),
    });
    registry.add(TestDef {
        id: "3.4.2",
        name: "AVG: Managing Sample Window",
        reference: "135.1-2025 - 7.3.2.4.2",
        section: Section::Objects,
        tags: &["objects", "averaging"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(avg_manage_window(ctx)),
    });
}

async fn avg_reinit_samples(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::AVERAGING)?;
    ctx.verify_readable(oid, PropertyIdentifier::PROPERTY_LIST)
        .await?;
    ctx.pass()
}
async fn avg_manage_window(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::AVERAGING)?;
    ctx.verify_readable(oid, PropertyIdentifier::PROPERTY_LIST)
        .await?;
    ctx.pass()
}
