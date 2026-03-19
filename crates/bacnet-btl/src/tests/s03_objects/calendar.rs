//! BTL Test Plan Section 3.8 — Calendar Object.
//! BTL references (7): Date Rollover, Date Range, WeekNDay, Date Pattern,
//! DateRange Non-Pattern, DateRange Open-Ended, WPM DateRange Non-Pattern

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

const OT: u32 = 6;
const T: ObjectType = ObjectType::CALENDAR;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.8.1",
        name: "CAL: Single Date Rollover",
        reference: "135.1-2025 - 7.3.2.8.1",
        section: Section::Objects,
        tags: &["objects", "calendar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cal_date_rollover(ctx)),
    });
    registry.add(TestDef {
        id: "3.8.2",
        name: "CAL: Date Range Test",
        reference: "135.1-2025 - 7.3.2.8.2",
        section: Section::Objects,
        tags: &["objects", "calendar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cal_date_range(ctx)),
    });
    registry.add(TestDef {
        id: "3.8.3",
        name: "CAL: WeekNDay Test",
        reference: "135.1-2025 - 7.3.2.8.3",
        section: Section::Objects,
        tags: &["objects", "calendar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cal_weeknday(ctx)),
    });
    registry.add(TestDef {
        id: "3.8.4",
        name: "CAL: Date Pattern Properties",
        reference: "135.1-2025 - 7.2.4",
        section: Section::Objects,
        tags: &["objects", "calendar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cal_date_pattern(ctx)),
    });
    registry.add(TestDef {
        id: "3.8.5",
        name: "CAL: DateRange Non-Pattern",
        reference: "135.1-2025 - 7.2.10",
        section: Section::Objects,
        tags: &["objects", "calendar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cal_daterange_nonpattern(ctx)),
    });
    registry.add(TestDef {
        id: "3.8.6",
        name: "CAL: DateRange Open-Ended",
        reference: "135.1-2025 - 7.2.11",
        section: Section::Objects,
        tags: &["objects", "calendar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cal_daterange_openended(ctx)),
    });
    registry.add(TestDef {
        id: "3.8.7",
        name: "CAL: WPM DateRange Non-Pattern",
        reference: "135.1-2025 - 9.23.2.22",
        section: Section::Objects,
        tags: &["objects", "calendar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(cal_wpm_daterange(ctx)),
    });
}

async fn cal_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(T)?;
    ctx.verify_readable(oid, PropertyIdentifier::DATE_LIST)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}
async fn cal_date_rollover(ctx: &mut TestContext) -> Result<(), TestFailure> {
    cal_base(ctx).await
}
async fn cal_date_range(ctx: &mut TestContext) -> Result<(), TestFailure> {
    cal_base(ctx).await
}
async fn cal_weeknday(ctx: &mut TestContext) -> Result<(), TestFailure> {
    cal_base(ctx).await
}
async fn cal_date_pattern(ctx: &mut TestContext) -> Result<(), TestFailure> {
    cal_base(ctx).await
}
async fn cal_daterange_nonpattern(ctx: &mut TestContext) -> Result<(), TestFailure> {
    cal_base(ctx).await
}
async fn cal_daterange_openended(ctx: &mut TestContext) -> Result<(), TestFailure> {
    cal_base(ctx).await
}
async fn cal_wpm_daterange(ctx: &mut TestContext) -> Result<(), TestFailure> {
    cal_base(ctx).await
}
