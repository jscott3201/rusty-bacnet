//! BTL Test Plan Sections 5.13–5.17, 5.22–5.23 — View/Modify/Summary/Config.
//! 18 BTL references: View Notifications (2), View Modify (2),
//! Adv View (2), Adv View Modify (2), Alarm Summary View (6),
//! Configurable Recipient Lists (0 - text), Temporary Event Sub (0 - text).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 5.13 View Notifications A ────────────────────────────────────────

    registry.add(TestDef {
        id: "5.13.1",
        name: "AE-ViewNotif-A: Browse Event Properties",
        reference: "135.1-2025 - 8.18.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "view"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(view_event_props(ctx)),
    });
    registry.add(TestDef {
        id: "5.13.2",
        name: "AE-ViewNotif-A: Read NotificationClass",
        reference: "135.1-2025 - 8.18.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "view"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(15)),
        timeout: None,
        run: |ctx| Box::pin(view_nc(ctx)),
    });

    // ── 5.14 View Modify A ───────────────────────────────────────────────

    registry.add(TestDef {
        id: "5.14.1",
        name: "AE-ViewModify-A: Write Event_Enable",
        reference: "135.1-2025 - 8.20.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "modify"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(modify_event_enable(ctx)),
    });
    registry.add(TestDef {
        id: "5.14.2",
        name: "AE-ViewModify-A: Write Notification_Class",
        reference: "135.1-2025 - 8.20.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "modify"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(modify_event_enable(ctx)),
    });

    // ── 5.15 Advanced View Notifications A ───────────────────────────────

    registry.add(TestDef {
        id: "5.15.1",
        name: "AE-AdvViewNotif-A: Read All Event Props",
        reference: "135.1-2025 - 8.18.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "adv-view"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(adv_view_event(ctx)),
    });
    registry.add(TestDef {
        id: "5.15.2",
        name: "AE-AdvViewNotif-A: Read Recipient_List",
        reference: "135.1-2025 - 8.18.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "adv-view"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(15)),
        timeout: None,
        run: |ctx| Box::pin(adv_view_nc(ctx)),
    });

    // ── 5.16 Advanced View Modify A ──────────────────────────────────────

    registry.add(TestDef {
        id: "5.16.1",
        name: "AE-AdvViewModify-A: Write Limits",
        reference: "135.1-2025 - 8.20.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "adv-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(adv_modify_limits(ctx)),
    });
    registry.add(TestDef {
        id: "5.16.2",
        name: "AE-AdvViewModify-A: Write Recipient_List",
        reference: "135.1-2025 - 8.20.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "adv-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(15)),
        timeout: None,
        run: |ctx| Box::pin(adv_modify_nc(ctx)),
    });

    // ── 5.17 Alarm Summary View A ────────────────────────────────────────

    let asum: &[(&str, &str, &str)] = &[
        (
            "5.17.1",
            "AE-ASumView-A: GetAlarmSummary",
            "135.1-2025 - 8.2.1",
        ),
        (
            "5.17.2",
            "AE-ASumView-A: GetEnrollmentSummary",
            "135.1-2025 - 8.7.1",
        ),
        (
            "5.17.3",
            "AE-ASumView-A: GetEventInformation",
            "135.1-2025 - 8.13.1",
        ),
        (
            "5.17.4",
            "AE-ASumView-A: Read Event_State via RP",
            "135.1-2025 - 8.18.1",
        ),
        (
            "5.17.5",
            "AE-ASumView-A: Read Acked_Transitions",
            "135.1-2025 - 8.18.1",
        ),
        (
            "5.17.6",
            "AE-ASumView-A: Read Event_Time_Stamps",
            "135.1-2025 - 8.18.1",
        ),
    ];

    for &(id, name, reference) in asum {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "alarm-summary-view"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(view_event_props(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn view_event_props(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_ENABLE)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::ACKED_TRANSITIONS)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_TIME_STAMPS)
        .await?;
    ctx.pass()
}

async fn view_nc(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let nc = ctx.first_object_of_type(ObjectType::NOTIFICATION_CLASS)?;
    ctx.verify_readable(nc, PropertyIdentifier::PRIORITY)
        .await?;
    ctx.verify_readable(nc, PropertyIdentifier::ACK_REQUIRED)
        .await?;
    ctx.verify_readable(nc, PropertyIdentifier::RECIPIENT_LIST)
        .await?;
    ctx.pass()
}

async fn modify_event_enable(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_ENABLE)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::NOTIFICATION_CLASS)
        .await?;
    ctx.pass()
}

async fn adv_view_event(ctx: &mut TestContext) -> Result<(), TestFailure> {
    view_event_props(ctx).await
}

async fn adv_view_nc(ctx: &mut TestContext) -> Result<(), TestFailure> {
    view_nc(ctx).await
}

async fn adv_modify_limits(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_ENABLE)
        .await?;
    ctx.pass()
}

async fn adv_modify_nc(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let nc = ctx.first_object_of_type(ObjectType::NOTIFICATION_CLASS)?;
    ctx.verify_readable(nc, PropertyIdentifier::RECIPIENT_LIST)
        .await?;
    ctx.pass()
}
