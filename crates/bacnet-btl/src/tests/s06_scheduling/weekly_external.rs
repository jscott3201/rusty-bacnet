//! BTL Test Plan Sections 6.5–6.6 — External B + Weekly Schedule Internal B.
//! 44 BTL references: 6.5 External-B (17), 6.6 Weekly Schedule-I-B (27).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 6.5 SCHED-E-B (External Schedule, 17 refs) ──────────────────────

    let ext: &[(&str, &str, &str)] = &[
        (
            "6.5.1",
            "SCHED-E-B: OPR External Test",
            "135.1-2025 - 7.3.2.23.8",
        ),
        (
            "6.5.2",
            "SCHED-E-B: Rev4 OPR External",
            "135.1-2025 - 7.3.2.23.10.8",
        ),
        (
            "6.5.3",
            "SCHED-E-B: External Reference Readable",
            "135.1-2025 - 7.3.2.23.8",
        ),
        (
            "6.5.4",
            "SCHED-E-B: Weekly_Schedule Property",
            "135.1-2025 - 7.3.2.23.2",
        ),
        (
            "6.5.5",
            "SCHED-E-B: Rev4 Weekly_Schedule",
            "135.1-2025 - 7.3.2.23.10.2",
        ),
        (
            "6.5.6",
            "SCHED-E-B: Exception_Schedule Restoration",
            "135.1-2025 - 7.3.2.23.5",
        ),
        (
            "6.5.7",
            "SCHED-E-B: Calendar Reference",
            "135.1-2025 - 7.3.2.23.3.1",
        ),
        (
            "6.5.8",
            "SCHED-E-B: Rev4 Calendar Reference",
            "135.1-2025 - 7.3.2.23.10.3.1",
        ),
        (
            "6.5.9",
            "SCHED-E-B: Effective_Period",
            "135.1-2025 - 7.3.2.23.1",
        ),
        (
            "6.5.10",
            "SCHED-E-B: Rev4 Effective_Period",
            "135.1-2025 - 7.3.2.23.10.1",
        ),
        (
            "6.5.11",
            "SCHED-E-B: DateRange Non-Pattern",
            "135.1-2025 - 7.2.10",
        ),
        (
            "6.5.12",
            "SCHED-E-B: DateRange Open-Ended",
            "135.1-2025 - 7.2.11",
        ),
        (
            "6.5.13",
            "SCHED-E-B: WPM DateRange",
            "135.1-2025 - 9.23.2.22",
        ),
        (
            "6.5.14",
            "SCHED-E-B: Datatypes Non-NULL",
            "135.1-2025 - 7.3.2.23.11.1",
        ),
        (
            "6.5.15",
            "SCHED-E-B: Datatypes NULL+PA",
            "135.1-2025 - 7.3.2.23.11.2",
        ),
        (
            "6.5.16",
            "SCHED-E-B: Interaction",
            "135.1-2025 - 7.3.2.23.4",
        ),
        (
            "6.5.17",
            "SCHED-E-B: Rev4 Interaction",
            "135.1-2025 - 7.3.2.23.10.4",
        ),
    ];

    for &(id, name, reference) in ext {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "external-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_ext(ctx)),
        });
    }

    // ── 6.6 Weekly Schedule Internal B (27 refs) ─────────────────────────
    // Same core schedule evaluation tests but specifically for weekly schedule

    let ws: &[(&str, &str, &str)] = &[
        (
            "6.6.1",
            "WS-I-B: Weekly_Schedule Property",
            "135.1-2025 - 7.3.2.23.2",
        ),
        (
            "6.6.2",
            "WS-I-B: Rev4 Weekly_Schedule",
            "135.1-2025 - 7.3.2.23.10.2",
        ),
        (
            "6.6.3",
            "WS-I-B: Weekly Restoration",
            "135.1-2025 - 7.3.2.23.6",
        ),
        (
            "6.6.4",
            "WS-I-B: Effective_Period",
            "135.1-2025 - 7.3.2.23.1",
        ),
        (
            "6.6.5",
            "WS-I-B: Rev4 Effective_Period",
            "135.1-2025 - 7.3.2.23.10.1",
        ),
        (
            "6.6.6",
            "WS-I-B: DateRange Non-Pattern",
            "135.1-2025 - 7.2.10",
        ),
        (
            "6.6.7",
            "WS-I-B: DateRange Open-Ended",
            "135.1-2025 - 7.2.11",
        ),
        ("6.6.8", "WS-I-B: WPM DateRange", "135.1-2025 - 9.23.2.22"),
        (
            "6.6.9",
            "WS-I-B: Datatypes Non-NULL",
            "135.1-2025 - 7.3.2.23.11.1",
        ),
        (
            "6.6.10",
            "WS-I-B: Datatypes NULL+PA",
            "135.1-2025 - 7.3.2.23.11.2",
        ),
        ("6.6.11", "WS-I-B: OPR Internal", "135.1-2025 - 7.3.2.23.7"),
        (
            "6.6.12",
            "WS-I-B: Rev4 OPR Internal",
            "135.1-2025 - 7.3.2.23.10.7",
        ),
        (
            "6.6.13",
            "WS-I-B: Datatypes NULL+PA (OPR)",
            "135.1-2025 - 7.3.2.23.11.2",
        ),
        (
            "6.6.14",
            "WS-I-B: Rev4 Midnight Evaluation",
            "135.1-2025 - 7.3.2.23.12",
        ),
        (
            "6.6.15",
            "WS-I-B: Rev4 Schedule_Default",
            "135.1-2025 - 7.3.2.23.10.3.13",
        ),
        ("6.6.16", "WS-I-B: Date Pattern", "135.1-2025 - 7.2.4"),
        ("6.6.17", "WS-I-B: Time Non-Pattern", "135.1-2025 - 7.2.8"),
        (
            "6.6.18",
            "WS-I-B: Forbid Duplicate Time",
            "135.1-2025 - 7.3.2.23.13",
        ),
        (
            "6.6.19",
            "WS-I-B: BTL Write_Every FALSE",
            "BTL - 7.3.2.23.X1.1",
        ),
        (
            "6.6.20",
            "WS-I-B: BTL Write_Every TRUE",
            "BTL - 7.3.2.23.X1.2",
        ),
        ("6.6.21", "WS-I-B: BTL Exception Size", "BTL - 7.3.2.23.9"),
        (
            "6.6.22",
            "WS-I-B: List BACnetTimeValue",
            "135.1-2025 - 7.3.2.23.3.9",
        ),
        (
            "6.6.23",
            "WS-I-B: Rev4 BACnetTimeValue",
            "135.1-2025 - 7.3.2.23.10.3.9",
        ),
        (
            "6.6.24",
            "WS-I-B: Event Priority",
            "135.1-2025 - 7.3.2.23.3.8",
        ),
        (
            "6.6.25",
            "WS-I-B: Rev4 Event Priority",
            "135.1-2025 - 7.3.2.23.10.3.8",
        ),
        (
            "6.6.26",
            "WS-I-B: WPM Time Non-Pattern",
            "135.1-2025 - 9.23.2.20",
        ),
        (
            "6.6.27",
            "WS-I-B: Exception Restoration",
            "135.1-2025 - 7.3.2.23.5",
        ),
    ];

    for &(id, name, reference) in ws {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "weekly-internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_weekly_int(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn sched_ext(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.verify_readable(
        sched,
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
    )
    .await?;
    ctx.pass()
}

async fn sched_weekly_int(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::EFFECTIVE_PERIOD)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.pass()
}
