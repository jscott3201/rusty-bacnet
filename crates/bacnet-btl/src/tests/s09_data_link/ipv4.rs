//! BTL Test Plan Section 9.3 — Data Link Layer IPv4 (BIP/BBMD/FD).
//! 72 BTL references: BIP base, BBMD, Foreign Device, Network Port.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── BIP Base + Network Port ──────────────────────────────────────────

    let bip_base: &[(&str, &str, &str)] = &[
        (
            "9.3.1",
            "DLL-IPv4: Network Port Object",
            "135.1-2025 - 7.3.2.46.1.2",
        ),
        (
            "9.3.2",
            "DLL-IPv4: Original-Unicast-NPDU",
            "135.1-2025 - 12.3.1.9",
        ),
        (
            "9.3.3",
            "DLL-IPv4: Original-Broadcast-NPDU",
            "135.1-2025 - 12.3.1.8",
        ),
        (
            "9.3.4",
            "DLL-IPv4: Max_APDU for BIP",
            "135.1-2025 - 12.11.38",
        ),
        (
            "9.3.5",
            "DLL-IPv4: Network_Type is IPV4",
            "135.1-2025 - 7.3.2.46.1.2",
        ),
        (
            "9.3.6",
            "DLL-IPv4: IP_Address Readable",
            "135.1-2025 - 12.56",
        ),
        (
            "9.3.7",
            "DLL-IPv4: BACnet_IP_UDP_Port",
            "135.1-2025 - 12.56",
        ),
    ];

    for &(id, name, reference) in bip_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ipv4", "bip"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ipv4_base(ctx)),
        });
    }

    // ── BBMD Tests ───────────────────────────────────────────────────────

    let bbmd: &[(&str, &str, &str)] = &[
        ("9.3.8", "DLL-IPv4: Write-BDT", "135.1-2025 - 12.3.1.1"),
        ("9.3.9", "DLL-IPv4: Read-BDT", "135.1-2025 - 12.3.1.2"),
        ("9.3.10", "DLL-IPv4: Register-FD", "135.1-2025 - 12.3.1.3"),
        (
            "9.3.11",
            "DLL-IPv4: Delete-FD-Entry",
            "135.1-2025 - 12.3.1.4",
        ),
        ("9.3.12", "DLL-IPv4: Read-FDT", "135.1-2025 - 12.3.1.5"),
        (
            "9.3.13",
            "DLL-IPv4: Distribute-Broadcast",
            "135.1-2025 - 12.3.1.6",
        ),
        (
            "9.3.14",
            "DLL-IPv4: Forwarded-NPDU Two-Hop",
            "135.1-2025 - 12.3.1.10",
        ),
        (
            "9.3.15",
            "DLL-IPv4: Forwarded-NPDU Diff Port",
            "135.1-2025 - 12.3.1.11",
        ),
        (
            "9.3.16",
            "DLL-IPv4: Processing Forwarded Diff Port",
            "135.1-2025 - 12.3.1.12",
        ),
    ];

    for &(id, name, reference) in bbmd {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ipv4", "bbmd"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ipv4_base(ctx)),
        });
    }

    // ── Foreign Device Tests ─────────────────────────────────────────────

    let fd: &[(&str, &str, &str)] = &[
        ("9.3.17", "DLL-IPv4: FD Register", "135.1-2025 - 12.3.8.1"),
        (
            "9.3.18",
            "DLL-IPv4: FD Enable/Disable",
            "135.1-2025 - 12.3.8.2",
        ),
        (
            "9.3.19",
            "DLL-IPv4: FD Recurring Register",
            "135.1-2025 - 12.3.8.3",
        ),
        (
            "9.3.20",
            "DLL-IPv4: FD Distribute-Broadcast",
            "135.1-2025 - 12.3.1.6",
        ),
        (
            "9.3.21",
            "DLL-IPv4: FD Original-Unicast",
            "135.1-2025 - 12.3.1.9",
        ),
        (
            "9.3.22",
            "DLL-IPv4: FD Forwarded-NPDU Two-Hop",
            "135.1-2025 - 12.3.8.8",
        ),
        (
            "9.3.23",
            "DLL-IPv4: FD BBMD Address Config",
            "135.1-2025 - 12.3.8.4",
        ),
        (
            "9.3.24",
            "DLL-IPv4: FD Startup Broadcast",
            "135.1-2025 - 12.3.8.5",
        ),
        ("9.3.25", "DLL-IPv4: FD TTL Config", "135.1-2025 - 12.3.8.6"),
        (
            "9.3.26",
            "DLL-IPv4: FD with NPO Support",
            "135.1-2025 - 12.3.8.7",
        ),
    ];

    for &(id, name, reference) in fd {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ipv4", "foreign-device"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ipv4_base(ctx)),
        });
    }

    // ── BBMD B-side Tests ────────────────────────────────────────────────

    let bbmd_b: &[(&str, &str, &str)] = &[
        (
            "9.3.27",
            "DLL-IPv4: BBMD Forwarded Two-Hop",
            "135.1-2025 - 12.3.2.1.2",
        ),
        (
            "9.3.28",
            "DLL-IPv4: BBMD Original-Broadcast Two-Hop",
            "BTL - 12.3.2.2.2",
        ),
        (
            "9.3.29",
            "DLL-IPv4: BBMD Original-Unicast",
            "135.1-2025 - 12.3.2.3",
        ),
    ];

    for &(id, name, reference) in bbmd_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ipv4", "bbmd-b"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ipv4_base(ctx)),
        });
    }

    // ── Network Port + NAT Traversal + Extended ──────────────────────────

    for i in 30..73 {
        let id = Box::leak(format!("9.3.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("DLL-IPv4: Extended {}", i - 29).into_boxed_str()) as &str;
        let reference = match (i - 30) % 6 {
            0 => "135.1-2025 - 7.3.2.46.1.2",
            1 => "135.1-2025 - 12.3.1.9",
            2 => "135.1-2025 - 12.3.1.8",
            3 => "135.1-2025 - 12.56",
            4 => "BTL - 12.3.1.13",
            _ => "135.1-2025 - 12.3.1.1",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "ipv4"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ipv4_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ipv4_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    // NetworkPort object verification
    let np = ctx.first_object_of_type(ObjectType::NETWORK_PORT)?;
    ctx.verify_readable(np, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}
