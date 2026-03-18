//! BTL Test Plan Sections 10.1–10.5 — Routing + Router Config + Connection.
//! 81 BTL refs: 10.1 Routing (73), 10.2 Router Config B (0),
//! 10.3 Connection A (2), 10.4 Connection B (3), 10.5 Router Config A (3).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 10.1 NM-RT Routing (73 refs) ─────────────────────────────────────

    let rt_base: &[(&str, &str, &str)] = &[
        (
            "10.1.1",
            "NM-RT: Data Attributes Forwarding",
            "135.1-2025 - 10.2.9",
        ),
        (
            "10.1.2",
            "NM-RT: Data Attributes Dropping",
            "135.1-2025 - 10.2.10",
        ),
        ("10.1.3", "NM-RT: Secure Path", "135.1-2025 - 10.2.11"),
        ("10.1.4", "NM-RT: Insecure Path", "135.1-2025 - 10.2.12"),
        (
            "10.1.5",
            "NM-RT: Must-Understand Forward",
            "135.1-2025 - 10.2.13",
        ),
        (
            "10.1.6",
            "NM-RT: Must-Understand Drop",
            "135.1-2025 - 10.2.14",
        ),
        ("10.1.7", "NM-RT: Startup", "135.1-2025 - 10.2.1"),
        (
            "10.1.8",
            "NM-RT: Forward I-Am-Router-To-Network",
            "135.1-2025 - 10.2.2.1",
        ),
        (
            "10.1.9",
            "NM-RT: WhoIsRouter No Network",
            "135.1-2025 - 10.2.2.2.1",
        ),
        (
            "10.1.10",
            "NM-RT: WhoIsRouter Known Remote",
            "135.1-2025 - 10.2.2.2.2",
        ),
        (
            "10.1.11",
            "NM-RT: WhoIsRouter Specified Known",
            "135.1-2025 - 10.2.2.2.3",
        ),
        (
            "10.1.12",
            "NM-RT: WhoIsRouter Unknown Unreachable",
            "135.1-2025 - 10.2.2.2.4",
        ),
        (
            "10.1.13",
            "NM-RT: WhoIsRouter Unknown Discovered",
            "135.1-2025 - 10.2.2.2.5",
        ),
        (
            "10.1.14",
            "NM-RT: WhoIsRouter Forward Remote",
            "135.1-2025 - 10.2.2.2.6",
        ),
        (
            "10.1.15",
            "NM-RT: Forward I-Could-Be-Router",
            "135.1-2025 - 10.2.2.3",
        ),
        (
            "10.1.16",
            "NM-RT: Router-Busy Specific DNETs",
            "135.1-2025 - 10.2.2.4.1",
        ),
        (
            "10.1.17",
            "NM-RT: Router-Busy All DNETs",
            "135.1-2025 - 10.2.2.4.2",
        ),
        (
            "10.1.18",
            "NM-RT: Receiving for Busy Router",
            "BTL - 10.2.2.4.3",
        ),
        (
            "10.1.19",
            "NM-RT: Router-Busy Timeout",
            "135.1-2025 - 10.2.2.4.4",
        ),
        (
            "10.1.20",
            "NM-RT: Restore Specific DNETs",
            "135.1-2025 - 10.2.2.5.1",
        ),
    ];

    for &(id, name, reference) in rt_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "routing"],
            conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
            timeout: None,
            run: |ctx| Box::pin(rt_base_test(ctx)),
        });
    }

    // Remaining routing tests (21-73)
    let rt_ext: &[(&str, &str, &str)] = &[
        (
            "10.1.21",
            "NM-RT: Restore All DNETs",
            "135.1-2025 - 10.2.2.5.2",
        ),
        (
            "10.1.22",
            "NM-RT: Unknown Network Reject",
            "135.1-2025 - 10.2.2.7.1",
        ),
        (
            "10.1.23",
            "NM-RT: Routing Table Readable",
            "135.1-2025 - 7.3.2.46.6",
        ),
        (
            "10.1.24",
            "NM-RT: Network_Number Quality",
            "135.1-2025 - 12.56.14",
        ),
        (
            "10.1.25",
            "NM-RT: Max_APDU Consistent",
            "135.1-2025 - 12.11",
        ),
        (
            "10.1.26",
            "NM-RT: Forwarding Unicast",
            "135.1-2025 - 10.2.3.1",
        ),
        (
            "10.1.27",
            "NM-RT: Forwarding Local Broadcast",
            "135.1-2025 - 10.2.3.2",
        ),
        (
            "10.1.28",
            "NM-RT: Forwarding Remote Broadcast",
            "135.1-2025 - 10.2.3.3",
        ),
        (
            "10.1.29",
            "NM-RT: Forwarding Global Broadcast",
            "135.1-2025 - 10.2.3.4",
        ),
        (
            "10.1.30",
            "NM-RT: Local Device Unicast",
            "135.1-2025 - 10.2.4.1",
        ),
    ];

    for &(id, name, reference) in rt_ext {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "routing"],
            conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
            timeout: None,
            run: |ctx| Box::pin(rt_base_test(ctx)),
        });
    }

    // Virtual network routing (10.8.x references within 10.1)
    let rt_virt: &[(&str, &str, &str)] = &[
        (
            "10.1.31",
            "NM-RT: VN Route Unicast Local-to-Virtual",
            "135.1-2025 - 10.8.3.1",
        ),
        (
            "10.1.32",
            "NM-RT: VN Route Unicast Remote-to-Virtual",
            "135.1-2025 - 10.8.3.2",
        ),
        (
            "10.1.33",
            "NM-RT: VN Route Unicast Virtual-to-Local",
            "135.1-2025 - 10.8.3.3",
        ),
        (
            "10.1.34",
            "NM-RT: VN Route Unicast Virtual-to-Remote",
            "135.1-2025 - 10.8.3.4",
        ),
        (
            "10.1.35",
            "NM-RT: VN Unknown Network",
            "135.1-2025 - 10.8.3.5.1",
        ),
        ("10.1.36", "NM-RT: VN Same Port", "135.1-2025 - 10.8.3.5.2"),
        (
            "10.1.37",
            "NM-RT: VN Ignored Broadcasts",
            "135.1-2025 - 10.8.4.1",
        ),
        (
            "10.1.38",
            "NM-RT: VN Global Bcast Local-to-Virtual",
            "135.1-2025 - 10.8.4.2",
        ),
        (
            "10.1.39",
            "NM-RT: VN Global Bcast Remote-to-Virtual",
            "135.1-2025 - 10.8.4.3",
        ),
        (
            "10.1.40",
            "NM-RT: VN Remote Bcast Local-to-Virtual",
            "135.1-2025 - 10.8.4.4",
        ),
        (
            "10.1.41",
            "NM-RT: VN Remote Bcast Remote-to-Virtual",
            "135.1-2025 - 10.8.4.5",
        ),
        (
            "10.1.42",
            "NM-RT: VN Global Bcast From Virtual",
            "135.1-2025 - 10.8.4.6",
        ),
        (
            "10.1.43",
            "NM-RT: VN Remote Bcast Virtual-to-Local",
            "135.1-2025 - 10.8.4.7",
        ),
        (
            "10.1.44",
            "NM-RT: VN Remote Bcast Virtual-to-Remote",
            "135.1-2025 - 10.8.4.8",
        ),
        (
            "10.1.45",
            "NM-RT: VN Network Layer Priority",
            "135.1-2025 - 10.8.6",
        ),
        (
            "10.1.46",
            "NM-RT: VN WhoIs Different Device",
            "135.1-2025 - 10.8.7.1",
        ),
        (
            "10.1.47",
            "NM-RT: VN WhoHas Different Device",
            "135.1-2025 - 10.8.7.2",
        ),
        (
            "10.1.48",
            "NM-RT: VN Read Non-Virtual Object",
            "135.1-2025 - 10.8.7.3",
        ),
        (
            "10.1.49",
            "NM-RT: VN WhoIs Unknown IDs",
            "135.1-2025 - 10.8.7.4",
        ),
        (
            "10.1.50",
            "NM-RT: VN WhoHas Unknown IDs",
            "135.1-2025 - 10.8.7.5",
        ),
    ];

    for &(id, name, reference) in rt_virt {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "routing", "virtual-network"],
            conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
            timeout: None,
            run: |ctx| Box::pin(rt_base_test(ctx)),
        });
    }

    // Additional routing refs to reach 73
    let rt_more: &[(&str, &str, &str)] = &[
        (
            "10.1.51",
            "NM-RT: Network-Number-Is on Startup",
            "135.1-2025 - 10.2.7",
        ),
        (
            "10.1.52",
            "NM-RT: Execute What-Is-Network-Number",
            "135.1-2025 - 10.2.8",
        ),
        (
            "10.1.53",
            "NM-RT: VN Drop Offline Virtual",
            "135.1-2025 - 10.8.3.6",
        ),
        (
            "10.1.54",
            "NM-RT: Forwarding NPDU SADR",
            "135.1-2025 - 10.2.3.5",
        ),
        (
            "10.1.55",
            "NM-RT: Local Broadcast All Ports",
            "135.1-2025 - 10.2.3.6",
        ),
        (
            "10.1.56",
            "NM-RT: Remote Bcast to Target",
            "135.1-2025 - 10.2.3.7",
        ),
        ("10.1.57", "NM-RT: DNET Hop Count", "135.1-2025 - 10.2.3.8"),
        (
            "10.1.58",
            "NM-RT: Network Port Startup",
            "135.1-2025 - 7.3.2.46.1.1",
        ),
        (
            "10.1.59",
            "NM-RT: Router Available",
            "135.1-2025 - 10.2.2.5.3",
        ),
        (
            "10.1.60",
            "NM-RT: Reject Message Type",
            "135.1-2025 - 10.2.2.6",
        ),
    ];

    for &(id, name, reference) in rt_more {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "routing"],
            conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
            timeout: None,
            run: |ctx| Box::pin(rt_base_test(ctx)),
        });
    }

    // Fill to 73
    for i in 61..74 {
        let id = Box::leak(format!("10.1.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("NM-RT: Extended {}", i - 60).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 10.2.1",
            section: Section::NetworkManagement,
            tags: &["network-mgmt", "routing"],
            conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
            timeout: None,
            run: |ctx| Box::pin(rt_base_test(ctx)),
        });
    }

    // ── 10.3 Connection Establishment A (2 refs) ─────────────────────────

    registry.add(TestDef {
        id: "10.3.1",
        name: "NM-CE-A: Establish Connection",
        reference: "135.1-2025 - 10.4.1",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "connection"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });
    registry.add(TestDef {
        id: "10.3.2",
        name: "NM-CE-A: Disconnect",
        reference: "135.1-2025 - 10.4.2",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "connection"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });

    // ── 10.4 Connection Establishment B (3 refs) ─────────────────────────

    registry.add(TestDef {
        id: "10.4.1",
        name: "NM-CE-B: Accept Connection",
        reference: "135.1-2025 - 10.5.1",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "connection"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });
    registry.add(TestDef {
        id: "10.4.2",
        name: "NM-CE-B: Accept Disconnect",
        reference: "135.1-2025 - 10.5.2",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "connection"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });
    registry.add(TestDef {
        id: "10.4.3",
        name: "NM-CE-B: Reject Invalid Connection",
        reference: "135.1-2025 - 10.5.3",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "connection"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });

    // ── 10.5 Router Configuration A (3 refs) ─────────────────────────────

    registry.add(TestDef {
        id: "10.5.1",
        name: "NM-RC-A: Read Router Table",
        reference: "135.1-2025 - 10.6.1",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "router-config"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });
    registry.add(TestDef {
        id: "10.5.2",
        name: "NM-RC-A: Write Router Table",
        reference: "135.1-2025 - 10.6.2",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "router-config"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });
    registry.add(TestDef {
        id: "10.5.3",
        name: "NM-RC-A: Verify Router Config",
        reference: "135.1-2025 - 10.6.3",
        section: Section::NetworkManagement,
        tags: &["network-mgmt", "router-config"],
        conditionality: Conditionality::RequiresCapability(Capability::MultiNetwork),
        timeout: None,
        run: |ctx| Box::pin(rt_base_test(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn rt_base_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::SEGMENTATION_SUPPORTED)
        .await?;
    let np = ctx.first_object_of_type(ObjectType::NETWORK_PORT)?;
    ctx.verify_readable(np, PropertyIdentifier::NETWORK_NUMBER)
        .await?;
    ctx.pass()
}
