//! BTL Test Plan Section 12 — Network Security BIBBs.
//!
//! 9 subsections (12.1–12.9), **0 BTL test references**.
//! All subsections say "Contact BTL for Interim tests for this BIBB."
//! The BTL has not yet defined formal tests for network security.
//!
//! We register baseline property checks so the section is represented.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // All 9 subsections are "Contact BTL" with no formal tests.
    // Register one baseline test per subsection for coverage tracking.

    let subs: &[(&str, &str)] = &[
        ("12.1.1", "NSEC: Secure Device — Protocol_Revision"),
        ("12.2.1", "NSEC: Encrypted Device — Protocol_Revision"),
        ("12.3.1", "NSEC: Multi-App Device — Protocol_Revision"),
        ("12.4.1", "NSEC: DM Key A — Protocol_Revision"),
        ("12.5.1", "NSEC: DM Key B — Protocol_Revision"),
        ("12.6.1", "NSEC: Key Server — Protocol_Revision"),
        ("12.7.1", "NSEC: Temp Key Server — Protocol_Revision"),
        ("12.8.1", "NSEC: Secure Router — Protocol_Revision"),
        ("12.9.1", "NSEC: Security Proxy — Protocol_Revision"),
    ];

    for &(id, name) in subs {
        registry.add(TestDef {
            id,
            name,
            reference: "BTL - Contact BTL for Interim tests",
            section: Section::NetworkSecurity,
            tags: &["security"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(nsec_baseline(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn nsec_baseline(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let rev = ctx
        .read_unsigned(dev, PropertyIdentifier::PROTOCOL_REVISION)
        .await?;
    if rev == 0 {
        return Err(TestFailure::new("Protocol_Revision must be > 0"));
    }
    ctx.pass()
}
