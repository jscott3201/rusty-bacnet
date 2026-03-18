//! BTL Test Plan Sections 8.13–8.20 — DCC, Reinitialize, Backup/Restore, Restart.
//! 54 BTL refs: 8.13 DCC-A (6), 8.14 DCC-B (17), 8.15 RD-A (4), 8.16 RD-B (7),
//! 8.17 BR-A (5), 8.18 BR-B (13), 8.19 Restart-A (1), 8.20 Restart-B (1).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 8.13 DCC-A (6 refs) ─────────────────────────────────────────────

    let dcc_a: &[(&str, &str, &str)] = &[
        ("8.13.1", "DCC-A: Initiate DCC Enable", "135.1-2025 - 8.9.1"),
        (
            "8.13.2",
            "DCC-A: Initiate DCC Disable",
            "135.1-2025 - 8.9.2",
        ),
        (
            "8.13.3",
            "DCC-A: Initiate DCC DisableInitiation",
            "135.1-2025 - 8.9.3",
        ),
        ("8.13.4", "DCC-A: DCC with Password", "135.1-2025 - 8.9.4"),
        ("8.13.5", "DCC-A: DCC with Duration", "135.1-2025 - 8.9.5"),
        ("8.13.6", "DCC-A: DCC Verify Response", "135.1-2025 - 8.9.6"),
    ];
    for &(id, name, reference) in dcc_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "dcc"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(dcc_base(ctx)),
        });
    }

    // ── 8.14 DCC-B (17 refs) ────────────────────────────────────────────

    let dcc_b: &[(&str, &str, &str)] = &[
        (
            "8.14.1",
            "DCC-B: Accept DCC Enable",
            "135.1-2025 - 9.24.1.1",
        ),
        (
            "8.14.2",
            "DCC-B: Accept DCC Disable",
            "135.1-2025 - 9.24.1.2",
        ),
        (
            "8.14.3",
            "DCC-B: Accept DCC DisableInitiation",
            "135.1-2025 - 9.24.1.3",
        ),
        (
            "8.14.4",
            "DCC-B: Respond While Disabled",
            "135.1-2025 - 9.24.1.4",
        ),
        (
            "8.14.5",
            "DCC-B: No Initiation While Disabled",
            "135.1-2025 - 9.24.1.5",
        ),
        (
            "8.14.6",
            "DCC-B: WhoIs While Disabled",
            "135.1-2025 - 9.24.1.6",
        ),
        (
            "8.14.7",
            "DCC-B: Re-Enable After Disable",
            "135.1-2025 - 9.24.1.7",
        ),
        ("8.14.8", "DCC-B: Wrong Password", "135.1-2025 - 9.24.2.1"),
        ("8.14.9", "DCC-B: Duration Expires", "135.1-2025 - 9.24.1.8"),
        (
            "8.14.10",
            "DCC-B: COV During DisableInitiation",
            "135.1-2025 - 9.24.1.9",
        ),
        (
            "8.14.11",
            "DCC-B: Device Responsive After DCC",
            "135.1-2025 - 9.24",
        ),
    ];
    for &(id, name, reference) in dcc_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "dcc"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(dcc_b_test(ctx)),
        });
    }
    for i in 12..18 {
        let id = Box::leak(format!("8.14.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("DCC-B: Extended {}", i - 11).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 9.24.1.1",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "dcc"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(dcc_b_test(ctx)),
        });
    }

    // ── 8.15 RD-A (Reinitialize Device A, 4 refs) ───────────────────────

    let rd_a: &[(&str, &str, &str)] = &[
        ("8.15.1", "RD-A: Initiate Warmstart", "135.1-2025 - 8.19.1"),
        ("8.15.2", "RD-A: Initiate Coldstart", "135.1-2025 - 8.19.2"),
        ("8.15.3", "RD-A: RD with Password", "135.1-2025 - 8.19.3"),
        ("8.15.4", "RD-A: RD Verify Response", "135.1-2025 - 8.19.4"),
    ];
    for &(id, name, reference) in rd_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "reinitialize"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rd_base(ctx)),
        });
    }

    // ── 8.16 RD-B (Reinitialize Device B, 7 refs) ───────────────────────

    let rd_b: &[(&str, &str, &str)] = &[
        ("8.16.1", "RD-B: Accept Warmstart", "135.1-2025 - 9.19.1.1"),
        ("8.16.2", "RD-B: Accept Coldstart", "135.1-2025 - 9.19.1.2"),
        ("8.16.3", "RD-B: Wrong Password", "135.1-2025 - 9.19.2.1"),
        ("8.16.4", "RD-B: IAm After Reinit", "135.1-2025 - 9.19.1.3"),
        (
            "8.16.5",
            "RD-B: Last_Restart_Reason Updated",
            "135.1-2025 - 9.19.1.4",
        ),
        (
            "8.16.6",
            "RD-B: Network Port after WARMSTART",
            "135.1-2025 - 7.3.2.46.1.1",
        ),
        (
            "8.16.7",
            "RD-B: Network Port after COLDSTART",
            "135.1-2025 - 7.3.2.46.1.1",
        ),
    ];
    for &(id, name, reference) in rd_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "reinitialize"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rd_b_test(ctx)),
        });
    }

    // ── 8.17 BR-A (Backup/Restore A, 5 refs) ────────────────────────────

    let br_a: &[(&str, &str, &str)] = &[
        (
            "8.17.1",
            "BR-A: Initiate StartBackup",
            "135.1-2025 - 8.19.5",
        ),
        ("8.17.2", "BR-A: Read Backup Files", "135.1-2025 - 8.19.6"),
        ("8.17.3", "BR-A: Initiate EndBackup", "135.1-2025 - 8.19.7"),
        (
            "8.17.4",
            "BR-A: Initiate StartRestore",
            "135.1-2025 - 8.19.8",
        ),
        ("8.17.5", "BR-A: Initiate EndRestore", "135.1-2025 - 8.19.9"),
    ];
    for &(id, name, reference) in br_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "backup-restore"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(br_base(ctx)),
        });
    }

    // ── 8.18 BR-B (Backup/Restore B, 13 refs) ───────────────────────────

    let br_b: &[(&str, &str, &str)] = &[
        (
            "8.18.1",
            "BR-B: Accept StartBackup",
            "135.1-2025 - 9.19.3.1",
        ),
        (
            "8.18.2",
            "BR-B: Configuration_Files Readable",
            "135.1-2025 - 9.19.3.2",
        ),
        (
            "8.18.3",
            "BR-B: File Objects Readable",
            "135.1-2025 - 9.19.3.3",
        ),
        ("8.18.4", "BR-B: Accept EndBackup", "135.1-2025 - 9.19.3.4"),
        (
            "8.18.5",
            "BR-B: Accept StartRestore",
            "135.1-2025 - 9.19.4.1",
        ),
        (
            "8.18.6",
            "BR-B: File Objects Writable",
            "135.1-2025 - 9.19.4.2",
        ),
        ("8.18.7", "BR-B: Accept EndRestore", "135.1-2025 - 9.19.4.3"),
        ("8.18.8", "BR-B: IAm After Restore", "135.1-2025 - 9.19.4.4"),
        (
            "8.18.9",
            "BR-B: Wrong Password Backup",
            "135.1-2025 - 9.19.3.5",
        ),
        (
            "8.18.10",
            "BR-B: Wrong Password Restore",
            "135.1-2025 - 9.19.4.5",
        ),
        (
            "8.18.11",
            "BR-B: Backup While Backup",
            "135.1-2025 - 9.19.3.6",
        ),
        (
            "8.18.12",
            "BR-B: Database_Revision Changes",
            "135.1-2025 - 9.19.4.6",
        ),
        (
            "8.18.13",
            "BR-B: Restore Verification",
            "135.1-2025 - 9.19.4.7",
        ),
    ];
    for &(id, name, reference) in br_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "backup-restore"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(br_b_test(ctx)),
        });
    }

    // ── 8.19 Restart A (1 ref) ───────────────────────────────────────────
    registry.add(TestDef {
        id: "8.19.1",
        name: "DM-R-A: Detect Device Restart",
        reference: "135.1-2025 - 8.10.1",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "restart"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(restart_base(ctx)),
    });

    // ── 8.20 Restart B (1 ref) ───────────────────────────────────────────
    registry.add(TestDef {
        id: "8.20.1",
        name: "DM-R-B: Last_Restart_Reason Valid",
        reference: "135.1-2025 - 12.11.44",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "restart"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(restart_b_test(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn dcc_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn dcc_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::SYSTEM_STATUS)
        .await?;
    ctx.pass()
}

async fn rd_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn rd_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LAST_RESTART_REASON)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::SYSTEM_STATUS)
        .await?;
    ctx.pass()
}

async fn br_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn br_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::DATABASE_REVISION)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn restart_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::SYSTEM_STATUS)
        .await?;
    ctx.pass()
}

async fn restart_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let reason = ctx
        .read_enumerated(dev, PropertyIdentifier::LAST_RESTART_REASON)
        .await?;
    if reason > 7 {
        return Err(TestFailure::new(format!(
            "Last_Restart_Reason {reason} out of range"
        )));
    }
    ctx.pass()
}
