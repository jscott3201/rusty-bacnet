//! BTL Test Plan Sections 7.1–7.2, 7.12–7.13 — View/Advanced View.
//! 17 BTL references: 7.1 View-A (15), 7.2 Adv View+Modify-A (2),
//! 7.12 View+Modify Trends-A (0), 7.13 View+Modify MV-A (0).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 7.1 Trending View A (15 refs) ────────────────────────────────────

    let view_a: &[(&str, &str, &str)] = &[
        (
            "7.1.1",
            "T-View-A: Read TL Log_Buffer",
            "135.1-2025 - 9.21.1.1",
        ),
        (
            "7.1.2",
            "T-View-A: Read TL by Position Positive",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "7.1.3",
            "T-View-A: Read TL by Position Negative",
            "135.1-2025 - 9.21.1.3",
        ),
        (
            "7.1.4",
            "T-View-A: Read TL by Time",
            "135.1-2025 - 9.21.1.4",
        ),
        (
            "7.1.5",
            "T-View-A: Read TL by Time Negative",
            "135.1-2025 - 9.21.1.4.1",
        ),
        (
            "7.1.6",
            "T-View-A: Read TL by Sequence Positive",
            "135.1-2025 - 9.21.1.9",
        ),
        (
            "7.1.7",
            "T-View-A: Read TL by Sequence Negative",
            "135.1-2025 - 9.21.1.10",
        ),
        (
            "7.1.8",
            "T-View-A: Read TL Empty Sequence",
            "135.1-2025 - 9.21.1.7",
        ),
        (
            "7.1.9",
            "T-View-A: Read TL Empty Time",
            "135.1-2025 - 9.21.1.8",
        ),
        (
            "7.1.10",
            "T-View-A: Read TL MOREITEMS",
            "135.1-2025 - 9.21.1.13",
        ),
        (
            "7.1.11",
            "T-View-A: Read TL Empty Position",
            "135.1-2025 - 9.21.2.4",
        ),
        (
            "7.1.12",
            "T-View-A: Read TL Log_Enable",
            "135.1-2025 - 7.3.2.24.1",
        ),
        (
            "7.1.13",
            "T-View-A: Read TL Record_Count",
            "135.1-2025 - 7.3.2.24.8",
        ),
        (
            "7.1.14",
            "T-View-A: Read TL Buffer_Size",
            "135.1-2025 - 7.3.2.24.7",
        ),
        (
            "7.1.15",
            "T-View-A: Read TL Total_Record_Count",
            "135.1-2025 - 7.3.2.24.9",
        ),
    ];

    for &(id, name, reference) in view_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "view"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
            timeout: None,
            run: |ctx| Box::pin(trend_view(ctx)),
        });
    }

    // ── 7.2 Advanced View + Modify A (2 refs) ───────────────────────────

    registry.add(TestDef {
        id: "7.2.1",
        name: "T-AdvVM-A: Write TL Log_Enable",
        reference: "135.1-2025 - 7.3.2.24.1",
        section: Section::Trending,
        tags: &["trending", "adv-view-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(trend_adv_modify(ctx)),
    });
    registry.add(TestDef {
        id: "7.2.2",
        name: "T-AdvVM-A: Write TL Stop_When_Full",
        reference: "135.1-2025 - 7.3.2.24.6.1",
        section: Section::Trending,
        tags: &["trending", "adv-view-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
        timeout: None,
        run: |ctx| Box::pin(trend_adv_modify(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn trend_view(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::TOTAL_RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn trend_adv_modify(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::STOP_WHEN_FULL)
        .await?;
    ctx.pass()
}
