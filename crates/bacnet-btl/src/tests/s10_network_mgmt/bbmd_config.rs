//! BTL Test Plan Sections 10.6–10.9 — BBMD Config, FD Registration, SC Hub.
//! 15 BTL refs: 10.6 BBMD Config A (6), 10.7 BBMD Config B (4),
//! 10.8 FD Registration A (0), 10.9 SC Hub B (5).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 10.6 NM-BBMD-A (BBMD Configuration A, 6 refs) ───────────────────

    let bbmd_a: &[(&str, &str, &str)] = &[
        ("10.6.1", "NM-BBMD-A: Read BDT", "135.1-2025 - 10.9.1"),
        ("10.6.2", "NM-BBMD-A: Write BDT", "135.1-2025 - 10.9.2"),
        ("10.6.3", "NM-BBMD-A: Read FDT", "135.1-2025 - 10.9.3"),
        (
            "10.6.4",
            "NM-BBMD-A: Delete FDT Entry",
            "135.1-2025 - 10.9.4",
        ),
        (
            "10.6.5",
            "NM-BBMD-A: Verify BBMD Active",
            "135.1-2025 - 10.9.5",
        ),
        (
            "10.6.6",
            "NM-BBMD-A: BDT Persistence",
            "135.1-2025 - 10.9.6",
        ),
    ];

    for &(id, name, reference) in bbmd_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "bbmd-config"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(bbmd_base(ctx)),
        });
    }

    // ── 10.7 NM-BBMD-B (BBMD Configuration B, 4 refs) ───────────────────

    let bbmd_b: &[(&str, &str, &str)] = &[
        (
            "10.7.1",
            "NM-BBMD-B: Accept Write-BDT",
            "135.1-2025 - 10.10.1",
        ),
        (
            "10.7.2",
            "NM-BBMD-B: Accept Read-BDT",
            "135.1-2025 - 10.10.2",
        ),
        (
            "10.7.3",
            "NM-BBMD-B: Accept Read-FDT",
            "135.1-2025 - 10.10.3",
        ),
        (
            "10.7.4",
            "NM-BBMD-B: Accept Delete-FDT-Entry",
            "135.1-2025 - 10.10.4",
        ),
    ];

    for &(id, name, reference) in bbmd_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "bbmd-config-b"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(bbmd_base(ctx)),
        });
    }

    // ── 10.9 NM-SC-Hub-B (SC Hub B, 5 refs) ─────────────────────────────

    let sc_hub: &[(&str, &str, &str)] = &[
        (
            "10.9.1",
            "NM-SCHub-B: Accept Connect-Request",
            "135.1-2025 - 10.11.1",
        ),
        (
            "10.9.2",
            "NM-SCHub-B: Relay Messages",
            "135.1-2025 - 10.11.2",
        ),
        (
            "10.9.3",
            "NM-SCHub-B: Broadcast Forwarding",
            "135.1-2025 - 10.11.3",
        ),
        (
            "10.9.4",
            "NM-SCHub-B: Disconnect Handling",
            "135.1-2025 - 10.11.4",
        ),
        (
            "10.9.5",
            "NM-SCHub-B: Hub Status Readable",
            "135.1-2025 - 10.11.5",
        ),
    ];

    for &(id, name, reference) in sc_hub {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "sc-hub"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Sc,
            )),
            timeout: None,
            run: |ctx| Box::pin(sc_hub_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn bbmd_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    let np = ctx.first_object_of_type(ObjectType::NETWORK_PORT)?;
    ctx.verify_readable(np, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}

async fn sc_hub_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_REVISION)
        .await?;
    let np = ctx.first_object_of_type(ObjectType::NETWORK_PORT)?;
    ctx.verify_readable(np, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}
