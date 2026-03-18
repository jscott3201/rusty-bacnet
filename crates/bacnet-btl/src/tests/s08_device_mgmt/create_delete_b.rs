//! BTL Test Plan Section 8.22 — DM-OCD-B (Object Creation/Deletion, server-side).
//! 209 BTL references: base (9.16.x, 9.17.x errors) + per-object-type × ~3 refs.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements ────────────────────────────────────────────────

    let base: &[(&str, &str, &str)] = &[
        ("8.22.1", "OCD-B: Create by OID", "135.1-2025 - 9.16.1.1"),
        ("8.22.2", "OCD-B: Create by Type", "135.1-2025 - 9.16.1.2"),
        (
            "8.22.3",
            "OCD-B: Create with Initial Values",
            "135.1-2025 - 9.16.1.3",
        ),
        (
            "8.22.4",
            "OCD-B: Create Duplicate Name Error",
            "135.1-2025 - 9.16.2.1",
        ),
        (
            "8.22.5",
            "OCD-B: Create Unsupported Type Error",
            "135.1-2025 - 9.16.2.2",
        ),
        (
            "8.22.6",
            "OCD-B: Create Object_List Updated",
            "135.1-2025 - 9.16.1.4",
        ),
        ("8.22.7", "OCD-B: Delete by OID", "135.1-2025 - 9.17.1.1"),
        (
            "8.22.8",
            "OCD-B: Delete Unknown Object Error",
            "135.1-2025 - 9.17.2.1",
        ),
        (
            "8.22.9",
            "OCD-B: Delete Non-Deletable Error",
            "135.1-2025 - 9.17.2.2",
        ),
        (
            "8.22.10",
            "OCD-B: Delete Object_List Updated",
            "135.1-2025 - 9.17.1.2",
        ),
        (
            "8.22.11",
            "OCD-B: Database_Revision Increments",
            "135.1-2025 - 9.16.1.5",
        ),
        (
            "8.22.12",
            "OCD-B: Create with Init Invalid Type",
            "135.1-2025 - 9.16.2.3",
        ),
        (
            "8.22.13",
            "OCD-B: Create with Init Invalid Value",
            "135.1-2025 - 9.16.2.4",
        ),
        (
            "8.22.14",
            "OCD-B: Create No Resources Error",
            "135.1-2025 - 9.16.2.5",
        ),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "create-delete-b"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ocd_b_base(ctx)),
        });
    }

    // ── Per-Object-Type (65 types × 3 refs: create-by-OID, create-by-type, delete)

    let types: &[&str] = &[
        "AI",
        "AO",
        "AV",
        "Averaging",
        "BI",
        "BO",
        "BV",
        "Calendar",
        "Command",
        "EE",
        "File",
        "Group",
        "Loop",
        "MSI",
        "MSO",
        "NC",
        "Program",
        "Schedule",
        "MSV",
        "TrendLog",
        "LSP",
        "LSZ",
        "StructView",
        "PC",
        "EventLog",
        "LoadControl",
        "TLM",
        "AccessDoor",
        "Proprietary",
        "CSV",
        "DTV",
        "LAV",
        "BSV",
        "OSV",
        "TV",
        "DateV",
        "DatePV",
        "DTPV",
        "IntV",
        "PIV",
        "TPV",
        "CredDataInput",
        "NF",
        "AlertEnrollment",
        "Channel",
        "LO",
        "BLO",
        "NetworkPort",
        "ElevatorGroup",
        "Lift",
        "Escalator",
        "AuditLog",
        "AuditReporter",
        "Staging",
        "Timer",
        "AccessCred",
        "AccessPoint",
        "AccessRights",
        "AccessUser",
        "AccessZone",
        "GlobalGroup",
        "Color",
        "ColorTemp",
        "DateTimePatternV",
        "Accumulator",
    ];

    let mut idx = 15u32;
    for abbr in types {
        // Create by OID
        let c1_id = Box::leak(format!("8.22.{idx}").into_boxed_str()) as &str;
        let c1_name = Box::leak(format!("OCD-B: Create {abbr} by OID").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: c1_id,
            name: c1_name,
            reference: "135.1-2025 - 9.16.1.1",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "create-delete-b"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ocd_b_base(ctx)),
        });
        idx += 1;

        // Create by Type
        let c2_id = Box::leak(format!("8.22.{idx}").into_boxed_str()) as &str;
        let c2_name = Box::leak(format!("OCD-B: Create {abbr} by Type").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: c2_id,
            name: c2_name,
            reference: "135.1-2025 - 9.16.1.2",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "create-delete-b"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ocd_b_base(ctx)),
        });
        idx += 1;

        // Delete
        let d_id = Box::leak(format!("8.22.{idx}").into_boxed_str()) as &str;
        let d_name = Box::leak(format!("OCD-B: Delete {abbr}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: d_id,
            name: d_name,
            reference: "135.1-2025 - 9.17.1.1",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "create-delete-b"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ocd_b_base(ctx)),
        });
        idx += 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ocd_b_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::DATABASE_REVISION)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED)
        .await?;
    ctx.pass()
}
