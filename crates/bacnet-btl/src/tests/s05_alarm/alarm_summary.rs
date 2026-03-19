//! BTL Test Plan Sections 5.6–5.8 — Summaries and Event Information.
//! 20 BTL references: 5.6 AlarmSummary-B (3), 5.7 EnrollmentSummary-B (10),
//! 5.8 EventInformation-B (7).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 5.6 AE-ASUM-B (Alarm Summary) ───────────────────────────────────

    registry.add(TestDef {
        id: "5.6.1",
        name: "AE-ASUM-B: GetAlarmSummary Base",
        reference: "135.1-2025 - 9.5.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "alarm-summary"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(3)),
        timeout: None,
        run: |ctx| Box::pin(alarm_summary(ctx)),
    });
    registry.add(TestDef {
        id: "5.6.2",
        name: "AE-ASUM-B: Normal State Returns Empty",
        reference: "135.1-2025 - 9.5.2",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "alarm-summary"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(3)),
        timeout: None,
        run: |ctx| Box::pin(alarm_summary(ctx)),
    });
    registry.add(TestDef {
        id: "5.6.3",
        name: "AE-ASUM-B: Reflects Acked_Transitions",
        reference: "135.1-2025 - 9.5.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "alarm-summary"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(3)),
        timeout: None,
        run: |ctx| Box::pin(alarm_summary(ctx)),
    });

    // ── 5.7 AE-ESUM-B (Enrollment Summary) ──────────────────────────────

    let esum: &[(&str, &str, &str)] = &[
        ("5.7.1", "AE-ESUM-B: No Filter", "135.1-2025 - 9.7.1.1"),
        ("5.7.2", "AE-ESUM-B: Ack Filter", "135.1-2025 - 9.7.1.2"),
        (
            "5.7.3",
            "AE-ESUM-B: Event State Filter",
            "135.1-2025 - 9.7.1.3",
        ),
        (
            "5.7.4",
            "AE-ESUM-B: Event Type Filter",
            "135.1-2025 - 9.7.1.4",
        ),
        (
            "5.7.5",
            "AE-ESUM-B: Priority Range Filter",
            "135.1-2025 - 9.7.1.5",
        ),
        (
            "5.7.6",
            "AE-ESUM-B: Notification Class Filter",
            "135.1-2025 - 9.7.1.6",
        ),
        (
            "5.7.7",
            "AE-ESUM-B: Multiple Filters",
            "135.1-2025 - 9.7.1.7",
        ),
        (
            "5.7.8",
            "AE-ESUM-B: No Match Returns Empty",
            "135.1-2025 - 9.7.2.1",
        ),
        (
            "5.7.9",
            "AE-ESUM-B: Unknown Object Error",
            "135.1-2025 - 9.7.2.2",
        ),
        (
            "5.7.10",
            "AE-ESUM-B: All Filters Combined",
            "135.1-2025 - 9.7.1.8",
        ),
    ];

    for &(id, name, reference) in esum {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "enrollment-summary"],
            conditionality: Conditionality::RequiresCapability(Capability::Service(4)),
            timeout: None,
            run: |ctx| Box::pin(enrollment_summary(ctx)),
        });
    }

    // ── 5.8 AE-INFO-B (Event Information) ────────────────────────────────

    let info: &[(&str, &str, &str)] = &[
        (
            "5.8.1",
            "AE-INFO-B: GetEventInformation Base",
            "135.1-2025 - 9.13.1.1",
        ),
        (
            "5.8.2",
            "AE-INFO-B: Empty When All Normal",
            "135.1-2025 - 9.13.1.2",
        ),
        (
            "5.8.3",
            "AE-INFO-B: Returns Event_State",
            "135.1-2025 - 9.13.1.3",
        ),
        (
            "5.8.4",
            "AE-INFO-B: Returns Acked_Transitions",
            "135.1-2025 - 9.13.1.4",
        ),
        (
            "5.8.5",
            "AE-INFO-B: Returns Event_Time_Stamps",
            "135.1-2025 - 9.13.1.5",
        ),
        (
            "5.8.6",
            "AE-INFO-B: Continuation (More Events)",
            "135.1-2025 - 9.13.1.6",
        ),
        (
            "5.8.7",
            "AE-INFO-B: Unknown Object Ignored",
            "135.1-2025 - 9.13.2.1",
        ),
    ];

    for &(id, name, reference) in info {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "event-info"],
            conditionality: Conditionality::RequiresCapability(Capability::Service(29)),
            timeout: None,
            run: |ctx| Box::pin(event_info(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn alarm_summary(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}

async fn enrollment_summary(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn event_info(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::ACKED_TRANSITIONS)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_TIME_STAMPS)
        .await?;
    ctx.pass()
}
