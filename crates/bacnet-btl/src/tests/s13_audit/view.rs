//! BTL Test Plan Sections 13.5–13.6 — Audit View + Advanced View & Modify.
//! 6 BTL refs: 13.5 View A (2), 13.6 Adv View+Modify A (4).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 13.5 AR-View-A (2 refs) ─────────────────────────────────────────

    registry.add(TestDef {
        id: "13.5.1",
        name: "AR-View-A: Browse Audit Log",
        reference: "135.1-2025 - 8.18.1",
        section: Section::AuditReporting,
        tags: &["audit", "view"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(62)),
        timeout: None,
        run: |ctx| Box::pin(audit_view(ctx)),
    });
    registry.add(TestDef {
        id: "13.5.2",
        name: "AR-View-A: Browse Audit Reporter",
        reference: "135.1-2025 - 8.18.1",
        section: Section::AuditReporting,
        tags: &["audit", "view"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
        timeout: None,
        run: |ctx| Box::pin(audit_view_reporter(ctx)),
    });

    // ── 13.6 AR-AdvVM-A (4 refs) ────────────────────────────────────────

    registry.add(TestDef {
        id: "13.6.1",
        name: "AR-AdvVM-A: Write Audit Log Enable",
        reference: "135.1-2025 - 8.22.4",
        section: Section::AuditReporting,
        tags: &["audit", "adv-view-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(62)),
        timeout: None,
        run: |ctx| Box::pin(audit_adv_modify(ctx)),
    });
    registry.add(TestDef {
        id: "13.6.2",
        name: "AR-AdvVM-A: Write Audit Reporter Level",
        reference: "135.1-2025 - 8.22.4",
        section: Section::AuditReporting,
        tags: &["audit", "adv-view-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
        timeout: None,
        run: |ctx| Box::pin(audit_adv_modify_reporter(ctx)),
    });
    registry.add(TestDef {
        id: "13.6.3",
        name: "AR-AdvVM-A: Write Monitored_Objects",
        reference: "135.1-2025 - 8.22.4",
        section: Section::AuditReporting,
        tags: &["audit", "adv-view-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
        timeout: None,
        run: |ctx| Box::pin(audit_adv_modify_reporter(ctx)),
    });
    registry.add(TestDef {
        id: "13.6.4",
        name: "AR-AdvVM-A: Write Notification_Recipient",
        reference: "135.1-2025 - 8.22.4",
        section: Section::AuditReporting,
        tags: &["audit", "adv-view-modify"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
        timeout: None,
        run: |ctx| Box::pin(audit_adv_modify_reporter(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn audit_view(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let al = ctx.first_object_of_type(ObjectType::AUDIT_LOG)?;
    ctx.verify_readable(al, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(al, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(al, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.pass()
}

async fn audit_view_reporter(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ar = ctx.first_object_of_type(ObjectType::AUDIT_REPORTER)?;
    ctx.verify_readable(ar, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.verify_readable(ar, PropertyIdentifier::STATUS_FLAGS)
        .await?;
    ctx.pass()
}

async fn audit_adv_modify(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let al = ctx.first_object_of_type(ObjectType::AUDIT_LOG)?;
    ctx.verify_readable(al, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.pass()
}

async fn audit_adv_modify_reporter(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ar = ctx.first_object_of_type(ObjectType::AUDIT_REPORTER)?;
    ctx.verify_readable(ar, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}
