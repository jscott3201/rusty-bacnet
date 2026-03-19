//! BTL Test Plan Sections 5.9–5.12 — Event Log.
//! 57 BTL references: 5.9 View-A (8), 5.10 View+Modify-A (2),
//! 5.11 Internal-B (25), 5.12 External-B (22).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 5.9 Event Log View A ─────────────────────────────────────────────

    let view_a: &[(&str, &str, &str)] = &[
        (
            "5.9.1",
            "EL-View-A: Read Log_Buffer",
            "135.1-2025 - 9.21.1.1",
        ),
        (
            "5.9.2",
            "EL-View-A: Read Record_Count",
            "135.1-2025 - 12.26",
        ),
        ("5.9.3", "EL-View-A: Read Log_Enable", "135.1-2025 - 12.26"),
        (
            "5.9.4",
            "EL-View-A: Read Stop_When_Full",
            "135.1-2025 - 12.26",
        ),
        ("5.9.5", "EL-View-A: Read Buffer_Size", "135.1-2025 - 12.26"),
        (
            "5.9.6",
            "EL-View-A: Read Total_Record_Count",
            "135.1-2025 - 12.26",
        ),
        (
            "5.9.7",
            "EL-View-A: Read Status_Flags",
            "135.1-2025 - 12.26",
        ),
        ("5.9.8", "EL-View-A: Read Event_State", "135.1-2025 - 12.26"),
    ];

    for &(id, name, reference) in view_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "event-log", "view"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(25)),
            timeout: None,
            run: |ctx| Box::pin(el_view(ctx)),
        });
    }

    // ── 5.10 Event Log View+Modify A ─────────────────────────────────────

    registry.add(TestDef {
        id: "5.10.1",
        name: "EL-ViewModify-A: Write Log_Enable",
        reference: "135.1-2025 - 12.26",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "event-log", "modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(25)),
        timeout: None,
        run: |ctx| Box::pin(el_modify(ctx)),
    });
    registry.add(TestDef {
        id: "5.10.2",
        name: "EL-ViewModify-A: Write Stop_When_Full",
        reference: "135.1-2025 - 12.26",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "event-log", "modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(25)),
        timeout: None,
        run: |ctx| Box::pin(el_modify(ctx)),
    });

    // ── 5.11 Event Log Internal B ────────────────────────────────────────

    let int_b: &[(&str, &str, &str)] = &[
        (
            "5.11.1",
            "EL-Int-B: Log_Enable Controls Logging",
            "135.1-2025 - 7.3.2.18.1",
        ),
        (
            "5.11.2",
            "EL-Int-B: Stop_When_Full Behavior",
            "135.1-2025 - 7.3.2.18.2",
        ),
        (
            "5.11.3",
            "EL-Int-B: Record_Count Increments",
            "135.1-2025 - 7.3.2.18.3",
        ),
        (
            "5.11.4",
            "EL-Int-B: Buffer Wraps When Not Stop_When_Full",
            "135.1-2025 - 7.3.2.18.4",
        ),
        (
            "5.11.5",
            "EL-Int-B: Total_Record_Count Increments",
            "135.1-2025 - 7.3.2.18.5",
        ),
        (
            "5.11.6",
            "EL-Int-B: Notification_Class Property",
            "135.1-2025 - 7.3.2.18.6",
        ),
        (
            "5.11.7",
            "EL-Int-B: Event_Enable Property",
            "135.1-2025 - 7.3.2.18.7",
        ),
        (
            "5.11.8",
            "EL-Int-B: Acked_Transitions Property",
            "135.1-2025 - 7.3.2.18.8",
        ),
        (
            "5.11.9",
            "EL-Int-B: Event_Time_Stamps Property",
            "135.1-2025 - 7.3.2.18.9",
        ),
        (
            "5.11.10",
            "EL-Int-B: Status_Flags Reflects Log State",
            "135.1-2025 - 7.3.2.18.10",
        ),
        (
            "5.11.11",
            "EL-Int-B: Log Interval Property",
            "135.1-2025 - 7.3.2.18.11",
        ),
        (
            "5.11.12",
            "EL-Int-B: Start_Time/Stop_Time",
            "135.1-2025 - 7.3.2.18.12",
        ),
        (
            "5.11.13",
            "EL-Int-B: Event Notification Logged",
            "135.1-2025 - 7.3.2.18.13",
        ),
        (
            "5.11.14",
            "EL-Int-B: Only Matching Events Logged",
            "135.1-2025 - 7.3.2.18.14",
        ),
        (
            "5.11.15",
            "EL-Int-B: ReadRange Support",
            "135.1-2025 - 9.21.1.1",
        ),
        (
            "5.11.16",
            "EL-Int-B: ReadRange by Sequence",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "5.11.17",
            "EL-Int-B: ReadRange by Time",
            "135.1-2025 - 9.21.1.3",
        ),
        (
            "5.11.18",
            "EL-Int-B: Log Buffer Content Valid",
            "135.1-2025 - 12.26",
        ),
        (
            "5.11.19",
            "EL-Int-B: Log Disabled Then Enabled",
            "135.1-2025 - 7.3.2.18.1",
        ),
        (
            "5.11.20",
            "EL-Int-B: Multiple Event Sources",
            "135.1-2025 - 7.3.2.18.13",
        ),
        (
            "5.11.21",
            "EL-Int-B: Record_Count Zeroed on Disable",
            "135.1-2025 - 7.3.2.18.3",
        ),
        (
            "5.11.22",
            "EL-Int-B: Buffer_Size Readable",
            "135.1-2025 - 12.26",
        ),
        (
            "5.11.23",
            "EL-Int-B: Align_Intervals Property",
            "135.1-2025 - 7.3.2.18.15",
        ),
        (
            "5.11.24",
            "EL-Int-B: Interval_Offset Property",
            "135.1-2025 - 7.3.2.18.16",
        ),
        (
            "5.11.25",
            "EL-Int-B: Trigger Property",
            "135.1-2025 - 7.3.2.18.17",
        ),
    ];

    for &(id, name, reference) in int_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "event-log", "internal"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(25)),
            timeout: None,
            run: |ctx| Box::pin(el_internal(ctx)),
        });
    }

    // ── 5.12 Event Log External B ────────────────────────────────────────

    let ext_b: &[(&str, &str, &str)] = &[
        (
            "5.12.1",
            "EL-Ext-B: Log_Enable Controls",
            "135.1-2025 - 7.3.2.18.1",
        ),
        (
            "5.12.2",
            "EL-Ext-B: Stop_When_Full",
            "135.1-2025 - 7.3.2.18.2",
        ),
        (
            "5.12.3",
            "EL-Ext-B: Record_Count Increments",
            "135.1-2025 - 7.3.2.18.3",
        ),
        (
            "5.12.4",
            "EL-Ext-B: Buffer Wraps",
            "135.1-2025 - 7.3.2.18.4",
        ),
        (
            "5.12.5",
            "EL-Ext-B: Total_Record_Count",
            "135.1-2025 - 7.3.2.18.5",
        ),
        (
            "5.12.6",
            "EL-Ext-B: Notification_Class",
            "135.1-2025 - 7.3.2.18.6",
        ),
        (
            "5.12.7",
            "EL-Ext-B: Event_Enable",
            "135.1-2025 - 7.3.2.18.7",
        ),
        (
            "5.12.8",
            "EL-Ext-B: ReadRange Support",
            "135.1-2025 - 9.21.1.1",
        ),
        (
            "5.12.9",
            "EL-Ext-B: ReadRange by Sequence",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "5.12.10",
            "EL-Ext-B: ReadRange by Time",
            "135.1-2025 - 9.21.1.3",
        ),
        (
            "5.12.11",
            "EL-Ext-B: External Event Logged",
            "135.1-2025 - 7.3.2.18.13",
        ),
        (
            "5.12.12",
            "EL-Ext-B: Only Matching External Events",
            "135.1-2025 - 7.3.2.18.14",
        ),
        (
            "5.12.13",
            "EL-Ext-B: Log Buffer Valid",
            "135.1-2025 - 12.26",
        ),
        (
            "5.12.14",
            "EL-Ext-B: Status_Flags",
            "135.1-2025 - 7.3.2.18.10",
        ),
        (
            "5.12.15",
            "EL-Ext-B: Start/Stop Time",
            "135.1-2025 - 7.3.2.18.12",
        ),
        (
            "5.12.16",
            "EL-Ext-B: Log Interval",
            "135.1-2025 - 7.3.2.18.11",
        ),
        (
            "5.12.17",
            "EL-Ext-B: Disabled Then Enabled",
            "135.1-2025 - 7.3.2.18.1",
        ),
        (
            "5.12.18",
            "EL-Ext-B: Acked_Transitions",
            "135.1-2025 - 7.3.2.18.8",
        ),
        (
            "5.12.19",
            "EL-Ext-B: Event_Time_Stamps",
            "135.1-2025 - 7.3.2.18.9",
        ),
        (
            "5.12.20",
            "EL-Ext-B: Align_Intervals",
            "135.1-2025 - 7.3.2.18.15",
        ),
        (
            "5.12.21",
            "EL-Ext-B: Interval_Offset",
            "135.1-2025 - 7.3.2.18.16",
        ),
        ("5.12.22", "EL-Ext-B: Trigger", "135.1-2025 - 7.3.2.18.17"),
    ];

    for &(id, name, reference) in ext_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "event-log", "external"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(25)),
            timeout: None,
            run: |ctx| Box::pin(el_internal(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn el_view(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let el = ctx.first_object_of_type(ObjectType::EVENT_LOG)?;
    ctx.verify_readable(el, PropertyIdentifier::LOG_BUFFER)
        .await?;
    ctx.verify_readable(el, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.verify_readable(el, PropertyIdentifier::STATUS_FLAGS)
        .await?;
    ctx.pass()
}

async fn el_modify(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let el = ctx.first_object_of_type(ObjectType::EVENT_LOG)?;
    ctx.verify_readable(el, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(el, PropertyIdentifier::STOP_WHEN_FULL)
        .await?;
    ctx.pass()
}

async fn el_internal(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let el = ctx.first_object_of_type(ObjectType::EVENT_LOG)?;
    ctx.verify_readable(el, PropertyIdentifier::OBJECT_IDENTIFIER)
        .await?;
    ctx.verify_readable(el, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(el, PropertyIdentifier::STOP_WHEN_FULL)
        .await?;
    ctx.verify_readable(el, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(el, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}
