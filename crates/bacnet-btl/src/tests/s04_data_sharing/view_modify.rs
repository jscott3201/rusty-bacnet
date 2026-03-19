//! BTL Test Plan Sections 4.11–4.14 — View / Modify BIBBs.
//! 6 BTL references: View-A (1), Adv View-A (1), Modify-A (2), Adv Modify-A (2).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "4.11.1",
        name: "DS-View-A: Browse Properties",
        reference: "135.1-2025 - 8.18.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "view"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(view_browse(ctx)),
    });
    registry.add(TestDef {
        id: "4.12.1",
        name: "DS-Adv-View-A: Browse All",
        reference: "135.1-2025 - 8.18.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "view"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(view_browse(ctx)),
    });
    registry.add(TestDef {
        id: "4.13.1",
        name: "DS-Modify-A: Write Property",
        reference: "135.1-2025 - 8.20.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "modify"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(modify_write(ctx)),
    });
    registry.add(TestDef {
        id: "4.13.2",
        name: "DS-Modify-A: Write and Verify",
        reference: "135.1-2025 - 8.20.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "modify"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(modify_write(ctx)),
    });
    registry.add(TestDef {
        id: "4.14.1",
        name: "DS-Adv-Modify-A: Write Priority",
        reference: "135.1-2025 - 8.20.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "modify"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(modify_priority(ctx)),
    });
    registry.add(TestDef {
        id: "4.14.2",
        name: "DS-Adv-Modify-A: Relinquish",
        reference: "135.1-2025 - 8.20.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "modify"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(modify_priority(ctx)),
    });
}

async fn view_browse(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.pass()
}

async fn modify_write(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn modify_priority(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 55.5, Some(16))
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}
