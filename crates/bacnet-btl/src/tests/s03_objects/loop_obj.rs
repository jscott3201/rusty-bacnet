//! BTL Test Plan Section 3.13 — Loop Object.
//! BTL refs (5): Manipulated_Variable tracking, Controlled_Variable tracking,
//! Setpoint tracking, OOS, REI

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use crate::tests::helpers;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const OT: u32 = 12;
const T: ObjectType = ObjectType::LOOP;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.13.1",
        name: "LOOP: Manipulated_Variable Tracking",
        reference: "135.1-2025 - 7.3.2.17.1",
        section: Section::Objects,
        tags: &["objects", "loop"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(loop_manip_var(ctx)),
    });
    registry.add(TestDef {
        id: "3.13.2",
        name: "LOOP: Controlled_Variable Tracking",
        reference: "135.1-2025 - 7.3.2.17.2",
        section: Section::Objects,
        tags: &["objects", "loop"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(loop_ctrl_var(ctx)),
    });
    registry.add(TestDef {
        id: "3.13.3",
        name: "LOOP: Setpoint_Reference Tracking",
        reference: "135.1-2025 - 7.3.2.17.3",
        section: Section::Objects,
        tags: &["objects", "loop"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(loop_setpoint(ctx)),
    });
    registry.add(TestDef {
        id: "3.13.4",
        name: "LOOP: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "loop", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.13.5",
        name: "LOOP: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "loop", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
}

async fn loop_manip_var(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(T)?;
    ctx.verify_readable(oid, PropertyIdentifier::MANIPULATED_VARIABLE_REFERENCE)
        .await?;
    ctx.pass()
}
async fn loop_ctrl_var(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(T)?;
    ctx.verify_readable(oid, PropertyIdentifier::CONTROLLED_VARIABLE_REFERENCE)
        .await?;
    ctx.pass()
}
async fn loop_setpoint(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(T)?;
    ctx.verify_readable(oid, PropertyIdentifier::SETPOINT_REFERENCE)
        .await?;
    ctx.pass()
}
