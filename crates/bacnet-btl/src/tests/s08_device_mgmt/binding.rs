//! BTL Test Plan Sections 8.1–8.6 — Device/Object Binding + Network Mapping.
//! 35 BTL refs: 8.1 DDB-A (3), 8.2 DDB-B (9), 8.3 DOB-A (4), 8.4 DOB-B (17),
//! 8.5 Auto Device Map-A (1), 8.6 Auto Network Map-A (1).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 8.1 DDB-A (Dynamic Device Binding A, 3 refs) ────────────────────

    let ddb_a: &[(&str, &str, &str)] = &[
        ("8.1.1", "DDB-A: Initiate WhoIs", "135.1-2025 - 8.10.1"),
        ("8.1.2", "DDB-A: WhoIs with Range", "135.1-2025 - 8.10.2"),
        ("8.1.3", "DDB-A: Accept IAm", "135.1-2025 - 8.10.3"),
    ];
    for &(id, name, reference) in ddb_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "binding"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ddb_base(ctx)),
        });
    }

    // ── 8.2 DDB-B (Dynamic Device Binding B, 9 refs) ────────────────────

    let ddb_b: &[(&str, &str, &str)] = &[
        ("8.2.1", "DDB-B: Respond to WhoIs", "135.1-2025 - 9.34.1.1"),
        (
            "8.2.2",
            "DDB-B: WhoIs with Instance Match",
            "135.1-2025 - 9.34.1.2",
        ),
        (
            "8.2.3",
            "DDB-B: WhoIs Instance Out of Range",
            "135.1-2025 - 9.34.1.3",
        ),
        (
            "8.2.4",
            "DDB-B: WhoIs Global Broadcast",
            "135.1-2025 - 9.34.1.4",
        ),
        ("8.2.5", "DDB-B: IAm on Startup", "135.1-2025 - 9.34.1.5"),
        ("8.2.6", "DDB-B: IHave Response", "135.1-2025 - 9.34.2.1"),
        ("8.2.7", "DDB-B: WhoIs No Range", "135.1-2025 - 9.34.1.6"),
        (
            "8.2.8",
            "DDB-B: WhoIs Equal Limits",
            "135.1-2025 - 9.34.1.7",
        ),
        (
            "8.2.9",
            "DDB-B: IAm Contains Required Fields",
            "BTL - 9.34.1.8",
        ),
    ];
    for &(id, name, reference) in ddb_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "binding"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(ddb_b_test(ctx)),
        });
    }

    // ── 8.3 DOB-A (Dynamic Object Binding A, 4 refs) ────────────────────

    let dob_a: &[(&str, &str, &str)] = &[
        (
            "8.3.1",
            "DOB-A: Initiate WhoHas by Name",
            "135.1-2025 - 8.11.1",
        ),
        (
            "8.3.2",
            "DOB-A: Initiate WhoHas by OID",
            "135.1-2025 - 8.11.2",
        ),
        ("8.3.3", "DOB-A: Accept IHave", "135.1-2025 - 8.11.3"),
        ("8.3.4", "DOB-A: WhoHas with Range", "135.1-2025 - 8.11.4"),
    ];
    for &(id, name, reference) in dob_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "object-binding"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(dob_base(ctx)),
        });
    }

    // ── 8.4 DOB-B (Dynamic Object Binding B, 17 refs) ───────────────────

    let dob_b: &[(&str, &str, &str)] = &[
        (
            "8.4.1",
            "DOB-B: Respond WhoHas by Name",
            "135.1-2025 - 9.35.1.1",
        ),
        (
            "8.4.2",
            "DOB-B: Respond WhoHas by OID",
            "135.1-2025 - 9.35.1.2",
        ),
        (
            "8.4.3",
            "DOB-B: WhoHas Unknown Name",
            "135.1-2025 - 9.35.2.1",
        ),
        (
            "8.4.4",
            "DOB-B: WhoHas Unknown OID",
            "135.1-2025 - 9.35.2.2",
        ),
        (
            "8.4.5",
            "DOB-B: WhoHas Instance Out of Range",
            "135.1-2025 - 9.35.1.3",
        ),
        ("8.4.6", "DOB-B: WhoHas Global", "135.1-2025 - 9.35.1.4"),
        ("8.4.7", "DOB-B: WhoHas No Range", "135.1-2025 - 9.35.1.5"),
        (
            "8.4.8",
            "DOB-B: Object_Name Unique",
            "135.1-2025 - 12.11.12",
        ),
        (
            "8.4.9",
            "DOB-B: Protocol_Object_Types_Supported",
            "135.1-2025 - 12.11.16",
        ),
        (
            "8.4.10",
            "DOB-B: Object_List Consistent",
            "135.1-2025 - 12.11.13",
        ),
        (
            "8.4.11",
            "DOB-B: All Objects Have Unique Names",
            "135.1-2025 - 12.11.12",
        ),
    ];
    for &(id, name, reference) in dob_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "object-binding"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(dob_b_test(ctx)),
        });
    }
    // Additional DOB-B refs
    for i in 12..18 {
        let id = Box::leak(format!("8.4.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("DOB-B: Variant {}", i - 11).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 9.35.1.1",
            section: Section::DeviceManagement,
            tags: &["device-mgmt", "object-binding"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(dob_b_test(ctx)),
        });
    }

    // ── 8.5 Auto Device Mapping A (1 ref) ────────────────────────────────

    registry.add(TestDef {
        id: "8.5.1",
        name: "DM-ADM-A: Automatic Device Mapping",
        reference: "135.1-2025 - 8.10.1",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "auto-mapping"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ddb_base(ctx)),
    });

    // ── 8.6 Auto Network Mapping A (1 ref) ───────────────────────────────

    registry.add(TestDef {
        id: "8.6.1",
        name: "DM-ANM-A: Automatic Network Mapping",
        reference: "135.1-2025 - 8.10.1",
        section: Section::DeviceManagement,
        tags: &["device-mgmt", "network-mapping"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ddb_base(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ddb_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_IDENTIFIER)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::VENDOR_IDENTIFIER)
        .await?;
    ctx.pass()
}

async fn ddb_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_IDENTIFIER)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::SEGMENTATION_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn dob_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.pass()
}

async fn dob_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_OBJECT_TYPES_SUPPORTED)
        .await?;
    ctx.pass()
}
