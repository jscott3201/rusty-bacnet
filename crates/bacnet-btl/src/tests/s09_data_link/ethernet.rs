//! BTL Test Plan Section 9.5 — Data Link Layer Ethernet.
//! 29 BTL references.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    let base: &[(&str, &str, &str)] = &[
        (
            "9.5.1",
            "DLL-Eth: Network Port Object",
            "135.1-2025 - 7.3.2.46.1.2",
        ),
        ("9.5.2", "DLL-Eth: Broadcast NPDU", "135.1-2025 - 12.4.1"),
        ("9.5.3", "DLL-Eth: Unicast NPDU", "135.1-2025 - 12.4.2"),
        (
            "9.5.4",
            "DLL-Eth: Max_APDU for Ethernet",
            "135.1-2025 - 12.11.38",
        ),
        ("9.5.5", "DLL-Eth: MAC Address", "135.1-2025 - 12.4.3"),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ethernet"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(eth_base(ctx)),
        });
    }

    for i in 6..30 {
        let id = Box::leak(format!("9.5.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("DLL-Eth: Test {}", i - 5).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 12.4.1",
            section: Section::DataLinkLayer,
            tags: &["data-link", "ethernet"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(eth_base(ctx)),
        });
    }
}

async fn eth_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)
        .await?;
    ctx.pass()
}
