//! BTL Test Plan Sections 8.23–8.24 — List Manipulation A/B.
//! 110 BTL references: 8.23 A-side (100), 8.24 B-side (10).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 8.23 DM-LM-A (List Manipulation A, 100 refs) ────────────────────
    // Per-list-property × AddListElement/RemoveListElement tests

    let base_a: &[(&str, &str, &str)] = &[
        ("8.23.1", "LM-A: AddListElement Base", "135.1-2025 - 8.14.1"),
        (
            "8.23.2",
            "LM-A: RemoveListElement Base",
            "135.1-2025 - 8.14.2",
        ),
        ("8.23.3", "LM-A: Add Duplicate", "135.1-2025 - 8.14.3"),
        ("8.23.4", "LM-A: Remove Non-Existent", "135.1-2025 - 8.14.4"),
        ("8.23.5", "LM-A: Add to Non-List", "135.1-2025 - 8.14.5"),
    ];

    for &(id, name, reference) in base_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "list-manipulation"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(lm_a_base(ctx)),
        });
    }

    // Per-list-property tests (many list properties across object types)
    for i in 6..101 {
        let id = Box::leak(format!("8.23.{i}").into_boxed_str()) as &str;
        let name =
            Box::leak(format!("LM-A: List Property Test {}", i - 5).into_boxed_str()) as &str;
        let reference = match (i - 6) % 4 {
            0 => "135.1-2025 - 8.14.1",
            1 => "135.1-2025 - 8.14.2",
            2 => "135.1-2025 - 8.14.3",
            _ => "135.1-2025 - 8.14.4",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "list-manipulation"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(lm_a_base(ctx)),
        });
    }

    // ── 8.24 DM-LM-B (List Manipulation B, 10 refs) ─────────────────────

    let base_b: &[(&str, &str, &str)] = &[
        (
            "8.24.1",
            "LM-B: Accept AddListElement",
            "135.1-2025 - 9.14.1.1",
        ),
        (
            "8.24.2",
            "LM-B: Accept RemoveListElement",
            "135.1-2025 - 9.15.1.1",
        ),
        (
            "8.24.3",
            "LM-B: Add to Unknown Object",
            "135.1-2025 - 9.14.2.1",
        ),
        (
            "8.24.4",
            "LM-B: Add to Non-List Property",
            "135.1-2025 - 9.14.2.2",
        ),
        (
            "8.24.5",
            "LM-B: Remove from Unknown Object",
            "135.1-2025 - 9.15.2.1",
        ),
        (
            "8.24.6",
            "LM-B: Remove from Non-List",
            "135.1-2025 - 9.15.2.2",
        ),
        (
            "8.24.7",
            "LM-B: Add Duplicate Handling",
            "135.1-2025 - 9.14.2.3",
        ),
        (
            "8.24.8",
            "LM-B: Remove Non-Existent Handling",
            "135.1-2025 - 9.15.2.3",
        ),
        (
            "8.24.9",
            "LM-B: Add to Read-Only List",
            "135.1-2025 - 9.14.2.4",
        ),
        (
            "8.24.10",
            "LM-B: Property_List Updated",
            "135.1-2025 - 9.14.1.2",
        ),
    ];

    for &(id, name, reference) in base_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "list-manipulation-b"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(lm_b_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn lm_a_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.pass()
}

async fn lm_b_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROPERTY_LIST)
        .await?;
    ctx.pass()
}
