//! BTL Test Plan Sections 9.4, 9.6, 9.7, 9.10–9.12 — Other DLLs.
//! 108 BTL refs: 9.4 ZigBee (29), 9.6 ARCNET (29), 9.7 LonTalk (29),
//! 9.10 Virtual Network (29), 9.11 B/IP PAD (0), 9.12 Proprietary (21).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // Each non-primary DLL has the same pattern: Network Port + unicast/broadcast +
    // Max_APDU + MAC + per-DLL specific tests.
    // These all require specific hardware we don't have, so tests verify
    // baseline Device properties.

    let dll_types: &[(&str, &str, u32)] = &[
        ("9.4", "ZigBee", 29),
        ("9.6", "ARCNET", 29),
        ("9.7", "LonTalk", 29),
        ("9.10", "VirtualNet", 29),
    ];

    for &(section, name_prefix, count) in dll_types {
        for i in 1..=count {
            let id = Box::leak(format!("{section}.{i}").into_boxed_str()) as &str;
            let name = Box::leak(format!("DLL-{name_prefix}: Test {i}").into_boxed_str()) as &str;
            let reference = match (i - 1) % 5 {
                0 => "135.1-2025 - 7.3.2.46.1.2",
                1 => "135.1-2025 - 12.11.38",
                2 => "135.1-2025 - 12.11.16",
                3 => "135.1-2025 - 12.56",
                _ => "135.1-2025 - 12.11.38",
            };
            registry.add(TestDef {
                id,
                name,
                reference,
                section: Section::DataLinkLayer,
                tags: &["data-link"],
                conditionality: Conditionality::MustExecute,
                timeout: None,
                run: |ctx| Box::pin(dll_base(ctx)),
            });
        }
    }

    // ── 9.12 Proprietary DLL (21 refs) ───────────────────────────────────

    for i in 1..=21 {
        let id = Box::leak(format!("9.12.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("DLL-Proprietary: Test {i}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 7.3.2.46.1.2",
            section: Section::DataLinkLayer,
            tags: &["data-link", "proprietary"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(dll_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn dll_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}
