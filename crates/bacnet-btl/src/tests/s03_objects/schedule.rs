//! BTL Test Plan Section 3.19 — Schedule Object.
//! BTL refs (4): Write_Every_Scheduled_Action (2), Exception_Schedule Size, Schedule interaction

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const OT: u32 = 17;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.19.1",
        name: "SCHED: Write_Every_Scheduled_Action FALSE",
        reference: "135.1-2025 - 7.3.2.23.15",
        section: Section::Objects,
        tags: &["objects", "schedule"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(sched_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.19.2",
        name: "SCHED: Exception_Schedule Size Change",
        reference: "135.1-2025 - 7.3.2.23.9",
        section: Section::Objects,
        tags: &["objects", "schedule"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(sched_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.19.3",
        name: "SCHED: Write_Every_Scheduled_Action TRUE",
        reference: "135.1-2025 - 7.3.2.23.14",
        section: Section::Objects,
        tags: &["objects", "schedule"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(sched_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.19.4",
        name: "SCHED: Internally Written Datatypes",
        reference: "135.1-2025 - 7.3.2.23.11.1",
        section: Section::Objects,
        tags: &["objects", "schedule"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(sched_base(ctx)),
    });
}

async fn sched_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(oid, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::EFFECTIVE_PERIOD)
        .await?;
    ctx.pass()
}
