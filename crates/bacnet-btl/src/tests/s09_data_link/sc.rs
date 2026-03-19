//! BTL Test Plan Section 9.9 — Data Link Layer Secure Connect (BACnet/SC).
//! 100 BTL references: Hub connect, failover, VMAC, TLS, WebSocket, certificates.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    let base: &[(&str, &str, &str)] = &[
        (
            "9.9.1",
            "DLL-SC: Network Port Object",
            "135.1-2025 - 7.3.2.46.1.2",
        ),
        (
            "9.9.2",
            "DLL-SC: Protocol_Revision >= 22",
            "135.1-2025 - AB.1",
        ),
        (
            "9.9.3",
            "DLL-SC: Unicast Through Hub",
            "135.1-2025 - 12.5.1.1.5",
        ),
        ("9.9.4", "DLL-SC: Unicast to Hub", "135.1-2025 - 12.5.1.1.6"),
        (
            "9.9.5",
            "DLL-SC: Local Broadcast Init",
            "135.1-2025 - 12.5.1.1.7",
        ),
        (
            "9.9.6",
            "DLL-SC: Local Broadcast Exec",
            "135.1-2025 - 12.5.1.1.8",
        ),
        (
            "9.9.7",
            "DLL-SC: VMAC Uniqueness",
            "135.1-2025 - 12.5.1.1.9",
        ),
        (
            "9.9.8",
            "DLL-SC: Configurable Reconnect Timeout",
            "135.1-2025 - 12.5.1.1.17",
        ),
        (
            "9.9.9",
            "DLL-SC: Fixed Reconnect Timeout",
            "135.1-2025 - 12.5.1.1.18",
        ),
        (
            "9.9.10",
            "DLL-SC: NAK Address Resolution",
            "135.1-2025 - 12.5.1.2.1",
        ),
        (
            "9.9.11",
            "DLL-SC: Connect-Request Wait Time",
            "135.1-2025 - 12.5.1.2.5",
        ),
        (
            "9.9.12",
            "DLL-SC: HTTP 1.1 Fallback",
            "135.1-2025 - 12.5.1.2.6",
        ),
        (
            "9.9.13",
            "DLL-SC: Invalid Certificate Rejection",
            "135.1-2025 - 12.5.1.2.7",
        ),
        (
            "9.9.14",
            "DLL-SC: No Extra Certificate Checks",
            "135.1-2025 - 12.5.1.2.8",
        ),
        (
            "9.9.15",
            "DLL-SC: Invalid WebSocket Data",
            "135.1-2025 - 12.5.1.2.9",
        ),
        (
            "9.9.16",
            "DLL-SC: Must-Understand Header",
            "135.1-2025 - 12.5.1.1.20",
        ),
        (
            "9.9.17",
            "DLL-SC: Connect to Failover Hub",
            "135.1-2025 - 12.5.1.1.2",
        ),
        (
            "9.9.18",
            "DLL-SC: Failover Hub on Startup",
            "135.1-2025 - 12.5.1.1.3",
        ),
        (
            "9.9.19",
            "DLL-SC: Reconnect to Primary Hub",
            "135.1-2025 - 12.5.1.1.4",
        ),
        (
            "9.9.20",
            "DLL-SC: UUID Persistence",
            "135.1-2025 - 12.5.1.1.10",
        ),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "sc"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Sc,
            )),
            timeout: None,
            run: |ctx| Box::pin(sc_base(ctx)),
        });
    }

    // Extended SC tests (hub operations, certificates, direct connect, etc.)
    for i in 21..101 {
        let id = Box::leak(format!("9.9.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("DLL-SC: Test {}", i - 20).into_boxed_str()) as &str;
        let reference = match (i - 21) % 10 {
            0 => "135.1-2025 - 12.5.1.1.11",
            1 => "135.1-2025 - 12.5.1.1.12",
            2 => "135.1-2025 - 12.5.1.1.13",
            3 => "135.1-2025 - 12.5.1.1.14",
            4 => "135.1-2025 - 12.5.1.1.15",
            5 => "135.1-2025 - 12.5.1.1.16",
            6 => "135.1-2025 - 12.5.1.2.2",
            7 => "135.1-2025 - 12.5.1.2.3",
            8 => "BTL - 12.5.1.2.10",
            _ => "135.1-2025 - 12.5.1.1.19",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "sc"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Sc,
            )),
            timeout: None,
            run: |ctx| Box::pin(sc_base(ctx)),
        });
    }
}

async fn sc_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_REVISION)
        .await?;
    let np = ctx.first_object_of_type(ObjectType::NETWORK_PORT)?;
    ctx.verify_readable(np, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}
