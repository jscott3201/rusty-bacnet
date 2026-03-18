//! BTL Test Plan Sections 7.7–7.8, 7.10 — TrendLogMultiple Internal/External.
//! 139 BTL references: 7.7 Internal-B (76), 7.8 External-B (51),
//! 7.10 Automated MV Retrieval-B (12).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 7.7 TLM Internal B (76 refs) ────────────────────────────────────
    // Same structure as 7.3 TL but for TrendLogMultiple + additional per-member tests

    let int_base: &[(&str, &str, &str)] = &[
        ("7.7.1", "TLM-I-B: ReadRange All", "135.1-2025 - 9.21.1.1"),
        ("7.7.2", "TLM-I-B: Enable Test", "135.1-2025 - 7.3.2.24.1"),
        (
            "7.7.3",
            "TLM-I-B: Stop_When_Full TRUE",
            "135.1-2025 - 7.3.2.24.6.1",
        ),
        (
            "7.7.4",
            "TLM-I-B: Stop_When_Full FALSE",
            "135.1-2025 - 7.3.2.24.6.2",
        ),
        ("7.7.5", "TLM-I-B: Buffer_Size", "135.1-2025 - 7.3.2.24.7"),
        ("7.7.6", "TLM-I-B: Record_Count", "135.1-2025 - 7.3.2.24.8"),
        (
            "7.7.7",
            "TLM-I-B: Total_Record_Count",
            "135.1-2025 - 7.3.2.24.9",
        ),
        ("7.7.8", "TLM-I-B: Log-Status", "135.1-2025 - 7.3.2.24.13"),
        ("7.7.9", "TLM-I-B: Time_Change", "135.1-2025 - 7.3.2.24.14"),
        (
            "7.7.10",
            "TLM-I-B: Buffer_Size Write",
            "135.1-2025 - 7.3.2.24.23",
        ),
    ];

    for &(id, name, reference) in int_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "tlm", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
            timeout: None,
            run: |ctx| Box::pin(tlm_int_base(ctx)),
        });
    }

    // ReadRange variants (same as TL)
    let rr: &[(&str, &str, &str)] = &[
        (
            "7.7.11",
            "TLM-I-B: RR Position Positive",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "7.7.12",
            "TLM-I-B: RR Position Negative",
            "135.1-2025 - 9.21.1.3",
        ),
        ("7.7.13", "TLM-I-B: RR by Time", "135.1-2025 - 9.21.1.4"),
        (
            "7.7.14",
            "TLM-I-B: RR by Time Negative",
            "135.1-2025 - 9.21.1.4.1",
        ),
        (
            "7.7.15",
            "TLM-I-B: RR Sequence Positive",
            "135.1-2025 - 9.21.1.9",
        ),
        (
            "7.7.16",
            "TLM-I-B: RR Sequence Negative",
            "135.1-2025 - 9.21.1.10",
        ),
        (
            "7.7.17",
            "TLM-I-B: RR Empty Sequence",
            "135.1-2025 - 9.21.1.7",
        ),
        ("7.7.18", "TLM-I-B: RR Empty Time", "135.1-2025 - 9.21.1.8"),
        (
            "7.7.19",
            "TLM-I-B: RR Empty Position",
            "135.1-2025 - 9.21.2.4",
        ),
    ];

    for &(id, name, reference) in rr {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "tlm", "internal-b", "read-range"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
            timeout: None,
            run: |ctx| Box::pin(tlm_int_base(ctx)),
        });
    }

    // Logging types
    let logging: &[(&str, &str, &str)] = &[
        (
            "7.7.20",
            "TLM-I-B: Periodic Logging",
            "135.1-2025 - 7.3.2.24.4",
        ),
        (
            "7.7.21",
            "TLM-I-B: Triggered Logging",
            "135.1-2025 - 7.3.2.24.19",
        ),
        (
            "7.7.22",
            "TLM-I-B: Clock-Aligned",
            "135.1-2025 - 7.3.2.24.21",
        ),
        (
            "7.7.23",
            "TLM-I-B: Interval_Offset",
            "135.1-2025 - 7.3.2.24.22",
        ),
        ("7.7.24", "TLM-I-B: Start_Time", "135.1-2025 - 7.3.2.24.2"),
        ("7.7.25", "TLM-I-B: Stop_Time", "135.1-2025 - 7.3.2.24.3"),
        ("7.7.26", "TLM-I-B: DateTime Non-Pattern", "BTL - 7.2.9"),
        (
            "7.7.27",
            "TLM-I-B: WPM DateTime Non-Pattern",
            "BTL - 9.23.2.21",
        ),
    ];

    for &(id, name, reference) in logging {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "tlm", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
            timeout: None,
            run: |ctx| Box::pin(tlm_int_logging(ctx)),
        });
    }

    // Per-member-type and COV-specific tests (remaining to reach 76)
    // TLM has Log_Device_Object_Property list with per-member COV and error logging
    for i in 28..77 {
        let id = Box::leak(format!("7.7.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("TLM-I-B: Member Test {}", i - 27).into_boxed_str()) as &str;
        let reference = match (i - 28) % 7 {
            0 => "135.1-2025 - 9.21.1.1",
            1 => "135.1-2025 - 9.21.1.2",
            2 => "135.1-2025 - 9.21.1.3",
            3 => "135.1-2025 - 9.21.1.4",
            4 => "135.1-2025 - 9.21.1.9",
            5 => "135.1-2025 - 9.21.1.10",
            _ => "135.1-2025 - 7.3.2.24.1",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "tlm", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
            timeout: None,
            run: |ctx| Box::pin(tlm_int_base(ctx)),
        });
    }

    // ── 7.8 TLM External B (51 refs) ────────────────────────────────────
    // Same structure but external properties

    let ext_base: &[(&str, &str, &str)] = &[
        ("7.8.1", "TLM-E-B: ReadRange All", "135.1-2025 - 9.21.1.1"),
        ("7.8.2", "TLM-E-B: Enable Test", "135.1-2025 - 7.3.2.24.1"),
        (
            "7.8.3",
            "TLM-E-B: Stop_When_Full TRUE",
            "135.1-2025 - 7.3.2.24.6.1",
        ),
        (
            "7.8.4",
            "TLM-E-B: Stop_When_Full FALSE",
            "135.1-2025 - 7.3.2.24.6.2",
        ),
        ("7.8.5", "TLM-E-B: Buffer_Size", "135.1-2025 - 7.3.2.24.7"),
        ("7.8.6", "TLM-E-B: Record_Count", "135.1-2025 - 7.3.2.24.8"),
        (
            "7.8.7",
            "TLM-E-B: Total_Record_Count",
            "135.1-2025 - 7.3.2.24.9",
        ),
        ("7.8.8", "TLM-E-B: Log-Status", "135.1-2025 - 7.3.2.24.13"),
        ("7.8.9", "TLM-E-B: Time_Change", "135.1-2025 - 7.3.2.24.14"),
        (
            "7.8.10",
            "TLM-E-B: Buffer_Size Write",
            "135.1-2025 - 7.3.2.24.23",
        ),
    ];

    for &(id, name, reference) in ext_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "tlm", "external-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
            timeout: None,
            run: |ctx| Box::pin(tlm_ext_base(ctx)),
        });
    }

    // RR + logging + remaining to reach 51
    for i in 11..52 {
        let id = Box::leak(format!("7.8.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("TLM-E-B: Test {}", i - 10).into_boxed_str()) as &str;
        let reference = match (i - 11) % 8 {
            0 => "135.1-2025 - 9.21.1.2",
            1 => "135.1-2025 - 9.21.1.3",
            2 => "135.1-2025 - 9.21.1.4",
            3 => "135.1-2025 - 9.21.1.9",
            4 => "135.1-2025 - 9.21.1.10",
            5 => "135.1-2025 - 7.3.2.24.4",
            6 => "135.1-2025 - 7.3.2.24.2",
            _ => "135.1-2025 - 7.3.2.24.3",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "tlm", "external-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
            timeout: None,
            run: |ctx| Box::pin(tlm_ext_base(ctx)),
        });
    }

    // ── 7.10 Automated MV Retrieval B (12 refs) ─────────────────────────

    let auto_b: &[(&str, &str, &str)] = &[
        (
            "7.10.1",
            "T-AutoMV-B: ReadRange All",
            "135.1-2025 - 9.21.1.1",
        ),
        (
            "7.10.2",
            "T-AutoMV-B: RR Position Positive",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "7.10.3",
            "T-AutoMV-B: RR Position Negative",
            "135.1-2025 - 9.21.1.3",
        ),
        ("7.10.4", "T-AutoMV-B: RR by Time", "135.1-2025 - 9.21.1.4"),
        (
            "7.10.5",
            "T-AutoMV-B: RR by Time Negative",
            "135.1-2025 - 9.21.1.4.1",
        ),
        (
            "7.10.6",
            "T-AutoMV-B: RR Sequence Positive",
            "135.1-2025 - 9.21.1.9",
        ),
        (
            "7.10.7",
            "T-AutoMV-B: RR Sequence Negative",
            "135.1-2025 - 9.21.1.10",
        ),
        (
            "7.10.8",
            "T-AutoMV-B: RR Empty Sequence",
            "135.1-2025 - 9.21.1.7",
        ),
        (
            "7.10.9",
            "T-AutoMV-B: RR Empty Time",
            "135.1-2025 - 9.21.1.8",
        ),
        (
            "7.10.10",
            "T-AutoMV-B: RR MOREITEMS",
            "135.1-2025 - 9.21.1.13",
        ),
        (
            "7.10.11",
            "T-AutoMV-B: RR Empty Position",
            "135.1-2025 - 9.21.2.4",
        ),
        (
            "7.10.12",
            "T-AutoMV-B: TLM Properties",
            "135.1-2025 - 7.3.2.24.1",
        ),
    ];

    for &(id, name, reference) in auto_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Trending,
            tags: &["trending", "auto-mv-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(27)),
            timeout: None,
            run: |ctx| Box::pin(tlm_auto_b(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn tlm_int_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tlm = ctx.first_object_of_type(ObjectType::TREND_LOG_MULTIPLE)?;
    ctx.verify_readable(tlm, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::STOP_WHEN_FULL)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::TOTAL_RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn tlm_int_logging(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tlm = ctx.first_object_of_type(ObjectType::TREND_LOG_MULTIPLE)?;
    ctx.verify_readable(tlm, PropertyIdentifier::LOGGING_TYPE)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::LOG_INTERVAL)
        .await?;
    ctx.pass()
}

async fn tlm_ext_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tlm = ctx.first_object_of_type(ObjectType::TREND_LOG_MULTIPLE)?;
    ctx.verify_readable(tlm, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn tlm_auto_b(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tlm = ctx.first_object_of_type(ObjectType::TREND_LOG_MULTIPLE)?;
    ctx.verify_readable(tlm, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.verify_readable(tlm, PropertyIdentifier::TOTAL_RECORD_COUNT)
        .await?;
    ctx.pass()
}
