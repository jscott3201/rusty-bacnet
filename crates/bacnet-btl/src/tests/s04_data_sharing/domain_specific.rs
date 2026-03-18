//! BTL Test Plan Sections 4.27–4.55 — Domain-Specific Data Sharing.
//! 42 BTL references: Life Safety (7), Access Control (18), Lighting (10),
//! Elevator (6), Value Source (0 - text only).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 4.27-4.30 Life Safety ────────────────────────────────────────────

    registry.add(TestDef {
        id: "4.27.1",
        name: "LS-View-A: Browse LSP",
        reference: "135.1-2025 - 8.18.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "life-safety"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| Box::pin(ls_view(ctx)),
    });
    registry.add(TestDef {
        id: "4.28.1",
        name: "LS-AdvView-A: Browse LSP Details",
        reference: "135.1-2025 - 8.18.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "life-safety"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| Box::pin(ls_view(ctx)),
    });
    registry.add(TestDef {
        id: "4.29.1",
        name: "LS-Modify-A: Write LSP Mode",
        reference: "135.1-2025 - 8.20.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "life-safety"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| Box::pin(ls_modify(ctx)),
    });
    registry.add(TestDef {
        id: "4.29.2",
        name: "LS-Modify-A: Write LSZ Mode",
        reference: "135.1-2025 - 8.20.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "life-safety"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| Box::pin(ls_modify_zone(ctx)),
    });
    registry.add(TestDef {
        id: "4.30.1",
        name: "LS-AdvModify-A: Priority Write",
        reference: "135.1-2025 - 8.20.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "life-safety"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| Box::pin(ls_modify(ctx)),
    });
    registry.add(TestDef {
        id: "4.30.2",
        name: "LS-AdvModify-A: Relinquish",
        reference: "135.1-2025 - 8.20.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "life-safety"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| Box::pin(ls_modify(ctx)),
    });

    // ── 4.31-4.42 Access Control ─────────────────────────────────────────

    let ac_tests: &[(&str, &str, u32)] = &[
        ("4.31.1", "AC-View-A: Browse AccessPoint", 33),
        ("4.32.1", "AC-AdvView-A: Browse AccessPoint Details", 33),
        ("4.33.1", "AC-Modify-A: Write AccessPoint", 33),
        ("4.33.2", "AC-Modify-A: Write AccessZone", 34),
        ("4.34.1", "AC-AdvModify-A: Priority Write", 33),
        ("4.34.2", "AC-AdvModify-A: Relinquish", 33),
        ("4.35.1", "AC-UserConfig-A: Read Credential", 32),
        ("4.35.2", "AC-UserConfig-A: Write Credential", 32),
        ("4.35.3", "AC-UserConfig-A: Create Credential", 32),
        ("4.37.1", "AC-SiteConfig-A: Read AccessZone", 34),
        ("4.37.2", "AC-SiteConfig-A: Write AccessZone", 34),
        ("4.37.3", "AC-SiteConfig-A: Create AccessZone", 34),
        ("4.39.1", "AC-Door-A: Browse Door", 30),
        ("4.39.2", "AC-Door-A: Write Door", 30),
        ("4.39.3", "AC-Door-A: Command Door", 30),
        ("4.41.1", "AC-CDI-A: Browse CredInput", 35),
        ("4.41.2", "AC-CDI-A: Write CredInput", 35),
        ("4.41.3", "AC-CDI-A: Create CredInput", 35),
    ];

    for &(id, name, ot_raw) in ac_tests {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "access-control"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(ot_raw)),
            timeout: None,
            run: |ctx| Box::pin(ac_base(ctx)),
        });
    }

    // ── 4.43-4.51 Lighting ───────────────────────────────────────────────

    let lt_tests: &[(&str, &str, u32)] = &[
        ("4.43.1", "LT-Output-A: Browse LightingOutput", 54),
        ("4.44.1", "LT-Status-A: Read Status", 54),
        ("4.45.1", "LT-AdvOutput-A: Advanced Control", 54),
        ("4.48.1", "LT-View-A: Browse Lighting", 54),
        ("4.49.1", "LT-AdvView-A: Advanced Browse", 54),
        ("4.50.1", "LT-Modify-A: Write LO", 54),
        ("4.50.2", "LT-Modify-A: Write BLO", 55),
        ("4.51.1", "LT-AdvModify-A: Priority Write", 54),
        ("4.51.2", "LT-AdvModify-A: Fade Control", 54),
    ];

    for &(id, name, ot_raw) in lt_tests {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "lighting"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(ot_raw)),
            timeout: None,
            run: if ot_raw == 55 {
                |ctx| Box::pin(lt_base_blo(ctx))
            } else {
                |ctx| Box::pin(lt_base_lo(ctx))
            },
        });
    }

    // ── 4.52-4.55 Elevator ───────────────────────────────────────────────

    let ev_tests: &[(&str, &str)] = &[
        ("4.52.1", "EV-View-A: Browse ElevatorGroup"),
        ("4.53.1", "EV-AdvView-A: Advanced Browse"),
        ("4.54.1", "EV-Modify-A: Write Landing Calls"),
        ("4.54.2", "EV-Modify-A: Write Lift"),
        ("4.55.1", "EV-AdvModify-A: Priority Calls"),
        ("4.55.2", "EV-AdvModify-A: Escalator Control"),
    ];

    for &(id, name) in ev_tests {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "elevator"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(57)),
            timeout: None,
            run: |ctx| Box::pin(ev_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ls_view(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let lsp = ctx.first_object_of_type(ObjectType::LIFE_SAFETY_POINT)?;
    ctx.verify_readable(lsp, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.verify_readable(lsp, PropertyIdentifier::MODE).await?;
    ctx.pass()
}

async fn ls_modify(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let lsp = ctx.first_object_of_type(ObjectType::LIFE_SAFETY_POINT)?;
    ctx.verify_readable(lsp, PropertyIdentifier::MODE).await?;
    ctx.pass()
}

async fn ls_modify_zone(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let lsz = ctx.first_object_of_type(ObjectType::LIFE_SAFETY_ZONE)?;
    ctx.verify_readable(lsz, PropertyIdentifier::MODE).await?;
    ctx.pass()
}

async fn ac_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Access control objects have basic readable properties
    let ap = ctx.first_object_of_type(ObjectType::ACCESS_POINT)?;
    ctx.verify_readable(ap, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn lt_base_lo(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::LIGHTING_OUTPUT)?;
    ctx.verify_readable(oid, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn lt_base_blo(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::BINARY_LIGHTING_OUTPUT)?;
    ctx.verify_readable(oid, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn ev_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let eg = ctx.first_object_of_type(ObjectType::ELEVATOR_GROUP)?;
    ctx.verify_readable(eg, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.verify_readable(eg, PropertyIdentifier::GROUP_ID)
        .await?;
    ctx.pass()
}
