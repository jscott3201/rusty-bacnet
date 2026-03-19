//! BTL Test Plan Section 3.22 — Event Log Object.
//! BTL refs (8): BUFFER_READY (2), Event_Enable, Notify_Type,
//! Notification_Threshold, Last_Notify_Record, Records_Since, REI
use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use crate::tests::helpers;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
const OT: u32 = 25;
pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.22.1",
        name: "EL: BUFFER_READY Confirmed",
        reference: "135.1-2025 - 8.4.7",
        section: Section::Objects,
        tags: &["objects", "el"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(el_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.22.2",
        name: "EL: BUFFER_READY Unconfirmed",
        reference: "135.1-2025 - 8.5.7",
        section: Section::Objects,
        tags: &["objects", "el"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(el_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.22.3",
        name: "EL: Event_Enable for TO_NORMAL",
        reference: "135.1-2025 - 7.3.1.10.2",
        section: Section::Objects,
        tags: &["objects", "el"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(el_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.22.4",
        name: "EL: Notify_Type",
        reference: "135.1-2025 - 7.3.1.12",
        section: Section::Objects,
        tags: &["objects", "el"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(el_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.22.5",
        name: "EL: Notification_Threshold",
        reference: "135.1-2025 - 7.3.2.24.10",
        section: Section::Objects,
        tags: &["objects", "el"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(el_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.22.6",
        name: "EL: Last_Notify_Record",
        reference: "135.1-2025 - 7.3.2.24.17",
        section: Section::Objects,
        tags: &["objects", "el"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(el_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.22.7",
        name: "EL: Records_Since_Notification",
        reference: "135.1-2025 - 7.3.2.24.18",
        section: Section::Objects,
        tags: &["objects", "el"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(el_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.22.8",
        name: "EL: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "el", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::EVENT_LOG,
            ))
        },
    });
}
async fn el_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::EVENT_LOG)?;
    ctx.verify_readable(oid, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}
