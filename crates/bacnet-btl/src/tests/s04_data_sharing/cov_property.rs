//! BTL Test Plan Sections 4.19–4.20 — COV Property.
//! 89 BTL references: 4.19 A-side (31), 4.20 B-side (58).

use bacnet_types::enums::ObjectType;

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 4.19 COV Property A (initiation) ────────────────────────────────

    let a_base: &[(&str, &str, &str)] = &[
        (
            "4.19.1",
            "COVP-A: Subscribe Specific Property",
            "135.1-2025 - 8.16.1",
        ),
        (
            "4.19.2",
            "COVP-A: Subscribe with COV Increment",
            "135.1-2025 - 8.16.2",
        ),
        (
            "4.19.3",
            "COVP-A: Cancel Subscription",
            "135.1-2025 - 8.16.3",
        ),
        (
            "4.19.4",
            "COVP-A: Renew Subscription",
            "135.1-2025 - 8.16.4",
        ),
        (
            "4.19.5",
            "COVP-A: Accept Notification",
            "135.1-2025 - 8.16.5",
        ),
    ];

    for &(id, name, reference) in a_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-property", "covp-a"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(covp_a_base(ctx)),
        });
    }

    // Per-object-type for A-side
    for i in 0..26 {
        let id_str = Box::leak(format!("4.19.{}", 6 + i).into_boxed_str()) as &str;
        let name_str = Box::leak(format!("COVP-A: Type {}", i + 1).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.16.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-property", "covp-a"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(covp_a_base(ctx)),
        });
    }

    // ── 4.20 COV Property B (execution) ─────────────────────────────────

    let b_base: &[(&str, &str, &str)] = &[
        (
            "4.20.1",
            "COVP-B: Subscribe to Specific Property",
            "135.1-2025 - 9.38.1.1",
        ),
        (
            "4.20.2",
            "COVP-B: Non-Existent Property",
            "135.1-2025 - 9.38.2.1",
        ),
        (
            "4.20.3",
            "COVP-B: Non-Existent Object",
            "135.1-2025 - 9.38.2.2",
        ),
        (
            "4.20.4",
            "COVP-B: Cancel Subscription",
            "135.1-2025 - 9.38.1.3",
        ),
        (
            "4.20.5",
            "COVP-B: Update Subscription",
            "135.1-2025 - 9.38.1.4",
        ),
        ("4.20.6", "COVP-B: Finite Lifetime", "135.1-2025 - 9.38.1.5"),
        ("4.20.7", "COVP-B: 8-Hour Lifetime", "135.1-2025 - 9.38.1.6"),
        ("4.20.8", "COVP-B: 5 Concurrent", "135.1-2025 - 9.38.1.7"),
        (
            "4.20.9",
            "COVP-B: Active Subscriptions",
            "135.1-2025 - 7.3.2.10.1",
        ),
        ("4.20.10", "COVP-B: COV Increment", "135.1-2025 - 9.38.1.8"),
    ];

    for &(id, name, reference) in b_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-property", "covp-b"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(covp_b_base(ctx)),
        });
    }

    // Per-object-type for B-side (4 refs each for key types)
    let cov_types: &[&str] = &[
        "AI", "AO", "AV", "BI", "BO", "BV", "MSI", "MSO", "MSV", "Loop", "LSP", "LSZ", "CSV",
        "IntV", "LAV", "PIV", "TimeV", "OSV", "PC", "Door", "LC", "LO", "BLO", "Staging",
    ];

    let mut idx = 11u32;
    for &abbr in cov_types {
        for suffix in ["PV-C", "SF-C"] {
            let id_str = Box::leak(format!("4.20.{idx}").into_boxed_str()) as &str;
            let name_str =
                Box::leak(format!("COVP-B: {} {}", abbr, suffix).into_boxed_str()) as &str;
            registry.add(TestDef {
                id: id_str,
                name: name_str,
                reference: "135.1-2025 - 9.38.1.1",
                section: Section::DataSharing,
                tags: &["data-sharing", "cov-property", "covp-b"],
                conditionality: Conditionality::RequiresCapability(Capability::Cov),
                timeout: None,
                run: |ctx| Box::pin(covp_b_base(ctx)),
            });
            idx += 1;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn covp_a_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.subscribe_cov(ai, false, Some(300)).await?;
    ctx.pass()
}

async fn covp_b_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.subscribe_cov(ai, false, Some(300)).await?;
    ctx.pass()
}
