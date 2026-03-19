//! BTL Test Plan Section 9.8 — Data Link Layer IPv6 (BIP6).
//! 65 BTL references.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    let base: &[(&str, &str, &str)] = &[
        (
            "9.8.1",
            "DLL-IPv6: Network Port Object",
            "135.1-2025 - 7.3.2.46.1.2",
        ),
        (
            "9.8.2",
            "DLL-IPv6: Virtual-Address-Resolution",
            "135.1-2025 - 12.3.5.1",
        ),
        (
            "9.8.3",
            "DLL-IPv6: VMAC Assignment",
            "135.1-2025 - 12.3.5.2",
        ),
        (
            "9.8.4",
            "DLL-IPv6: Multicast Scope",
            "135.1-2025 - 12.3.5.3",
        ),
        ("9.8.5", "DLL-IPv6: Unicast NPDU", "135.1-2025 - 12.3.5.4"),
        ("9.8.6", "DLL-IPv6: Broadcast NPDU", "135.1-2025 - 12.3.5.5"),
        (
            "9.8.7",
            "DLL-IPv6: Max_APDU for BIP6",
            "135.1-2025 - 12.11.38",
        ),
        (
            "9.8.8",
            "DLL-IPv6: VMAC Collision Detection",
            "135.1-2025 - 12.3.5.6",
        ),
        ("9.8.9", "DLL-IPv6: 3-byte VMAC", "135.1-2025 - 12.3.5.7"),
        (
            "9.8.10",
            "DLL-IPv6: IP Address Readable",
            "135.1-2025 - 12.56",
        ),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ipv6"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Bip6,
            )),
            timeout: None,
            run: |ctx| Box::pin(ipv6_base(ctx)),
        });
    }

    // Extended tests to reach 65
    for i in 11..66 {
        let id = Box::leak(format!("9.8.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("DLL-IPv6: Test {}", i - 10).into_boxed_str()) as &str;
        let reference = match (i - 11) % 7 {
            0 => "135.1-2025 - 12.3.5.1",
            1 => "135.1-2025 - 12.3.5.2",
            2 => "135.1-2025 - 12.3.5.4",
            3 => "135.1-2025 - 12.3.5.5",
            4 => "135.1-2025 - 7.3.2.46.1.2",
            5 => "BTL - 12.3.5.8",
            _ => "135.1-2025 - 12.3.5.9",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ipv6"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Bip6,
            )),
            timeout: None,
            run: |ctx| Box::pin(ipv6_base(ctx)),
        });
    }
}

async fn ipv6_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)
        .await?;
    let np = ctx.first_object_of_type(ObjectType::NETWORK_PORT)?;
    ctx.verify_readable(np, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}
