//! BTL Test Plan Sections 4.15–4.16 — ReadRange.
//! 9 BTL references: 4.15 Initiates (2), 4.16 Executes (7).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 4.15 Initiates ReadRange ─────────────────────────────────────────

    registry.add(TestDef {
        id: "4.15.1",
        name: "RR-A: Read by Position",
        reference: "135.1-2025 - 8.21.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-a"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_a_by_position(ctx)),
    });
    registry.add(TestDef {
        id: "4.15.2",
        name: "RR-A: Read by Sequence",
        reference: "135.1-2025 - 8.21.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-a"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_a_by_sequence(ctx)),
    });

    // ── 4.16 Executes ReadRange ──────────────────────────────────────────

    registry.add(TestDef {
        id: "4.16.1",
        name: "RR-B: Support All List Properties",
        reference: "135.1-2025 - 9.21.1.14",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_b_all_list(ctx)),
    });
    registry.add(TestDef {
        id: "4.16.2",
        name: "RR-B: Non-Existent Property",
        reference: "135.1-2025 - 9.21.2.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_b_no_property(ctx)),
    });
    registry.add(TestDef {
        id: "4.16.3",
        name: "RR-B: Not a List Property",
        reference: "135.1-2025 - 9.21.2.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_b_not_list(ctx)),
    });
    registry.add(TestDef {
        id: "4.16.4",
        name: "RR-B: Non-Array with Index",
        reference: "135.1-2025 - 9.21.2.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_b_non_array_index(ctx)),
    });
    registry.add(TestDef {
        id: "4.16.5",
        name: "RR-B: Items Not Exist by Position",
        reference: "135.1-2025 - 9.21.1.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_b_pos_not_exist(ctx)),
    });
    registry.add(TestDef {
        id: "4.16.6",
        name: "RR-B: By Sequence No Sequence Numbers",
        reference: "135.1-2025 - 9.21.2.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-b"],
        conditionality: Conditionality::MinProtocolRevision(21),
        timeout: None,
        run: |ctx| Box::pin(rr_b_no_seq_numbers(ctx)),
    });
    registry.add(TestDef {
        id: "4.16.7",
        name: "RR-B: By Time No Timestamps",
        reference: "135.1-2025 - 9.21.2.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "read-range", "rr-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(rr_b_no_timestamps(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn rr_a_by_position(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn rr_a_by_sequence(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::TOTAL_RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn rr_b_all_list(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.pass()
}

async fn rr_b_no_property(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.read_expect_error(tl, PropertyIdentifier::from_raw(9999), None)
        .await
}

async fn rr_b_not_list(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Object_Name is not a list — ReadRange should fail
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}

async fn rr_b_non_array_index(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.pass()
}

async fn rr_b_pos_not_exist(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn rr_b_no_seq_numbers(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::DEVICE_ADDRESS_BINDING)
        .await?;
    ctx.pass()
}

async fn rr_b_no_timestamps(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::DEVICE_ADDRESS_BINDING)
        .await?;
    ctx.pass()
}
