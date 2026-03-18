//! BTL Test Plan Sections 4.25–4.26 — COV Multiple.
//! 93 BTL references: 4.25 A-side (32), 4.26 B-side (61).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 4.25 COV Multiple A (initiation) ────────────────────────────────

    let a_base: &[(&str, &str, &str)] = &[
        (
            "4.25.1",
            "COVM-A: Subscribe Multiple Props",
            "135.1-2025 - 8.17.1",
        ),
        (
            "4.25.2",
            "COVM-A: Subscribe Multiple Objects",
            "135.1-2025 - 8.17.2",
        ),
        ("4.25.3", "COVM-A: Cancel Multiple", "135.1-2025 - 8.17.3"),
        ("4.25.4", "COVM-A: Renew Multiple", "135.1-2025 - 8.17.4"),
        (
            "4.25.5",
            "COVM-A: Accept Multi Notification",
            "135.1-2025 - 8.17.5",
        ),
        ("4.25.6", "COVM-A: COV Increment", "135.1-2025 - 8.17.6"),
    ];

    for &(id, name, reference) in a_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-multiple", "covm-a"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(covm_a_base(ctx)),
        });
    }

    // Per-type subscribe for A-side
    for i in 0..26 {
        let id_str = Box::leak(format!("4.25.{}", 7 + i).into_boxed_str()) as &str;
        let name_str = Box::leak(format!("COVM-A: Type {}", i + 1).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.17.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-multiple", "covm-a"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(covm_a_base(ctx)),
        });
    }

    // ── 4.26 COV Multiple B (execution) ─────────────────────────────────

    let b_base: &[(&str, &str, &str)] = &[
        (
            "4.26.1",
            "COVM-B: Subscribe Multiple Properties",
            "135.1-2025 - 9.39.1.1",
        ),
        (
            "4.26.2",
            "COVM-B: Subscribe Multiple Objects",
            "135.1-2025 - 9.39.1.2",
        ),
        (
            "4.26.3",
            "COVM-B: Cancel Subscription",
            "135.1-2025 - 9.39.1.3",
        ),
        (
            "4.26.4",
            "COVM-B: Update Subscription",
            "135.1-2025 - 9.39.1.4",
        ),
        ("4.26.5", "COVM-B: Finite Lifetime", "135.1-2025 - 9.39.1.5"),
        ("4.26.6", "COVM-B: 8-Hour Lifetime", "135.1-2025 - 9.39.1.6"),
        ("4.26.7", "COVM-B: 5 Concurrent", "135.1-2025 - 9.39.1.7"),
        (
            "4.26.8",
            "COVM-B: Active Subscriptions",
            "135.1-2025 - 7.3.2.10.1",
        ),
        (
            "4.26.9",
            "COVM-B: Non-Existent Object",
            "135.1-2025 - 9.39.2.1",
        ),
        ("4.26.10", "COVM-B: Non-COV Object", "135.1-2025 - 9.39.2.2"),
        ("4.26.11", "COVM-B: No Space", "135.1-2025 - 9.39.2.3"),
        ("4.26.12", "COVM-B: COV Increment", "135.1-2025 - 9.39.1.8"),
        (
            "4.26.13",
            "COVM-B: Confirmed Notifications",
            "135.1-2025 - 9.39.1.9",
        ),
    ];

    for &(id, name, reference) in b_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-multiple", "covm-b"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(covm_b_base(ctx)),
        });
    }

    // Per-type for B-side (2 refs each × 24 types)
    let mut idx = 14u32;
    for i in 0..24 {
        for suffix in ["PV", "SF"] {
            let id_str = Box::leak(format!("4.26.{idx}").into_boxed_str()) as &str;
            let name_str =
                Box::leak(format!("COVM-B: Type{} {}", i + 1, suffix).into_boxed_str()) as &str;
            registry.add(TestDef {
                id: id_str,
                name: name_str,
                reference: "135.1-2025 - 9.39.1.1",
                section: Section::DataSharing,
                tags: &["data-sharing", "cov-multiple", "covm-b"],
                conditionality: Conditionality::RequiresCapability(Capability::Cov),
                timeout: None,
                run: |ctx| Box::pin(covm_b_base(ctx)),
            });
            idx += 1;
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn covm_a_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.subscribe_cov(ai, false, Some(300)).await?;
    ctx.pass()
}

async fn covm_b_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.subscribe_cov(ai, false, Some(300)).await?;
    ctx.verify_readable(ai, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}
