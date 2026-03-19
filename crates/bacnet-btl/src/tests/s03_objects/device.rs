//! BTL Test Plan Section 3.10 — Device Object.
//! BTL references (13): Object_Name/OID config, Database_Revision (4 variants),
//! TimeSynchronization Recipients, Date/Time Non-Pattern (4), UTC_Offset config

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.10.1",
        name: "DEV: Object_Name Configurable",
        reference: "135.1-2025 - 7.3.2.10.9",
        section: Section::Objects,
        tags: &["objects", "device"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_object_name_config(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.2",
        name: "DEV: Object_Identifier Configurable",
        reference: "135.1-2025 - 7.3.2.10.10",
        section: Section::Objects,
        tags: &["objects", "device"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_oid_config(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.3",
        name: "DEV: DB_Revision Increments on Create",
        reference: "135.1-2025 - 7.3.2.10.3",
        section: Section::Objects,
        tags: &["objects", "device", "db-rev"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_dbrev(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.4",
        name: "DEV: DB_Revision Increments on Delete",
        reference: "135.1-2025 - 7.3.2.10.4",
        section: Section::Objects,
        tags: &["objects", "device", "db-rev"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_dbrev(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.5",
        name: "DEV: DB_Revision Increments on Property Change",
        reference: "135.1-2025 - 7.3.2.10.5",
        section: Section::Objects,
        tags: &["objects", "device", "db-rev"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_dbrev(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.6",
        name: "DEV: DB_Revision Increments on Config Change",
        reference: "135.1-2025 - 7.3.2.10.6",
        section: Section::Objects,
        tags: &["objects", "device", "db-rev"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_dbrev(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.7",
        name: "DEV: TimeSynchronization Recipients",
        reference: "135.1-2025 - 13.2.1",
        section: Section::Objects,
        tags: &["objects", "device", "time-sync"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_base(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.8",
        name: "DEV: Date Non-Pattern Properties",
        reference: "135.1-2025 - 7.2.7",
        section: Section::Objects,
        tags: &["objects", "device", "date"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_date(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.9",
        name: "DEV: Date Non-Pattern via WPM",
        reference: "135.1-2025 - 9.23.2.19",
        section: Section::Objects,
        tags: &["objects", "device", "date"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_date(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.10",
        name: "DEV: Time Non-Pattern Properties",
        reference: "135.1-2025 - 7.2.8",
        section: Section::Objects,
        tags: &["objects", "device", "time"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_time(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.11",
        name: "DEV: Time Non-Pattern via WPM",
        reference: "135.1-2025 - 9.23.2.20",
        section: Section::Objects,
        tags: &["objects", "device", "time"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_time(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.12",
        name: "DEV: UTC_Offset Configurable",
        reference: "135.1-2025 - 7.3.2.10.8",
        section: Section::Objects,
        tags: &["objects", "device", "utc"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_utc_offset(ctx)),
    });
    registry.add(TestDef {
        id: "3.10.13",
        name: "DEV: Align_Intervals Configurable",
        reference: "135.1-2025 - 7.3.2.10.11",
        section: Section::Objects,
        tags: &["objects", "device"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(dev_base(ctx)),
    });
}

async fn dev_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_IDENTIFIER)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_VERSION)
        .await?;
    ctx.pass()
}
async fn dev_object_name_config(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}
async fn dev_oid_config(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_IDENTIFIER)
        .await?;
    ctx.pass()
}
async fn dev_dbrev(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::DATABASE_REVISION)
        .await?;
    ctx.pass()
}
async fn dev_date(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_DATE)
        .await?;
    ctx.pass()
}
async fn dev_time(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_TIME)
        .await?;
    ctx.pass()
}
async fn dev_utc_offset(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::UTC_OFFSET)
        .await?;
    ctx.pass()
}
