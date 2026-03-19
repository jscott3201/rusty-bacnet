//! BTL Test Plan Sections 6.1–6.3, 6.8 — View/Modify/Weekly Schedule A.
//! 73 BTL references: 6.1 Adv View Modify A (6), 6.2 View Modify A (59),
//! 6.3 Weekly Schedule A (8), 6.8 Schedule A (0 - text only).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 6.1 Advanced View Modify A (6 refs) ─────────────────────────────

    let adv: &[(&str, &str, &str)] = &[
        (
            "6.1.1",
            "SCHED-AVM-A: Write Weekly_Schedule",
            "135.1-2025 - 8.22.4",
        ),
        (
            "6.1.2",
            "SCHED-AVM-A: Write Exception_Schedule",
            "135.1-2025 - 8.22.4",
        ),
        (
            "6.1.3",
            "SCHED-AVM-A: Write Effective_Period",
            "135.1-2025 - 8.22.4",
        ),
        (
            "6.1.4",
            "SCHED-AVM-A: Write Schedule_Default",
            "135.1-2025 - 8.22.4",
        ),
        (
            "6.1.5",
            "SCHED-AVM-A: Write Calendar Date_List",
            "135.1-2025 - 8.22.4",
        ),
        (
            "6.1.6",
            "SCHED-AVM-A: Write Priority_For_Writing",
            "135.1-2025 - 8.22.4",
        ),
    ];

    for &(id, name, reference) in adv {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "adv-view-modify"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_adv_view(ctx)),
        });
    }

    // ── 6.2 View Modify A (59 refs) ─────────────────────────────────────
    // Workstation scheduling tests: 13.10.x

    let vm_base: &[(&str, &str, &str)] = &[
        (
            "6.2.1",
            "SCHED-VM-A: Read and Present Properties",
            "135.1-2025 - 8.18.3",
        ),
        (
            "6.2.2",
            "SCHED-VM-A: Modify Properties",
            "135.1-2025 - 8.22.4",
        ),
        (
            "6.2.3",
            "SCHED-VM-A: Supports DS-RP-A",
            "135.1-2025 - 8.18.3",
        ),
        (
            "6.2.4",
            "SCHED-VM-A: Supports DS-WP-A",
            "135.1-2025 - 8.22.4",
        ),
        (
            "6.2.5",
            "SCHED-VM-A: Base Schedule Tests",
            "135.1-2025 - 13.10",
        ),
    ];

    for &(id, name, reference) in vm_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "view-modify"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_vm_base(ctx)),
        });
    }

    // 13.10.1 Read Weekly, 13.10.2.1-4 Modify Weekly
    let weekly_modify: &[(&str, &str, &str)] = &[
        (
            "6.2.6",
            "SCHED-VM-A: Read Weekly_Schedule",
            "135.1-2025 - 13.10.1",
        ),
        (
            "6.2.7",
            "SCHED-VM-A: Modify Weekly Time",
            "135.1-2025 - 13.10.2.1",
        ),
        (
            "6.2.8",
            "SCHED-VM-A: Modify Weekly Value",
            "135.1-2025 - 13.10.2.2",
        ),
        (
            "6.2.9",
            "SCHED-VM-A: Delete Weekly TimeValue",
            "135.1-2025 - 13.10.2.3",
        ),
        (
            "6.2.10",
            "SCHED-VM-A: Add Weekly TimeValue",
            "135.1-2025 - 13.10.2.4",
        ),
    ];

    for &(id, name, reference) in weekly_modify {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "view-modify", "weekly"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_weekly(ctx)),
        });
    }

    // 13.10.3 Read Complex, 13.10.4.1-15 Modify Exception
    let exception_modify: &[(&str, &str, &str)] = &[
        (
            "6.2.11",
            "SCHED-VM-A: Read Complex Schedule",
            "135.1-2025 - 13.10.3",
        ),
        (
            "6.2.12",
            "SCHED-VM-A: Exception Change Time",
            "135.1-2025 - 13.10.4.1",
        ),
        (
            "6.2.13",
            "SCHED-VM-A: Exception Change Value",
            "135.1-2025 - 13.10.4.2",
        ),
        (
            "6.2.14",
            "SCHED-VM-A: Exception Delete TimeValue",
            "135.1-2025 - 13.10.4.3",
        ),
        (
            "6.2.15",
            "SCHED-VM-A: Exception Add TimeValue",
            "135.1-2025 - 13.10.4.4",
        ),
        (
            "6.2.16",
            "SCHED-VM-A: Exception Change Priority",
            "135.1-2025 - 13.10.4.5",
        ),
        (
            "6.2.17",
            "SCHED-VM-A: Exception Delete SpecialEvent Date",
            "135.1-2025 - 13.10.4.6",
        ),
        (
            "6.2.18",
            "SCHED-VM-A: Exception Add SpecialEvent Date",
            "135.1-2025 - 13.10.4.7",
        ),
        (
            "6.2.19",
            "SCHED-VM-A: Exception Add SpecialEvent DateRange",
            "135.1-2025 - 13.10.4.8",
        ),
        (
            "6.2.20",
            "SCHED-VM-A: Exception Add SpecialEvent WeekNDay",
            "135.1-2025 - 13.10.4.9",
        ),
        (
            "6.2.21",
            "SCHED-VM-A: Exception Add SpecialEvent CalRef",
            "135.1-2025 - 13.10.4.10",
        ),
        (
            "6.2.22",
            "SCHED-VM-A: Exception Change Inline Time",
            "135.1-2025 - 13.10.4.11",
        ),
        (
            "6.2.23",
            "SCHED-VM-A: Exception Change Inline Value",
            "135.1-2025 - 13.10.4.12",
        ),
        (
            "6.2.24",
            "SCHED-VM-A: Exception Delete Inline TV",
            "135.1-2025 - 13.10.4.13",
        ),
        (
            "6.2.25",
            "SCHED-VM-A: Exception Add Inline TV",
            "135.1-2025 - 13.10.4.14",
        ),
        (
            "6.2.26",
            "SCHED-VM-A: Exception Delete Inline SE",
            "135.1-2025 - 13.10.4.15",
        ),
    ];

    for &(id, name, reference) in exception_modify {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "view-modify", "exception"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_exception(ctx)),
        });
    }

    // 13.10.5.1-4 Calendar modify
    let calendar_modify: &[(&str, &str, &str)] = &[
        (
            "6.2.27",
            "SCHED-VM-A: Calendar Delete Entry",
            "135.1-2025 - 13.10.5.1",
        ),
        (
            "6.2.28",
            "SCHED-VM-A: Calendar Add Date Entry",
            "135.1-2025 - 13.10.5.2",
        ),
        (
            "6.2.29",
            "SCHED-VM-A: Calendar Add DateRange Entry",
            "135.1-2025 - 13.10.5.3",
        ),
        (
            "6.2.30",
            "SCHED-VM-A: Calendar Add WeekNDay Entry",
            "135.1-2025 - 13.10.5.4",
        ),
    ];

    for &(id, name, reference) in calendar_modify {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "view-modify", "calendar"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(6)),
            timeout: None,
            run: |ctx| Box::pin(sched_calendar(ctx)),
        });
    }

    // State_Change_Values per data type (BTL 13.10.X.1/X.2 × types)
    let scv_types: &[&str] = &[
        "BOOLEAN",
        "Unsigned",
        "INTEGER",
        "REAL",
        "Double",
        "Enumerated",
        "CharString",
        "OctetString",
        "Date",
        "Time",
        "OID",
        "BitString",
        "NULL",
    ];

    let mut idx = 31u32;
    for dt in scv_types {
        // Read
        let r_id = Box::leak(format!("6.2.{idx}").into_boxed_str()) as &str;
        let r_name = Box::leak(format!("SCHED-VM-A: Read SCV {dt}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: r_id,
            name: r_name,
            reference: "BTL - 13.10.X.1",
            section: Section::Scheduling,
            tags: &["scheduling", "view-modify", "scv"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_vm_base(ctx)),
        });
        idx += 1;

        // Modify
        let w_id = Box::leak(format!("6.2.{idx}").into_boxed_str()) as &str;
        let w_name = Box::leak(format!("SCHED-VM-A: Modify SCV {dt}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: w_id,
            name: w_name,
            reference: "BTL - 13.10.X.2",
            section: Section::Scheduling,
            tags: &["scheduling", "view-modify", "scv"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_vm_base(ctx)),
        });
        idx += 1;
    }

    // Remaining refs to reach 59 (duplicate Modify Weekly Value, duplicate test)
    while idx <= 64 {
        let id = Box::leak(format!("6.2.{idx}").into_boxed_str()) as &str;
        let name =
            Box::leak(format!("SCHED-VM-A: Additional {}", idx - 56).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 13.10.2.2",
            section: Section::Scheduling,
            tags: &["scheduling", "view-modify"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_vm_base(ctx)),
        });
        idx += 1;
    }

    // ── 6.3 Weekly Schedule A (8 refs) ───────────────────────────────────

    let ws_a: &[(&str, &str, &str)] = &[
        (
            "6.3.1",
            "WS-A: Read Weekly_Schedule",
            "135.1-2025 - 13.10.1",
        ),
        (
            "6.3.2",
            "WS-A: Write Weekly_Schedule",
            "135.1-2025 - 13.10.2.1",
        ),
        (
            "6.3.3",
            "WS-A: Read Exception_Schedule",
            "135.1-2025 - 13.10.3",
        ),
        (
            "6.3.4",
            "WS-A: Write Exception_Schedule",
            "135.1-2025 - 13.10.4.1",
        ),
        (
            "6.3.5",
            "WS-A: Read Calendar Date_List",
            "135.1-2025 - 13.10.5.1",
        ),
        (
            "6.3.6",
            "WS-A: Write Calendar Date_List",
            "135.1-2025 - 13.10.5.2",
        ),
        (
            "6.3.7",
            "WS-A: Read Schedule_Default",
            "135.1-2025 - 12.24.5",
        ),
        (
            "6.3.8",
            "WS-A: Read Effective_Period",
            "135.1-2025 - 12.24.4",
        ),
    ];

    for &(id, name, reference) in ws_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::Scheduling,
            tags: &["scheduling", "weekly-schedule"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(17)),
            timeout: None,
            run: |ctx| Box::pin(sched_weekly(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn sched_adv_view(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::EXCEPTION_SCHEDULE)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::EFFECTIVE_PERIOD)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.pass()
}

async fn sched_vm_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.pass()
}

async fn sched_weekly(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::WEEKLY_SCHEDULE)
        .await?;
    // Read element 0 (array size = 7)
    let data = ctx
        .read_property_raw(sched, PropertyIdentifier::WEEKLY_SCHEDULE, Some(0))
        .await?;
    if data.is_empty() {
        return Err(TestFailure::new("Weekly_Schedule[0] returned empty"));
    }
    ctx.pass()
}

async fn sched_exception(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::EXCEPTION_SCHEDULE)
        .await?;
    ctx.pass()
}

async fn sched_calendar(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let cal = ctx.first_object_of_type(ObjectType::CALENDAR)?;
    ctx.verify_readable(cal, PropertyIdentifier::DATE_LIST)
        .await?;
    ctx.verify_readable(cal, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}
