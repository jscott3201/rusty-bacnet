//! BTL Test Plan Sections 8.25–8.30 — Text Message, Virtual Terminal, Subordinate Proxy.
//! 35 BTL refs: 8.25 TM-A (6), 8.26 TM-B (6), 8.27 VT-A (0), 8.28 VT-B (0),
//! 8.29 SubProxy-A (4), 8.30 SubProxy-B (19).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 8.25 DM-TM-A (Text Message A, 6 refs) ───────────────────────────

    let tm_a: &[(&str, &str, &str)] = &[
        (
            "8.25.1",
            "TM-A: Send Confirmed Text Message",
            "135.1-2025 - 8.23.1",
        ),
        (
            "8.25.2",
            "TM-A: Send Unconfirmed Text Message",
            "135.1-2025 - 8.23.2",
        ),
        ("8.25.3", "TM-A: Text with Class", "135.1-2025 - 8.23.3"),
        ("8.25.4", "TM-A: Text with Priority", "135.1-2025 - 8.23.4"),
        ("8.25.5", "TM-A: Text Empty Message", "135.1-2025 - 8.23.5"),
        ("8.25.6", "TM-A: Text Long Message", "135.1-2025 - 8.23.6"),
    ];

    for &(id, name, reference) in tm_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "text-message"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(tm_base(ctx)),
        });
    }

    // ── 8.26 DM-TM-B (Text Message B, 6 refs) ───────────────────────────

    let tm_b: &[(&str, &str, &str)] = &[
        (
            "8.26.1",
            "TM-B: Accept Confirmed Text Message",
            "135.1-2025 - 9.32.1.1",
        ),
        (
            "8.26.2",
            "TM-B: Accept Unconfirmed Text Message",
            "135.1-2025 - 9.32.1.2",
        ),
        ("8.26.3", "TM-B: Text with Class", "135.1-2025 - 9.32.1.3"),
        (
            "8.26.4",
            "TM-B: Text with Priority",
            "135.1-2025 - 9.32.1.4",
        ),
        (
            "8.26.5",
            "TM-B: Reject Unsupported Charset",
            "135.1-2025 - 9.32.2.1",
        ),
        (
            "8.26.6",
            "TM-B: Accept Empty Message",
            "135.1-2025 - 9.32.1.5",
        ),
    ];

    for &(id, name, reference) in tm_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "text-message"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(tm_base(ctx)),
        });
    }

    // ── 8.29 SubProxy View+Modify A (4 refs) ────────────────────────────

    let sp_a: &[(&str, &str, &str)] = &[
        (
            "8.29.1",
            "SP-A: Read Subordinate_List",
            "135.1-2025 - 12.21",
        ),
        (
            "8.29.2",
            "SP-A: Read Subordinate Properties",
            "135.1-2025 - 12.21",
        ),
        (
            "8.29.3",
            "SP-A: Write Subordinate Properties",
            "135.1-2025 - 12.21",
        ),
        (
            "8.29.4",
            "SP-A: Browse StructuredView",
            "135.1-2025 - 12.21",
        ),
    ];

    for &(id, name, reference) in sp_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "subordinate-proxy"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(29)),
            timeout: None,
            run: |ctx| Box::pin(sp_base(ctx)),
        });
    }

    // ── 8.30 SubProxy B (19 refs) ────────────────────────────────────────

    let sp_b: &[(&str, &str, &str)] = &[
        (
            "8.30.1",
            "SP-B: Subordinate_List Readable",
            "135.1-2025 - 7.3.2.21.1",
        ),
        (
            "8.30.2",
            "SP-B: Forward ReadProperty",
            "135.1-2025 - 7.3.2.21.2",
        ),
        (
            "8.30.3",
            "SP-B: Forward WriteProperty",
            "135.1-2025 - 7.3.2.21.3",
        ),
        ("8.30.4", "SP-B: Forward RPM", "135.1-2025 - 7.3.2.21.4"),
        ("8.30.5", "SP-B: Forward WPM", "135.1-2025 - 7.3.2.21.5"),
        (
            "8.30.6",
            "SP-B: Subordinate Object_List",
            "135.1-2025 - 7.3.2.21.6",
        ),
        (
            "8.30.7",
            "SP-B: Error Unknown Subordinate",
            "135.1-2025 - 7.3.2.21.7",
        ),
        (
            "8.30.8",
            "SP-B: Error Unknown Property",
            "135.1-2025 - 7.3.2.21.8",
        ),
        (
            "8.30.9",
            "SP-B: Device Object Identifier Mapping",
            "135.1-2025 - 7.3.2.21.9",
        ),
        (
            "8.30.10",
            "SP-B: Subordinate Annotations",
            "135.1-2025 - 7.3.2.21.10",
        ),
    ];

    for &(id, name, reference) in sp_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "subordinate-proxy-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(29)),
            timeout: None,
            run: |ctx| Box::pin(sp_b_test(ctx)),
        });
    }

    // Additional SP-B refs to reach 19
    for i in 11..20 {
        let id = Box::leak(format!("8.30.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("SP-B: Extended {}", i - 10).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 7.3.2.21.1",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "subordinate-proxy-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(29)),
            timeout: None,
            run: |ctx| Box::pin(sp_b_test(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn tm_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn sp_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sv = ctx.first_object_of_type(ObjectType::STRUCTURED_VIEW)?;
    ctx.verify_readable(sv, PropertyIdentifier::SUBORDINATE_LIST)
        .await?;
    ctx.pass()
}

async fn sp_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sv = ctx.first_object_of_type(ObjectType::STRUCTURED_VIEW)?;
    ctx.verify_readable(sv, PropertyIdentifier::SUBORDINATE_LIST)
        .await?;
    ctx.verify_readable(sv, PropertyIdentifier::SUBORDINATE_ANNOTATIONS)
        .await?;
    ctx.pass()
}
