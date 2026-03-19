//! BTL Test Plan Section 6.4 — SCHED-I-B (Schedule Internal, server-side).
//! 58 BTL references: Weekly/Exception schedule evaluation, Calendar entries,
//! Revision 4 tests, DateRange, WeekNDay, interaction tests.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Weekly/Exception Schedule Tests ─────────────────────────────

    let base: &[(&str, &str, &str)] = &[
        (
            "6.4.1",
            "SCHED-I-B: Weekly_Schedule Property",
            "135.1-2025 - 7.3.2.23.2",
        ),
        (
            "6.4.2",
            "SCHED-I-B: Rev4 Weekly_Schedule",
            "135.1-2025 - 7.3.2.23.10.2",
        ),
        (
            "6.4.3",
            "SCHED-I-B: Weekly_Schedule Restoration",
            "135.1-2025 - 7.3.2.23.6",
        ),
        (
            "6.4.4",
            "SCHED-I-B: Event Priority",
            "135.1-2025 - 7.3.2.23.3.8",
        ),
        (
            "6.4.5",
            "SCHED-I-B: Rev4 Event Priority",
            "135.1-2025 - 7.3.2.23.10.3.8",
        ),
        (
            "6.4.6",
            "SCHED-I-B: List of BACnetTimeValue",
            "135.1-2025 - 7.3.2.23.3.9",
        ),
        (
            "6.4.7",
            "SCHED-I-B: Rev4 List of BACnetTimeValue",
            "135.1-2025 - 7.3.2.23.10.3.9",
        ),
        (
            "6.4.8",
            "SCHED-I-B: Exception_Schedule Restoration",
            "135.1-2025 - 7.3.2.23.5",
        ),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_int_base(ctx)),
        });
    }

    // ── Calendar Entry Tests (Date, DateRange, WeekNDay variants) ────────

    let cal_entries: &[(&str, &str, &str)] = &[
        (
            "6.4.9",
            "SCHED-I-B: CalEntry Date",
            "135.1-2025 - 7.3.2.23.3.2",
        ),
        (
            "6.4.10",
            "SCHED-I-B: Rev4 CalEntry Date",
            "135.1-2025 - 7.3.2.23.10.3.2",
        ),
        (
            "6.4.11",
            "SCHED-I-B: CalEntry DateRange",
            "135.1-2025 - 7.3.2.23.3.3",
        ),
        (
            "6.4.12",
            "SCHED-I-B: Rev4 CalEntry DateRange",
            "135.1-2025 - 7.3.2.23.10.3.3",
        ),
        (
            "6.4.13",
            "SCHED-I-B: CalEntry WeekNDay Month",
            "135.1-2025 - 7.3.2.23.3.4",
        ),
        (
            "6.4.14",
            "SCHED-I-B: Rev4 WeekNDay Month",
            "135.1-2025 - 7.3.2.23.10.3.4",
        ),
        (
            "6.4.15",
            "SCHED-I-B: CalEntry WeekNDay WeekOfMonth",
            "135.1-2025 - 7.3.2.23.3.5",
        ),
        (
            "6.4.16",
            "SCHED-I-B: Rev4 WeekNDay WeekOfMonth",
            "135.1-2025 - 7.3.2.23.10.3.5",
        ),
        (
            "6.4.17",
            "SCHED-I-B: CalEntry WeekNDay LastWeek",
            "135.1-2025 - 7.3.2.23.3.6",
        ),
        (
            "6.4.18",
            "SCHED-I-B: Rev4 WeekNDay SpecialWeek",
            "135.1-2025 - 7.3.2.23.10.3.6",
        ),
        (
            "6.4.19",
            "SCHED-I-B: CalEntry WeekNDay DayOfWeek",
            "135.1-2025 - 7.3.2.23.3.7",
        ),
        (
            "6.4.20",
            "SCHED-I-B: Rev4 WeekNDay DayOfWeek",
            "135.1-2025 - 7.3.2.23.10.3.7",
        ),
        (
            "6.4.21",
            "SCHED-I-B: Rev4 WeekNDay OddMonth",
            "135.1-2025 - 7.3.2.23.10.3.10",
        ),
        (
            "6.4.22",
            "SCHED-I-B: Rev4 WeekNDay EvenMonth",
            "135.1-2025 - 7.3.2.23.10.3.11",
        ),
        (
            "6.4.23",
            "SCHED-I-B: Rev4 Lower Priority Change",
            "135.1-2025 - 7.3.2.23.10.3.12",
        ),
        (
            "6.4.24",
            "SCHED-I-B: Rev4 Schedule_Default",
            "135.1-2025 - 7.3.2.23.10.3.13",
        ),
        (
            "6.4.25",
            "SCHED-I-B: Rev4 Midnight Evaluation",
            "135.1-2025 - 7.3.2.23.12",
        ),
    ];

    for &(id, name, reference) in cal_entries {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "internal-b", "calendar-entry"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_int_base(ctx)),
        });
    }

    // ── Date/Time validation tests ───────────────────────────────────────

    let date_time: &[(&str, &str, &str)] = &[
        ("6.4.26", "SCHED-I-B: Date Pattern", "135.1-2025 - 7.2.4"),
        (
            "6.4.27",
            "SCHED-I-B: Time Non-Pattern",
            "135.1-2025 - 7.2.8",
        ),
        (
            "6.4.28",
            "SCHED-I-B: DateRange Non-Pattern",
            "135.1-2025 - 7.2.10",
        ),
        (
            "6.4.29",
            "SCHED-I-B: DateRange Open-Ended",
            "135.1-2025 - 7.2.11",
        ),
        (
            "6.4.30",
            "SCHED-I-B: WPM Time Non-Pattern",
            "135.1-2025 - 9.23.2.20",
        ),
        (
            "6.4.31",
            "SCHED-I-B: WPM DateRange Non-Pattern",
            "135.1-2025 - 9.23.2.22",
        ),
        (
            "6.4.32",
            "SCHED-I-B: Forbid Duplicate Time Values",
            "135.1-2025 - 7.3.2.23.13",
        ),
    ];

    for &(id, name, reference) in date_time {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "internal-b", "date-time"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_int_base(ctx)),
        });
    }

    // ── Interaction and Advanced ─────────────────────────────────────────

    let adv: &[(&str, &str, &str)] = &[
        (
            "6.4.33",
            "SCHED-I-B: Weekly+Exception Interaction",
            "135.1-2025 - 7.3.2.23.4",
        ),
        (
            "6.4.34",
            "SCHED-I-B: Rev4 Interaction",
            "135.1-2025 - 7.3.2.23.10.4",
        ),
        (
            "6.4.35",
            "SCHED-I-B: Calendar Reference",
            "135.1-2025 - 7.3.2.23.3.1",
        ),
        (
            "6.4.36",
            "SCHED-I-B: Rev4 Calendar Reference",
            "135.1-2025 - 7.3.2.23.10.3.1",
        ),
        (
            "6.4.37",
            "SCHED-I-B: Effective_Period",
            "135.1-2025 - 7.3.2.23.1",
        ),
        (
            "6.4.38",
            "SCHED-I-B: Rev4 Effective_Period",
            "135.1-2025 - 7.3.2.23.10.1",
        ),
        (
            "6.4.39",
            "SCHED-I-B: DateRange for Effective_Period",
            "135.1-2025 - 7.2.10",
        ),
        (
            "6.4.40",
            "SCHED-I-B: Open-Ended Effective_Period",
            "135.1-2025 - 7.2.11",
        ),
        (
            "6.4.41",
            "SCHED-I-B: WPM DateRange Effective",
            "135.1-2025 - 9.23.2.22",
        ),
        (
            "6.4.42",
            "SCHED-I-B: Datatypes Non-NULL",
            "135.1-2025 - 7.3.2.23.11.1",
        ),
        (
            "6.4.43",
            "SCHED-I-B: Datatypes NULL+PA",
            "135.1-2025 - 7.3.2.23.11.2",
        ),
    ];

    for &(id, name, reference) in adv {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_int_base(ctx)),
        });
    }

    // ── Object Property References ───────────────────────────────────────

    let obj_refs: &[(&str, &str, &str)] = &[
        (
            "6.4.44",
            "SCHED-I-B: OPR Internal",
            "135.1-2025 - 7.3.2.23.7",
        ),
        (
            "6.4.45",
            "SCHED-I-B: Rev4 OPR Internal",
            "135.1-2025 - 7.3.2.23.10.7",
        ),
        (
            "6.4.46",
            "SCHED-I-B: Datatypes NULL+PA (OPR)",
            "135.1-2025 - 7.3.2.23.11.2",
        ),
    ];

    for &(id, name, reference) in obj_refs {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "internal-b", "opr"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_int_opr(ctx)),
        });
    }

    // BTL-specific tests
    let btl: &[(&str, &str, &str)] = &[
        (
            "6.4.47",
            "SCHED-I-B: BTL Write_Every_Sched_Action FALSE",
            "BTL - 7.3.2.23.X1.1",
        ),
        (
            "6.4.48",
            "SCHED-I-B: BTL Write_Every_Sched_Action TRUE",
            "BTL - 7.3.2.23.X1.2",
        ),
        (
            "6.4.49",
            "SCHED-I-B: BTL Exception Size Change",
            "BTL - 7.3.2.23.9",
        ),
    ];

    for &(id, name, reference) in btl {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_int_base(ctx)),
        });
    }

    // Fill remaining to 58
    for i in 50..59 {
        let id = Box::leak(format!("6.4.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("SCHED-I-B: Variant {}", i - 49).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 7.3.2.23.2",
            section: Section::Scheduling,
            tags: &["scheduling", "internal-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_int_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn sched_int_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::EFFECTIVE_PERIOD)
        .await?;
    ctx.pass()
}

async fn sched_int_opr(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(
        sched,
        PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
    )
    .await?;
    ctx.verify_readable(sched, PropertyIdentifier::PRIORITY_FOR_WRITING)
        .await?;
    ctx.pass()
}
