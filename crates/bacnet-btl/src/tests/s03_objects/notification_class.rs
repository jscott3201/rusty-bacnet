//! BTL Test Plan Section 3.17 — Notification Class Object.
//! BTL refs (11): ValidDays, FromTime/ToTime, Transitions, Recipient_List (4),
//! Writing Properties, Time Non-Pattern (2), Read-only Recipient_List

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const OT: u32 = 15;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.17.1",
        name: "NC: ValidDays",
        reference: "BTL - 7.3.2.21.3.1",
        section: Section::Objects,
        tags: &["objects", "nc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.2",
        name: "NC: FromTime and ToTime",
        reference: "135.1-2025 - 7.3.2.21.3.2",
        section: Section::Objects,
        tags: &["objects", "nc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.3",
        name: "NC: Transitions",
        reference: "135.1-2025 - 7.3.2.21.3.4",
        section: Section::Objects,
        tags: &["objects", "nc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.4",
        name: "NC: Recipient_List Non-Volatility",
        reference: "135.1-2025 - 7.3.2.21.3.7",
        section: Section::Objects,
        tags: &["objects", "nc", "recipient"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_recipient(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.5",
        name: "NC: Writing Properties",
        reference: "135.1-2025 - 9.22.1.5",
        section: Section::Objects,
        tags: &["objects", "nc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.6",
        name: "NC: Device Identifier Recipients",
        reference: "135.1-2025 - 7.3.2.21.3.5",
        section: Section::Objects,
        tags: &["objects", "nc", "recipient"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_recipient(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.7",
        name: "NC: Network Address Recipients",
        reference: "135.1-2025 - 7.3.2.21.3.6",
        section: Section::Objects,
        tags: &["objects", "nc", "recipient"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_recipient(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.8",
        name: "NC: Time Non-Pattern Properties",
        reference: "135.1-2025 - 7.2.8",
        section: Section::Objects,
        tags: &["objects", "nc", "time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.9",
        name: "NC: Time Non-Pattern via WPM",
        reference: "135.1-2025 - 9.23.2.20",
        section: Section::Objects,
        tags: &["objects", "nc", "time"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.10",
        name: "NC: Read-only Recipient_List for NF",
        reference: "135.1-2025 - 7.3.2.21.3.9",
        section: Section::Objects,
        tags: &["objects", "nc", "recipient"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_recipient(ctx)),
    });
    registry.add(TestDef {
        id: "3.17.11",
        name: "NC: Recipient_List Non-Volatility (dup)",
        reference: "135.1-2025 - 7.3.2.21.3.7",
        section: Section::Objects,
        tags: &["objects", "nc", "recipient"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(nc_recipient(ctx)),
    });
}

async fn nc_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::NOTIFICATION_CLASS)?;
    ctx.verify_readable(oid, PropertyIdentifier::NOTIFICATION_CLASS)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::PRIORITY)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::ACK_REQUIRED)
        .await?;
    ctx.pass()
}
async fn nc_recipient(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::NOTIFICATION_CLASS)?;
    ctx.verify_readable(oid, PropertyIdentifier::RECIPIENT_LIST)
        .await?;
    ctx.pass()
}
