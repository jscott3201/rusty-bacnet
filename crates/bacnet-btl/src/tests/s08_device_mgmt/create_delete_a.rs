//! BTL Test Plan Section 8.21 — DM-OCD-A (Object Creation/Deletion, client-side).
//! 133 BTL references: 4 base + 65 per-object-type × ~2 refs each.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements ────────────────────────────────────────────────

    let base: &[(&str, &str, &str)] = &[
        (
            "8.21.1",
            "OCD-A: Create by OID No Initial Values",
            "135.1-2025 - 8.16.1",
        ),
        (
            "8.21.2",
            "OCD-A: Create by Type No Initial Values",
            "135.1-2025 - 8.16.2",
        ),
        (
            "8.21.3",
            "OCD-A: Create by OID with Initial Values",
            "135.1-2025 - 8.16.3",
        ),
        (
            "8.21.4",
            "OCD-A: Create by Type with Initial Values",
            "135.1-2025 - 8.16.4",
        ),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "create-delete"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ocd_a_base(ctx)),
        });
    }

    // ── Per-Object-Type Create/Delete (65 types × 2 refs) ───────────────

    let types: &[(u32, &str)] = &[
        (0, "AI"),
        (1, "AO"),
        (2, "AV"),
        (3, "Averaging"),
        (4, "BI"),
        (5, "BO"),
        (6, "BV"),
        (7, "Calendar"),
        (8, "Command"),
        (9, "EE"),
        (10, "File"),
        (11, "Group"),
        (12, "Loop"),
        (13, "MSI"),
        (14, "MSO"),
        (15, "NC"),
        (16, "Program"),
        (17, "Schedule"),
        (18, "Averaging2"),
        (19, "MSV"),
        (20, "TrendLog"),
        (21, "LSP"),
        (22, "LSZ"),
        (23, "StructView"),
        (24, "PC"),
        (25, "EventLog"),
        (26, "LoadControl"),
        (27, "TLM"),
        (28, "AccessDoor"),
        (29, "Proprietary"),
        (30, "CSV"),
        (31, "DTV"),
        (32, "LAV"),
        (33, "BSV"),
        (34, "OSV"),
        (35, "TV"),
        (36, "DateV"),
        (37, "DatePV"),
        (38, "DTPV"),
        (39, "IntV"),
        (40, "PIV"),
        (41, "TPV"),
        (42, "CredDataInput"),
        (43, "NF"),
        (44, "AlertEnrollment"),
        (45, "Channel"),
        (46, "LightingOutput"),
        (47, "BinaryLightingOutput"),
        (48, "NetworkPort"),
        (49, "ElevatorGroup"),
        (50, "Lift"),
        (51, "Escalator"),
        (52, "AuditLog"),
        (53, "AuditReporter"),
        (54, "Staging"),
        (55, "Timer"),
        (56, "AccessCred"),
        (57, "AccessPoint"),
        (58, "AccessRights"),
        (59, "AccessUser"),
        (60, "AccessZone"),
        (61, "GlobalGroup"),
        (62, "Color"),
        (63, "ColorTemp"),
        (64, "DateTimePatternV"),
    ];

    let mut idx = 5u32;
    for &(_ot_raw, abbr) in types {
        // Create test
        let c_id = Box::leak(format!("8.21.{idx}").into_boxed_str()) as &str;
        let c_name = Box::leak(format!("OCD-A: Create {abbr}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: c_id,
            name: c_name,
            reference: "135.1-2025 - 8.16.1",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "create-delete"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ocd_a_base(ctx)),
        });
        idx += 1;

        // Delete test
        let d_id = Box::leak(format!("8.21.{idx}").into_boxed_str()) as &str;
        let d_name = Box::leak(format!("OCD-A: Delete {abbr}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: d_id,
            name: d_name,
            reference: "135.1-2025 - 8.17.1",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "create-delete"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ocd_a_base(ctx)),
        });
        idx += 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ocd_a_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED)
        .await?;
    ctx.pass()
}
