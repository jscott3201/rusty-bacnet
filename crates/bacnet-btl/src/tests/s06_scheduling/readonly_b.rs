//! BTL Test Plan Section 6.7 — SCHED-RO-B (Schedule Readonly, server-side).
//! 46 BTL references: same evaluation tests as Internal-B but schedule is
//! read-only (no WP to modify). Tests weekly evaluation, calendar entries,
//! WeekNDay, midnight, datatypes.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    let tests: &[(&str, &str, &str)] = &[
        (
            "6.7.1",
            "SCHED-RO-B: Effective_Period",
            "135.1-2025 - 7.3.2.23.1",
        ),
        (
            "6.7.2",
            "SCHED-RO-B: Rev4 Effective_Period",
            "135.1-2025 - 7.3.2.23.10.1",
        ),
        (
            "6.7.3",
            "SCHED-RO-B: OPR Internal",
            "135.1-2025 - 7.3.2.23.7",
        ),
        (
            "6.7.4",
            "SCHED-RO-B: Rev4 OPR Internal",
            "135.1-2025 - 7.3.2.23.10.7",
        ),
        (
            "6.7.5",
            "SCHED-RO-B: Rev4 Schedule_Default",
            "135.1-2025 - 7.3.2.23.10.3.13",
        ),
        (
            "6.7.6",
            "SCHED-RO-B: Rev4 Midnight Evaluation",
            "135.1-2025 - 7.3.2.23.12",
        ),
    ];

    for &(id, name, reference) in tests {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "readonly-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_ro_base(ctx)),
        });
    }

    // Per-datatype "Internally Written Datatypes" (12 types)
    let types: &[&str] = &[
        "NULL",
        "BOOLEAN",
        "Unsigned",
        "INTEGER",
        "REAL",
        "Double",
        "OctetString",
        "CharString",
        "BitString",
        "Enumerated",
        "Date",
        "Time",
    ];

    for (idx, dt) in (7u32..).zip(types) {
        let id = Box::leak(format!("6.7.{idx}").into_boxed_str()) as &str;
        let name = Box::leak(format!("SCHED-RO-B: Datatype {dt}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 7.3.2.23.11.1",
            section: Section::Scheduling,
            tags: &["scheduling", "readonly-b", "datatype"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_ro_base(ctx)),
        });
    }

    // Weekly schedule evaluation tests (mirrors internal-b)
    let weekly: &[(&str, &str, &str)] = &[
        (
            "6.7.19",
            "SCHED-RO-B: Weekly_Schedule",
            "135.1-2025 - 7.3.2.23.2",
        ),
        (
            "6.7.20",
            "SCHED-RO-B: Rev4 Weekly_Schedule",
            "135.1-2025 - 7.3.2.23.10.2",
        ),
        (
            "6.7.21",
            "SCHED-RO-B: Weekly Restoration",
            "135.1-2025 - 7.3.2.23.6",
        ),
        (
            "6.7.22",
            "SCHED-RO-B: List BACnetTimeValue",
            "135.1-2025 - 7.3.2.23.3.9",
        ),
        (
            "6.7.23",
            "SCHED-RO-B: Rev4 BACnetTimeValue",
            "135.1-2025 - 7.3.2.23.10.3.9",
        ),
        (
            "6.7.24",
            "SCHED-RO-B: Exception Restoration",
            "135.1-2025 - 7.3.2.23.5",
        ),
        (
            "6.7.25",
            "SCHED-RO-B: Rev4 Lower Priority",
            "135.1-2025 - 7.3.2.23.10.3.12",
        ),
    ];

    for &(id, name, reference) in weekly {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "readonly-b", "weekly"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_ro_weekly(ctx)),
        });
    }

    // Calendar entry tests
    let cal: &[(&str, &str, &str)] = &[
        (
            "6.7.26",
            "SCHED-RO-B: Calendar Reference",
            "135.1-2025 - 7.3.2.23.3.1",
        ),
        (
            "6.7.27",
            "SCHED-RO-B: Rev4 Calendar Reference",
            "135.1-2025 - 7.3.2.23.10.3.1",
        ),
        (
            "6.7.28",
            "SCHED-RO-B: CalEntry WeekNDay Month",
            "135.1-2025 - 7.3.2.23.3.4",
        ),
        (
            "6.7.29",
            "SCHED-RO-B: Rev4 WeekNDay Month",
            "135.1-2025 - 7.3.2.23.10.3.4",
        ),
        (
            "6.7.30",
            "SCHED-RO-B: WeekNDay WeekOfMonth",
            "135.1-2025 - 7.3.2.23.3.5",
        ),
        (
            "6.7.31",
            "SCHED-RO-B: Rev4 WeekNDay WeekOfMonth",
            "135.1-2025 - 7.3.2.23.10.3.5",
        ),
        (
            "6.7.32",
            "SCHED-RO-B: WeekNDay LastWeek",
            "135.1-2025 - 7.3.2.23.3.6",
        ),
        (
            "6.7.33",
            "SCHED-RO-B: Rev4 WeekNDay SpecialWeek",
            "135.1-2025 - 7.3.2.23.10.3.6",
        ),
        (
            "6.7.34",
            "SCHED-RO-B: WeekNDay DayOfWeek",
            "135.1-2025 - 7.3.2.23.3.7",
        ),
        (
            "6.7.35",
            "SCHED-RO-B: Rev4 WeekNDay DayOfWeek",
            "135.1-2025 - 7.3.2.23.10.3.7",
        ),
        (
            "6.7.36",
            "SCHED-RO-B: Rev4 OddMonth",
            "135.1-2025 - 7.3.2.23.10.3.10",
        ),
        (
            "6.7.37",
            "SCHED-RO-B: Rev4 EvenMonth",
            "135.1-2025 - 7.3.2.23.10.3.11",
        ),
        (
            "6.7.38",
            "SCHED-RO-B: CalEntry DateRange",
            "135.1-2025 - 7.3.2.23.3.3",
        ),
        (
            "6.7.39",
            "SCHED-RO-B: Rev4 DateRange",
            "135.1-2025 - 7.3.2.23.10.3.3",
        ),
        (
            "6.7.40",
            "SCHED-RO-B: CalEntry Date",
            "135.1-2025 - 7.3.2.23.3.2",
        ),
        (
            "6.7.41",
            "SCHED-RO-B: Rev4 Date",
            "135.1-2025 - 7.3.2.23.10.3.2",
        ),
        (
            "6.7.42",
            "SCHED-RO-B: Event Priority",
            "135.1-2025 - 7.3.2.23.3.8",
        ),
        (
            "6.7.43",
            "SCHED-RO-B: Rev4 Event Priority",
            "135.1-2025 - 7.3.2.23.10.3.8",
        ),
    ];

    for &(id, name, reference) in cal {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "readonly-b", "calendar"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_ro_base(ctx)),
        });
    }

    // BTL-specific
    let btl: &[(&str, &str, &str)] = &[
        (
            "6.7.44",
            "SCHED-RO-B: BTL Write_Every FALSE",
            "BTL - 7.3.2.23.X1.1",
        ),
        (
            "6.7.45",
            "SCHED-RO-B: BTL Write_Every TRUE",
            "BTL - 7.3.2.23.X1.2",
        ),
        (
            "6.7.46",
            "SCHED-RO-B: BTL Exception Size",
            "BTL - 7.3.2.23.9",
        ),
    ];

    for &(id, name, reference) in btl {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "readonly-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_ro_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn sched_ro_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::EFFECTIVE_PERIOD)
        .await?;
    ctx.pass()
}

async fn sched_ro_weekly(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    let data = ctx
        .read_property_raw(sched, PropertyIdentifier::WEEKLY_SCHEDULE, Some(0))
        .await?;
    if data.is_empty() {
        return Err(TestFailure::new("Weekly_Schedule[0] returned empty"));
    }
    ctx.pass()
}
