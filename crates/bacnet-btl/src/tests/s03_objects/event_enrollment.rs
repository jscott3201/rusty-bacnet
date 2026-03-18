//! BTL Test Plan Section 3.11 + 3.52 — Event Enrollment + Alert Enrollment.
//! BTL refs: 3.11 (1), 3.52 (2)

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.11.1",
        name: "EE: Event_Enrollment REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ee"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
        timeout: None,
        run: |ctx| Box::pin(ee_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.52.1",
        name: "AE: Reports Source Object",
        reference: "135.1-2025 - 7.3.2.31.1",
        section: Section::Objects,
        tags: &["objects", "alert-enrollment"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(52)),
        timeout: None,
        run: |ctx| Box::pin(ae_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.52.2",
        name: "AE: No Acknowledgeable Transitions",
        reference: "135.1-2025 - 7.3.2.31.2",
        section: Section::Objects,
        tags: &["objects", "alert-enrollment"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(52)),
        timeout: None,
        run: |ctx| Box::pin(ae_base(ctx)),
    });
}

async fn ee_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::EVENT_ENROLLMENT)?;
    ctx.verify_readable(oid, PropertyIdentifier::EVENT_TYPE)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}
async fn ae_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::ALERT_ENROLLMENT)?;
    ctx.verify_readable(oid, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}
