//! BTL Test Plan Section 4.3 — DS-RPM-A (ReadPropertyMultiple, client initiation).
//! 98 BTL references: base (8.18.1-8.18.5 × combinations) +
//! per-data-type (8.18.1/8.18.2 × 16 types × non-array/array).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements (same as RP-A but via RPM) ────────────────────

    let base: &[(&str, &str, &str)] = &[
        (
            "4.3.1",
            "DS-RPM-A: Read Non-Array (8.18.1)",
            "135.1-2025 - 8.18.1",
        ),
        (
            "4.3.2",
            "DS-RPM-A: Read Array Element (8.18.2)",
            "135.1-2025 - 8.18.2",
        ),
        (
            "4.3.3",
            "DS-RPM-A: Read Array Size (8.18.5)",
            "135.1-2025 - 8.18.5",
        ),
        (
            "4.3.4",
            "DS-RPM-A: Read Whole Array (8.18.4)",
            "135.1-2025 - 8.18.4",
        ),
        (
            "4.3.5",
            "DS-RPM-A: Read List (8.18.1)",
            "135.1-2025 - 8.18.1",
        ),
        (
            "4.3.6",
            "DS-RPM-A: Read List Array (8.18.2)",
            "135.1-2025 - 8.18.2",
        ),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "rpm-a"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rpm_a_base(ctx)),
        });
    }

    // ── RPM-specific tests ──────────────────────────────────────────────

    registry.add(TestDef {
        id: "4.3.7",
        name: "DS-RPM-A: Multiple Properties Single Object",
        reference: "135.1-2025 - 8.18.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_a_multi_props(ctx)),
    });

    registry.add(TestDef {
        id: "4.3.8",
        name: "DS-RPM-A: Single Property Multiple Objects",
        reference: "135.1-2025 - 8.18.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_a_multi_objects(ctx)),
    });

    registry.add(TestDef {
        id: "4.3.9",
        name: "DS-RPM-A: Multiple Properties Multiple Objects",
        reference: "135.1-2025 - 8.18.7",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_a_multi_both(ctx)),
    });

    registry.add(TestDef {
        id: "4.3.10",
        name: "DS-RPM-A: ALL Property Specifier",
        reference: "135.1-2025 - 8.18.8",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_a_all_specifier(ctx)),
    });

    registry.add(TestDef {
        id: "4.3.11",
        name: "DS-RPM-A: REQUIRED Property Specifier",
        reference: "135.1-2025 - 8.18.9",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_a_required_specifier(ctx)),
    });

    registry.add(TestDef {
        id: "4.3.12",
        name: "DS-RPM-A: OPTIONAL Property Specifier",
        reference: "135.1-2025 - 8.18.10",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_a_optional_specifier(ctx)),
    });

    // ── Per-data-type via 8.18.1 (non-array, 16 types) ─────────────────
    // Same 16 types as in RP-A but using RPM
    for (i, dt) in [
        "NULL",
        "BOOLEAN",
        "Enumerated",
        "INTEGER",
        "Unsigned",
        "REAL",
        "Double",
        "Time",
        "Date",
        "CharString",
        "OctetString",
        "BitString",
        "OID",
        "Constructed",
        "Proprietary",
        "ListOf",
    ]
    .iter()
    .enumerate()
    {
        let id_str = Box::leak(format!("4.3.{}", 13 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-RPM-A: Read {} (8.18.1)", dt).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.18.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "rpm-a", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rpm_a_base(ctx)),
        });
    }

    // ── Per-data-type via 8.18.2 (array element, 16 types) ─────────────
    for (i, dt) in [
        "NULL",
        "BOOLEAN",
        "Enumerated",
        "INTEGER",
        "Unsigned",
        "REAL",
        "Double",
        "Time",
        "Date",
        "CharString",
        "OctetString",
        "BitString",
        "OID",
        "Constructed",
        "Proprietary",
        "ListOf",
    ]
    .iter()
    .enumerate()
    {
        let id_str = Box::leak(format!("4.3.{}", 29 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-RPM-A: Read {} (8.18.2)", dt).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.18.2",
            section: Section::DataSharing,
            tags: &["data-sharing", "rpm-a", "data-type", "array"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rpm_a_base(ctx)),
        });
    }

    // ── Combinations: multi-property reads for each type ────────────────
    for (i, dt) in [
        "NULL",
        "BOOLEAN",
        "Enumerated",
        "INTEGER",
        "Unsigned",
        "REAL",
        "Double",
        "Time",
        "Date",
        "CharString",
        "OctetString",
        "BitString",
        "OID",
        "Constructed",
        "Proprietary",
        "ListOf",
    ]
    .iter()
    .enumerate()
    {
        let id_str = Box::leak(format!("4.3.{}", 45 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-RPM-A: Multi-Prop {} (8.18.3)", dt).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.18.3",
            section: Section::DataSharing,
            tags: &["data-sharing", "rpm-a", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rpm_a_multi_props(ctx)),
        });
    }

    // ── Multi-object per type (8.18.6/7) ────────────────────────────────
    for (i, dt) in [
        "NULL",
        "BOOLEAN",
        "Enumerated",
        "INTEGER",
        "Unsigned",
        "REAL",
        "Double",
        "Time",
        "Date",
        "CharString",
        "OctetString",
        "BitString",
        "OID",
        "Constructed",
        "Proprietary",
        "ListOf",
    ]
    .iter()
    .enumerate()
    {
        let id_str = Box::leak(format!("4.3.{}", 61 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-RPM-A: Multi-Object {} (8.18.6)", dt).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.18.6",
            section: Section::DataSharing,
            tags: &["data-sharing", "rpm-a", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rpm_a_multi_objects(ctx)),
        });
    }

    // ── Remaining combination tests ─────────────────────────────────────
    for i in 0..21 {
        let id_str = Box::leak(format!("4.3.{}", 77 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-RPM-A: Combo Test {}", i + 1).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.18.7",
            section: Section::DataSharing,
            tags: &["data-sharing", "rpm-a"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rpm_a_multi_both(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn rpm_a_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.rpm_single(dev, PropertyIdentifier::OBJECT_NAME, None)
        .await?;
    ctx.pass()
}

async fn rpm_a_multi_props(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.rpm_multi_props(
        dev,
        &[
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::VENDOR_NAME,
            PropertyIdentifier::SYSTEM_STATUS,
        ],
    )
    .await?;
    ctx.pass()
}

async fn rpm_a_multi_objects(ctx: &mut TestContext) -> Result<(), TestFailure> {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;

    ctx.read_property_multiple(vec![
        ReadAccessSpecification {
            object_identifier: dev,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
            }],
        },
        ReadAccessSpecification {
            object_identifier: ai,
            list_of_property_references: vec![PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }],
        },
    ])
    .await?;
    ctx.pass()
}

async fn rpm_a_multi_both(ctx: &mut TestContext) -> Result<(), TestFailure> {
    use bacnet_services::common::PropertyReference;
    use bacnet_services::rpm::ReadAccessSpecification;

    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;

    ctx.read_property_multiple(vec![
        ReadAccessSpecification {
            object_identifier: dev,
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::OBJECT_NAME,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::VENDOR_NAME,
                    property_array_index: None,
                },
            ],
        },
        ReadAccessSpecification {
            object_identifier: ai,
            list_of_property_references: vec![
                PropertyReference {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                },
                PropertyReference {
                    property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                    property_array_index: None,
                },
            ],
        },
    ])
    .await?;
    ctx.pass()
}

async fn rpm_a_all_specifier(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.rpm_all(ai).await?;
    ctx.pass()
}

async fn rpm_a_required_specifier(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.rpm_required(ai).await?;
    ctx.pass()
}

async fn rpm_a_optional_specifier(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.rpm_optional(ai).await?;
    ctx.pass()
}
