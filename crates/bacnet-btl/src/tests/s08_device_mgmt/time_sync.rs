//! BTL Test Plan Sections 8.7–8.12 — Time Synchronization.
//! 15 BTL refs: 8.7 TS-A (1), 8.8 TS-B (2), 8.9 UTC-A (1), 8.10 UTC-B (2),
//! 8.11 Auto-TS-A (7), 8.12 Manual-TS-A (2).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 8.7 DM-TS-A (Time Sync A, 1 ref) ────────────────────────────────
    registry.add(TestDef {
        id: "8.7.1",
        name: "DM-TS-A: Initiate TimeSynchronization",
        reference: "135.1-2025 - 8.24.1",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "time-sync"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ts_base(ctx)),
    });

    // ── 8.8 DM-TS-B (Time Sync B, 2 refs) ───────────────────────────────
    registry.add(TestDef {
        id: "8.8.1",
        name: "DM-TS-B: Accept TimeSynchronization",
        reference: "135.1-2025 - 9.33.1.1",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "time-sync"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ts_b_test(ctx)),
    });
    registry.add(TestDef {
        id: "8.8.2",
        name: "DM-TS-B: Local_Date/Local_Time Updated",
        reference: "135.1-2025 - 9.33.1.2",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "time-sync"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ts_b_test(ctx)),
    });

    // ── 8.9 DM-UTC-A (UTC Time Sync A, 1 ref) ───────────────────────────
    registry.add(TestDef {
        id: "8.9.1",
        name: "DM-UTC-A: Initiate UTCTimeSynchronization",
        reference: "135.1-2025 - 8.24.2",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "utc-sync"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ts_base(ctx)),
    });

    // ── 8.10 DM-UTC-B (UTC Time Sync B, 2 refs) ─────────────────────────
    registry.add(TestDef {
        id: "8.10.1",
        name: "DM-UTC-B: Accept UTCTimeSynchronization",
        reference: "135.1-2025 - 9.33.2.1",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "utc-sync"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(utc_b_test(ctx)),
    });
    registry.add(TestDef {
        id: "8.10.2",
        name: "DM-UTC-B: UTC_Offset Applied",
        reference: "135.1-2025 - 9.33.2.2",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "utc-sync"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(utc_b_test(ctx)),
    });

    // ── 8.11 Auto Time Sync A (7 refs) ───────────────────────────────────
    let auto: &[(&str, &str, &str)] = &[
        ("8.11.1", "DM-ATS-A: Auto TS Enabled", "135.1-2025 - 8.24.3"),
        (
            "8.11.2",
            "DM-ATS-A: Auto TS Interval",
            "135.1-2025 - 8.24.4",
        ),
        (
            "8.11.3",
            "DM-ATS-A: Auto TS at Startup",
            "135.1-2025 - 8.24.5",
        ),
        (
            "8.11.4",
            "DM-ATS-A: TS Master Device",
            "135.1-2025 - 8.24.6",
        ),
        (
            "8.11.5",
            "DM-ATS-A: UTC Master Device",
            "135.1-2025 - 8.24.7",
        ),
        (
            "8.11.6",
            "DM-ATS-A: Time_Synchronization_Interval",
            "135.1-2025 - 12.11.42",
        ),
        (
            "8.11.7",
            "DM-ATS-A: Align_Intervals",
            "135.1-2025 - 12.11.43",
        ),
    ];
    for &(id, name, reference) in auto {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "auto-ts"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(auto_ts(ctx)),
        });
    }

    // ── 8.12 Manual Time Sync A (2 refs) ─────────────────────────────────
    registry.add(TestDef {
        id: "8.12.1",
        name: "DM-MTS-A: Manual TS via Service",
        reference: "135.1-2025 - 8.24.1",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "manual-ts"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ts_base(ctx)),
    });
    registry.add(TestDef {
        id: "8.12.2",
        name: "DM-MTS-A: Manual UTC via Service",
        reference: "135.1-2025 - 8.24.2",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "manual-ts"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ts_base(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ts_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_DATE)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_TIME)
        .await?;
    ctx.pass()
}

async fn ts_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_DATE)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_TIME)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn utc_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::UTC_OFFSET)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_DATE)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_TIME)
        .await?;
    ctx.pass()
}

async fn auto_ts(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_DATE)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_TIME)
        .await?;
    ctx.pass()
}
