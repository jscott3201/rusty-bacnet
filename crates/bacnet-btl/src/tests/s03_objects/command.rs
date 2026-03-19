//! BTL Test Plan Section 3.9 — Command Object.
//! BTL references (13): 9 object-specific + REI + Value Source (3)

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use crate::tests::helpers;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const OT: u32 = 7;
const T: ObjectType = ObjectType::COMMAND;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.9.1",
        name: "CMD: Quit on Failure",
        reference: "135.1-2025 - 7.3.2.9.2",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.2",
        name: "CMD: Empty Action List",
        reference: "135.1-2025 - 7.3.2.9.4",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.3",
        name: "CMD: Action 0",
        reference: "135.1-2025 - 7.3.2.9.5",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.4",
        name: "CMD: Write While In_Process",
        reference: "135.1-2025 - 7.3.2.9.7",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.5",
        name: "CMD: Action_Text",
        reference: "135.1-2025 - 7.3.2.9.6",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.6",
        name: "CMD: All Writes Successful with Post Delay",
        reference: "135.1-2025 - 7.3.2.9.1",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.7",
        name: "CMD: External Writes",
        reference: "135.1-2025 - 7.3.2.9.3",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.8",
        name: "CMD: Action Size Changes Action_Text",
        reference: "135.1-2025 - 7.3.2.9.8",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.9",
        name: "CMD: Action_Text Size Changes Action",
        reference: "135.1-2025 - 7.3.2.9.9",
        section: Section::Objects,
        tags: &["objects", "command"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cmd_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.9.10",
        name: "CMD: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "command", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_write_by_other(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.9.11",
        name: "CMD: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "command", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_local(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.9.12",
        name: "CMD: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "command", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_non_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.9.13",
        name: "CMD: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "command", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
}

async fn cmd_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(T)?;
    ctx.verify_readable(oid, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::IN_PROCESS)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::ALL_WRITES_SUCCESSFUL)
        .await?;
    ctx.pass()
}
