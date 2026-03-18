//! BTL Test Plan Sections 13.2–13.4 — Audit Reporter B + Simple B + Forwarder B.
//! 49 BTL refs: 13.2 Reporter B (24), 13.3 Reporter Simple B (24),
//! 13.4 Forwarder B (1).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 13.2 AR-RPT-B (Audit Reporter B, 24 refs) ───────────────────────

    let rpt: &[(&str, &str, &str)] = &[
        (
            "13.2.1",
            "AR-RPT-B: Notification_Recipient",
            "135.1-2025 - 7.3.1.30.1",
        ),
        (
            "13.2.2",
            "AR-RPT-B: Audit_Level NONE",
            "135.1-2025 - 7.3.1.29.1",
        ),
        (
            "13.2.3",
            "AR-RPT-B: Audit_Level Test",
            "135.1-2025 - 7.3.1.29.2",
        ),
        (
            "13.2.4",
            "AR-RPT-B: Audit_Level Change Notification",
            "135.1-2025 - 7.3.1.29.3",
        ),
        (
            "13.2.5",
            "AR-RPT-B: Monitored_Objects",
            "135.1-2025 - 7.3.1.34.1",
        ),
        (
            "13.2.6",
            "AR-RPT-B: Target Basic Notification",
            "135.1-2025 - 7.3.2.49.1",
        ),
        (
            "13.2.7",
            "AR-RPT-B: Target Unconfirmed Op",
            "135.1-2025 - 7.3.2.49.2",
        ),
        (
            "13.2.8",
            "AR-RPT-B: Target Confirmed Op",
            "135.1-2025 - 7.3.2.49.3",
        ),
        (
            "13.2.9",
            "AR-RPT-B: Target Priority",
            "135.1-2025 - 7.3.2.49.4",
        ),
        (
            "13.2.10",
            "AR-RPT-B: Priority_Filter Target",
            "135.1-2025 - 7.3.1.31.1",
        ),
        (
            "13.2.11",
            "AR-RPT-B: Target/Current Value",
            "135.1-2025 - 7.3.2.49.5",
        ),
        (
            "13.2.12",
            "AR-RPT-B: Target Error Notification",
            "135.1-2025 - 7.3.2.49.6",
        ),
        (
            "13.2.13",
            "AR-RPT-B: Target GENERAL Op",
            "135.1-2025 - 7.3.2.49.7",
        ),
        (
            "13.2.14",
            "AR-RPT-B: Auditable_Operations Target",
            "135.1-2025 - 7.3.1.32.2",
        ),
        (
            "13.2.15",
            "AR-RPT-B: Source Basic Notification",
            "135.1-2025 - 7.3.2.49.8",
        ),
        (
            "13.2.16",
            "AR-RPT-B: Source Same Device",
            "135.1-2025 - 7.3.2.49.9",
        ),
        (
            "13.2.17",
            "AR-RPT-B: Source Unconfirmed Op",
            "135.1-2025 - 7.3.2.49.10",
        ),
        (
            "13.2.18",
            "AR-RPT-B: Source Confirmed Op",
            "135.1-2025 - 7.3.2.49.11",
        ),
        (
            "13.2.19",
            "AR-RPT-B: Source Priority",
            "135.1-2025 - 7.3.2.49.12",
        ),
        (
            "13.2.20",
            "AR-RPT-B: Source Error Notification",
            "135.1-2025 - 7.3.2.49.13",
        ),
        (
            "13.2.21",
            "AR-RPT-B: Source Single Reporter",
            "135.1-2025 - 7.3.2.49.14",
        ),
        (
            "13.2.22",
            "AR-RPT-B: Auditable_Operations Source",
            "135.1-2025 - 7.3.1.32.3",
        ),
        (
            "13.2.23",
            "AR-RPT-B: Delay Notifications",
            "135.1-2025 - 7.3.2.49.15",
        ),
        (
            "13.2.24",
            "AR-RPT-B: TS Recipients",
            "135.1-2025 - 7.3.2.49.16",
        ),
    ];

    for &(id, name, reference) in rpt {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AuditReporting,
            tags: &["audit", "reporter"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
            timeout: None,
            run: |ctx| Box::pin(reporter_base(ctx)),
        });
    }

    // ── 13.3 AR-RPT-Simple-B (24 refs — same structure as 13.2) ─────────

    let simple: &[(&str, &str, &str)] = &[
        (
            "13.3.1",
            "AR-RPTS-B: Notification_Recipient",
            "135.1-2025 - 7.3.1.30.1",
        ),
        (
            "13.3.2",
            "AR-RPTS-B: Audit_Level NONE",
            "135.1-2025 - 7.3.1.29.1",
        ),
        (
            "13.3.3",
            "AR-RPTS-B: Audit_Level Test",
            "135.1-2025 - 7.3.1.29.2",
        ),
        (
            "13.3.4",
            "AR-RPTS-B: Audit_Level Change",
            "135.1-2025 - 7.3.1.29.3",
        ),
        (
            "13.3.5",
            "AR-RPTS-B: Monitored_Objects",
            "135.1-2025 - 7.3.1.34.1",
        ),
        (
            "13.3.6",
            "AR-RPTS-B: Target Basic",
            "135.1-2025 - 7.3.2.49.1",
        ),
        (
            "13.3.7",
            "AR-RPTS-B: Target Unconfirmed",
            "135.1-2025 - 7.3.2.49.2",
        ),
        (
            "13.3.8",
            "AR-RPTS-B: Target Confirmed",
            "135.1-2025 - 7.3.2.49.3",
        ),
        (
            "13.3.9",
            "AR-RPTS-B: Target Priority",
            "135.1-2025 - 7.3.2.49.4",
        ),
        (
            "13.3.10",
            "AR-RPTS-B: Priority_Filter",
            "135.1-2025 - 7.3.1.31.1",
        ),
        (
            "13.3.11",
            "AR-RPTS-B: Target/Current Value",
            "135.1-2025 - 7.3.2.49.5",
        ),
        (
            "13.3.12",
            "AR-RPTS-B: Target Error",
            "135.1-2025 - 7.3.2.49.6",
        ),
        (
            "13.3.13",
            "AR-RPTS-B: Target GENERAL",
            "135.1-2025 - 7.3.2.49.7",
        ),
        (
            "13.3.14",
            "AR-RPTS-B: Auditable_Ops Target",
            "135.1-2025 - 7.3.1.32.2",
        ),
        (
            "13.3.15",
            "AR-RPTS-B: Source Basic",
            "135.1-2025 - 7.3.2.49.8",
        ),
        (
            "13.3.16",
            "AR-RPTS-B: Source Same Device",
            "135.1-2025 - 7.3.2.49.9",
        ),
        (
            "13.3.17",
            "AR-RPTS-B: Source Unconfirmed",
            "135.1-2025 - 7.3.2.49.10",
        ),
        (
            "13.3.18",
            "AR-RPTS-B: Source Confirmed",
            "135.1-2025 - 7.3.2.49.11",
        ),
        (
            "13.3.19",
            "AR-RPTS-B: Source Priority",
            "135.1-2025 - 7.3.2.49.12",
        ),
        (
            "13.3.20",
            "AR-RPTS-B: Source Error",
            "135.1-2025 - 7.3.2.49.13",
        ),
        (
            "13.3.21",
            "AR-RPTS-B: Source Single Reporter",
            "135.1-2025 - 7.3.2.49.14",
        ),
        (
            "13.3.22",
            "AR-RPTS-B: Auditable_Ops Source",
            "135.1-2025 - 7.3.1.32.3",
        ),
        (
            "13.3.23",
            "AR-RPTS-B: Delay Notifications",
            "135.1-2025 - 7.3.2.49.15",
        ),
        (
            "13.3.24",
            "AR-RPTS-B: TS Recipients",
            "135.1-2025 - 7.3.2.49.16",
        ),
    ];

    for &(id, name, reference) in simple {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AuditReporting,
            tags: &["audit", "reporter-simple"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
            timeout: None,
            run: |ctx| Box::pin(reporter_base(ctx)),
        });
    }

    // ── 13.4 AR-FWD-B (Audit Forwarder, 1 ref) ──────────────────────────

    registry.add(TestDef {
        id: "13.4.1",
        name: "AR-FWD-B: Forward Audit Notifications",
        reference: "135.1-2025 - 7.3.2.48.7",
        section: Section::AuditReporting,
        tags: &["audit", "forwarder"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(61)),
        timeout: None,
        run: |ctx| Box::pin(reporter_base(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn reporter_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ar = ctx.first_object_of_type(ObjectType::AUDIT_REPORTER)?;
    ctx.verify_readable(ar, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.verify_readable(ar, PropertyIdentifier::STATUS_FLAGS)
        .await?;
    ctx.pass()
}
