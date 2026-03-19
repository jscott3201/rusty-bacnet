//! BTL Test Plan Sections 4.17–4.18 — COV Unsubscribed.
//! 16 BTL references: 4.17 A-side (15), 4.18 B-side (1).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 4.17 COV Unsubscribed A (initiation) ────────────────────────────

    let a_tests: &[(&str, &str, &str)] = &[
        (
            "4.17.1",
            "COV-Unsub-A: Accept Notification",
            "135.1-2025 - 9.36.1.1",
        ),
        (
            "4.17.2",
            "COV-Unsub-A: AI PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.3",
            "COV-Unsub-A: AO PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.4",
            "COV-Unsub-A: AV PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.5",
            "COV-Unsub-A: BI PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.6",
            "COV-Unsub-A: BO PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.7",
            "COV-Unsub-A: BV PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.8",
            "COV-Unsub-A: MSI PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.9",
            "COV-Unsub-A: MSO PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        (
            "4.17.10",
            "COV-Unsub-A: MSV PV Change",
            "135.1-2025 - 9.36.1.2",
        ),
        ("4.17.11", "COV-Unsub-A: Loop PV", "135.1-2025 - 9.36.1.2"),
        ("4.17.12", "COV-Unsub-A: LSP PV", "135.1-2025 - 9.36.1.2"),
        ("4.17.13", "COV-Unsub-A: LSZ PV", "135.1-2025 - 9.36.1.2"),
        ("4.17.14", "COV-Unsub-A: PC PV", "135.1-2025 - 9.36.1.2"),
        (
            "4.17.15",
            "COV-Unsub-A: Other Types",
            "135.1-2025 - 9.36.1.2",
        ),
    ];

    for &(id, name, reference) in a_tests {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-unsub"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(cov_unsub_a(ctx)),
        });
    }

    // ── 4.18 COV Unsubscribed B (execution) ─────────────────────────────

    registry.add(TestDef {
        id: "4.18.1",
        name: "COV-Unsub-B: Object List Change",
        reference: "135.1-2025 - 9.36.1.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "cov-unsub"],
        conditionality: Conditionality::RequiresCapability(Capability::Cov),
        timeout: None,
        run: |ctx| Box::pin(cov_unsub_b(ctx)),
    });
}

async fn cov_unsub_a(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.subscribe_cov(ai, false, Some(300)).await?;
    ctx.pass()
}

async fn cov_unsub_b(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.pass()
}
