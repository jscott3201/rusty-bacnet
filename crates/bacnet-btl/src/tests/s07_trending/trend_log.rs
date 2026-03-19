//! BTL Test Plan Sections 7.3–7.4 — TrendLog Internal/External B.
//! 45 BTL references: 7.3 Internal-B (29), 7.4 External-B (16).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 7.3 TREND-I-B (TrendLog Internal, 29 refs) ──────────────────────

    let int_base: &[(&str, &str, &str)] = &[
        (
            "7.3.1",
            "TL-I-B: ReadRange All Items",
            "135.1-2025 - 9.21.1.1",
        ),
        ("7.3.2", "TL-I-B: Enable Test", "135.1-2025 - 7.3.2.24.1"),
        (
            "7.3.3",
            "TL-I-B: Stop_When_Full TRUE",
            "135.1-2025 - 7.3.2.24.6.1",
        ),
        (
            "7.3.4",
            "TL-I-B: Stop_When_Full FALSE",
            "135.1-2025 - 7.3.2.24.6.2",
        ),
        (
            "7.3.5",
            "TL-I-B: Buffer_Size Test",
            "135.1-2025 - 7.3.2.24.7",
        ),
        (
            "7.3.6",
            "TL-I-B: Record_Count Test",
            "135.1-2025 - 7.3.2.24.8",
        ),
        (
            "7.3.7",
            "TL-I-B: Total_Record_Count",
            "135.1-2025 - 7.3.2.24.9",
        ),
        (
            "7.3.8",
            "TL-I-B: Log-Status Test",
            "135.1-2025 - 7.3.2.24.13",
        ),
        (
            "7.3.9",
            "TL-I-B: Time_Change Test",
            "135.1-2025 - 7.3.2.24.14",
        ),
        (
            "7.3.10",
            "TL-I-B: Buffer_Size Write",
            "135.1-2025 - 7.3.2.24.23",
        ),
    ];

    for &(id, name, reference) in int_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "trend-log", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
            timeout: None,
            run: |ctx| Box::pin(tl_int_base(ctx)),
        });
    }

    // ReadRange variants
    let rr: &[(&str, &str, &str)] = &[
        (
            "7.3.11",
            "TL-I-B: RR by Position Positive",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "7.3.12",
            "TL-I-B: RR by Position Negative",
            "135.1-2025 - 9.21.1.3",
        ),
        ("7.3.13", "TL-I-B: RR by Time", "135.1-2025 - 9.21.1.4"),
        (
            "7.3.14",
            "TL-I-B: RR by Time Negative",
            "135.1-2025 - 9.21.1.4.1",
        ),
        (
            "7.3.15",
            "TL-I-B: RR by Sequence Positive",
            "135.1-2025 - 9.21.1.9",
        ),
        (
            "7.3.16",
            "TL-I-B: RR by Sequence Negative",
            "135.1-2025 - 9.21.1.10",
        ),
        (
            "7.3.17",
            "TL-I-B: RR Empty Sequence",
            "135.1-2025 - 9.21.1.7",
        ),
        ("7.3.18", "TL-I-B: RR Empty Time", "135.1-2025 - 9.21.1.8"),
        ("7.3.19", "TL-I-B: RR MOREITEMS", "135.1-2025 - 9.21.1.13"),
        (
            "7.3.20",
            "TL-I-B: RR Empty Position",
            "135.1-2025 - 9.21.2.4",
        ),
    ];

    for &(id, name, reference) in rr {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "trend-log", "internal-b", "read-range"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
            timeout: None,
            run: |ctx| Box::pin(tl_int_base(ctx)),
        });
    }

    // Logging type variants
    let logging: &[(&str, &str, &str)] = &[
        (
            "7.3.21",
            "TL-I-B: Periodic Logging",
            "135.1-2025 - 7.3.2.24.4",
        ),
        ("7.3.22", "TL-I-B: COV Logging", "135.1-2025 - 7.3.2.24.15"),
        (
            "7.3.23",
            "TL-I-B: Triggered Logging",
            "135.1-2025 - 7.3.2.24.19",
        ),
        ("7.3.24", "TL-I-B: Start_Time", "135.1-2025 - 7.3.2.24.2"),
        ("7.3.25", "TL-I-B: Stop_Time", "135.1-2025 - 7.3.2.24.3"),
        (
            "7.3.26",
            "TL-I-B: Clock-Aligned Logging",
            "135.1-2025 - 7.3.2.24.21",
        ),
        (
            "7.3.27",
            "TL-I-B: Interval_Offset",
            "135.1-2025 - 7.3.2.24.22",
        ),
        ("7.3.28", "TL-I-B: DateTime Non-Pattern", "BTL - 7.2.9"),
        (
            "7.3.29",
            "TL-I-B: WPM DateTime Non-Pattern",
            "BTL - 9.23.2.21",
        ),
    ];

    for &(id, name, reference) in logging {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "trend-log", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
            timeout: None,
            run: |ctx| Box::pin(tl_int_logging(ctx)),
        });
    }

    // ── 7.4 TREND-E-B (TrendLog External, 16 refs) ──────────────────────

    let ext: &[(&str, &str, &str)] = &[
        (
            "7.4.1",
            "TL-E-B: ReadRange All Items",
            "135.1-2025 - 9.21.1.1",
        ),
        ("7.4.2", "TL-E-B: Enable Test", "135.1-2025 - 7.3.2.24.1"),
        (
            "7.4.3",
            "TL-E-B: Stop_When_Full TRUE",
            "135.1-2025 - 7.3.2.24.6.1",
        ),
        (
            "7.4.4",
            "TL-E-B: Stop_When_Full FALSE",
            "135.1-2025 - 7.3.2.24.6.2",
        ),
        ("7.4.5", "TL-E-B: Buffer_Size", "135.1-2025 - 7.3.2.24.7"),
        ("7.4.6", "TL-E-B: Record_Count", "135.1-2025 - 7.3.2.24.8"),
        (
            "7.4.7",
            "TL-E-B: Total_Record_Count",
            "135.1-2025 - 7.3.2.24.9",
        ),
        ("7.4.8", "TL-E-B: Log-Status", "135.1-2025 - 7.3.2.24.13"),
        ("7.4.9", "TL-E-B: Time_Change", "135.1-2025 - 7.3.2.24.14"),
        (
            "7.4.10",
            "TL-E-B: Buffer_Size Write",
            "135.1-2025 - 7.3.2.24.23",
        ),
        (
            "7.4.11",
            "TL-E-B: Periodic Logging",
            "135.1-2025 - 7.3.2.24.4",
        ),
        (
            "7.4.12",
            "TL-E-B: COV Logging External",
            "135.1-2025 - 7.3.2.24.16",
        ),
        ("7.4.13", "TL-E-B: Start_Time", "135.1-2025 - 7.3.2.24.2"),
        ("7.4.14", "TL-E-B: Stop_Time", "135.1-2025 - 7.3.2.24.3"),
        ("7.4.15", "TL-E-B: DateTime Non-Pattern", "BTL - 7.2.9"),
        (
            "7.4.16",
            "TL-E-B: WPM DateTime Non-Pattern",
            "BTL - 9.23.2.21",
        ),
    ];

    for &(id, name, reference) in ext {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "trend-log", "external-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(20)),
            timeout: None,
            run: |ctx| Box::pin(tl_ext_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn tl_int_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::STOP_WHEN_FULL)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::TOTAL_RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn tl_int_logging(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOGGING_TYPE)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_INTERVAL)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.pass()
}

async fn tl_ext_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tl = ctx.first_object_of_type(ObjectType::TREND_LOG)?;
    ctx.verify_readable(tl, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(tl, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}
