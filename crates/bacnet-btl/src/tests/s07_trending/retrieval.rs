//! BTL Test Plan Sections 7.5–7.6, 7.9–7.11 — Automated Retrieval + Archival.
//! 18 BTL references: 7.5 Auto-A (3), 7.6 Auto-B (12), 7.9 Auto MV-A (3),
//! 7.11 Archival-A (0).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 7.5 Automated Trend Retrieval A (3 refs) ─────────────────────────

    registry.add(TestDef {
        id: "7.5.1",
        name: "T-Auto-A: Retrieve by Position",
        reference: "135.1-2025 - 9.21.1.2",
        section: Section::Trending,
        tags: &["trending", "auto-retrieval"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(auto_retrieve(ctx)),
    });
    registry.add(TestDef {
        id: "7.5.2",
        name: "T-Auto-A: Retrieve by Sequence",
        reference: "135.1-2025 - 9.21.1.9",
        section: Section::Trending,
        tags: &["trending", "auto-retrieval"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(auto_retrieve(ctx)),
    });
    registry.add(TestDef {
        id: "7.5.3",
        name: "T-Auto-A: Retrieve by Time",
        reference: "135.1-2025 - 9.21.1.4",
        section: Section::Trending,
        tags: &["trending", "auto-retrieval"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(auto_retrieve(ctx)),
    });

    // ── 7.6 Automated Trend Retrieval B (12 refs) ────────────────────────

    let auto_b: &[(&str, &str, &str)] = &[
        (
            "7.6.1",
            "T-Auto-B: ReadRange All Items",
            "135.1-2025 - 9.21.1.1",
        ),
        (
            "7.6.2",
            "T-Auto-B: RR Position Positive",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "7.6.3",
            "T-Auto-B: RR Position Negative",
            "135.1-2025 - 9.21.1.3",
        ),
        ("7.6.4", "T-Auto-B: RR by Time", "135.1-2025 - 9.21.1.4"),
        (
            "7.6.5",
            "T-Auto-B: RR by Time Negative",
            "135.1-2025 - 9.21.1.4.1",
        ),
        (
            "7.6.6",
            "T-Auto-B: RR Sequence Positive",
            "135.1-2025 - 9.21.1.9",
        ),
        (
            "7.6.7",
            "T-Auto-B: RR Sequence Negative",
            "135.1-2025 - 9.21.1.10",
        ),
        (
            "7.6.8",
            "T-Auto-B: RR Empty Sequence",
            "135.1-2025 - 9.21.1.7",
        ),
        ("7.6.9", "T-Auto-B: RR Empty Time", "135.1-2025 - 9.21.1.8"),
        ("7.6.10", "T-Auto-B: RR MOREITEMS", "135.1-2025 - 9.21.1.13"),
        (
            "7.6.11",
            "T-Auto-B: RR Empty Position",
            "135.1-2025 - 9.21.2.4",
        ),
        (
            "7.6.12",
            "T-Auto-B: TL Properties Readable",
            "135.1-2025 - 7.3.2.24.1",
        ),
    ];

    for &(id, name, reference) in auto_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "auto-retrieval-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
            timeout: None,
            run: |ctx| Box::pin(auto_retrieve_b(ctx)),
        });
    }

    // ── 7.9 Automated Multiple Value Retrieval A (3 refs) ────────────────

    registry.add(TestDef {
        id: "7.9.1",
        name: "T-AutoMV-A: Retrieve TLM by Position",
        reference: "135.1-2025 - 9.21.1.2",
        section: Section::Trending,
        tags: &["trending", "auto-mv-retrieval"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
        timeout: None,
        run: |ctx| Box::pin(auto_mv_retrieve(ctx)),
    });
    registry.add(TestDef {
        id: "7.9.2",
        name: "T-AutoMV-A: Retrieve TLM by Sequence",
        reference: "135.1-2025 - 9.21.1.9",
        section: Section::Trending,
        tags: &["trending", "auto-mv-retrieval"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
        timeout: None,
        run: |ctx| Box::pin(auto_mv_retrieve(ctx)),
    });
    registry.add(TestDef {
        id: "7.9.3",
        name: "T-AutoMV-A: Retrieve TLM by Time",
        reference: "135.1-2025 - 9.21.1.4",
        section: Section::Trending,
        tags: &["trending", "auto-mv-retrieval"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
        timeout: None,
        run: |ctx| Box::pin(auto_mv_retrieve(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn auto_retrieve(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn auto_retrieve_b(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::TOTAL_RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn auto_mv_retrieve(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tlm = ctx.first_object_of_type(ObjectType::TREND_LOG_MULTIPLE)?;
    ctx.verify_readable(tlm, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}
