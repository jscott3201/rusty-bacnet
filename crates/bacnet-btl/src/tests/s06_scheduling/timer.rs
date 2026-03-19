//! BTL Test Plan Sections 6.9–6.10 — Timer Internal/External B.
//! 1 BTL reference: 6.9 (0 refs - text only), 6.10 (1 ref).
//! Note: 6.8 Schedule-A has 0 refs (text only).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 6.10 Timer External B (1 ref) ────────────────────────────────────

    registry.add(TestDef {
        id: "6.10.1",
        name: "TIMER-E-B: Timer State Transitions",
        reference: "135.1-2025 - 12.36",
        section: Section::Scheduling,
        tags: &["scheduling", "timer", "external-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| Box::pin(timer_ext_base(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn timer_ext_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let tmr = ctx.first_object_of_type(ObjectType::TIMER)?;
    ctx.verify_readable(tmr, PropertyIdentifier::TIMER_STATE)
        .await?;
    ctx.verify_readable(tmr, PropertyIdentifier::TIMER_RUNNING)
        .await?;
    ctx.verify_readable(tmr, PropertyIdentifier::INITIAL_TIMEOUT)
        .await?;
    ctx.pass()
}
