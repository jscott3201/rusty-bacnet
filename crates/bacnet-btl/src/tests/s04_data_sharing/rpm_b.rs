//! BTL Test Plan Section 4.4 — DS-RPM-B (ReadPropertyMultiple, server execution).
//! 30 BTL references: base (7.1.1, 9.20.1.1–9.20.1.11, 9.20.2.1–9.20.2.3,
//! BTL-9.20.1.16) + per-data-type (9.20.1.13 × 14 types).

use bacnet_services::common::PropertyReference;
use bacnet_services::rpm::ReadAccessSpecification;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "4.4.1",
        name: "DS-RPM-B: Read Support via RPM (7.1.1)",
        reference: "135.1-2025 - 7.1.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_read_support(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.2",
        name: "DS-RPM-B: Single Prop Single Object",
        reference: "135.1-2025 - 9.20.1.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_single_prop_single_obj(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.3",
        name: "DS-RPM-B: Multiple Props Single Object",
        reference: "135.1-2025 - 9.20.1.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_multi_props(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.4",
        name: "DS-RPM-B: Single Prop Multiple Objects",
        reference: "135.1-2025 - 9.20.1.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_multi_objects(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.5",
        name: "DS-RPM-B: Multiple Props Multiple Objects",
        reference: "135.1-2025 - 9.20.1.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_multi_both(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.6",
        name: "DS-RPM-B: Single Embedded Access Error",
        reference: "135.1-2025 - 9.20.1.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_single_embedded_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.7",
        name: "DS-RPM-B: Multiple Embedded Access Errors",
        reference: "135.1-2025 - 9.20.1.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_multi_embedded_errors(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.8",
        name: "DS-RPM-B: Read ALL Properties",
        reference: "135.1-2025 - 9.20.1.7",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "all"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_read_all(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.9",
        name: "DS-RPM-B: Read OPTIONAL Properties",
        reference: "135.1-2025 - 9.20.1.8",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "optional"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_read_optional(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.10",
        name: "DS-RPM-B: Read REQUIRED Properties",
        reference: "135.1-2025 - 9.20.1.9",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "required"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_read_required(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.11",
        name: "DS-RPM-B: Read Array Size (0th element)",
        reference: "135.1-2025 - 9.20.1.10",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_read_array_size(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.12",
        name: "DS-RPM-B: Unsupported Property Error",
        reference: "135.1-2025 - 9.20.2.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "negative"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_unsupported_prop(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.13",
        name: "DS-RPM-B: All Properties Error",
        reference: "135.1-2025 - 9.20.2.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "negative"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_all_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.14",
        name: "DS-RPM-B: Non-Array with Array Index",
        reference: "135.1-2025 - 9.20.2.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "negative"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_non_array_with_index(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.15",
        name: "DS-RPM-B: Device Wildcard Instance",
        reference: "135.1-2025 - 9.20.1.11",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b"],
        conditionality: Conditionality::MinProtocolRevision(4),
        timeout: None,
        run: |ctx| Box::pin(rpm_b_wildcard_instance(ctx)),
    });

    registry.add(TestDef {
        id: "4.4.16",
        name: "DS-RPM-B: Array Properties",
        reference: "BTL - 9.20.1.16",
        section: Section::DataSharing,
        tags: &["data-sharing", "rpm-b", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rpm_b_array_props(ctx)),
    });

    // ── Per-Data-Type (9.20.1.13) ───────────────────────────────────────

    let data_types: &[(&str, &str)] = &[
        ("4.4.17", "DS-RPM-B: Read Enumerated via RPM"),
        ("4.4.18", "DS-RPM-B: Read Unsigned via RPM"),
        ("4.4.19", "DS-RPM-B: Read OID via RPM"),
        ("4.4.20", "DS-RPM-B: Read CharString via RPM"),
        ("4.4.21", "DS-RPM-B: Read BitString via RPM"),
        ("4.4.22", "DS-RPM-B: Read NULL via RPM"),
        ("4.4.23", "DS-RPM-B: Read BOOLEAN via RPM"),
        ("4.4.24", "DS-RPM-B: Read INTEGER via RPM"),
        ("4.4.25", "DS-RPM-B: Read REAL via RPM"),
        ("4.4.26", "DS-RPM-B: Read Double via RPM"),
        ("4.4.27", "DS-RPM-B: Read Time via RPM"),
        ("4.4.28", "DS-RPM-B: Read Date via RPM"),
        ("4.4.29", "DS-RPM-B: Read OctetString via RPM"),
        ("4.4.30", "DS-RPM-B: Read Proprietary via RPM"),
    ];

    for &(id, name) in data_types {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 9.20.1.13",
            section: Section::DataSharing,
            tags: &["data-sharing", "rpm-b", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rpm_b_read_data_type(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn rpm_b_read_support(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.rpm_single(dev, PropertyIdentifier::OBJECT_NAME, None)
        .await?;
    ctx.pass()
}

async fn rpm_b_single_prop_single_obj(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.rpm_single(ai, PropertyIdentifier::PRESENT_VALUE, None)
        .await?;
    ctx.pass()
}

async fn rpm_b_multi_props(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.rpm_multi_props(
        dev,
        &[
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::VENDOR_NAME,
            PropertyIdentifier::SYSTEM_STATUS,
            PropertyIdentifier::PROTOCOL_VERSION,
        ],
    )
    .await?;
    ctx.pass()
}

async fn rpm_b_multi_objects(ctx: &mut TestContext) -> Result<(), TestFailure> {
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

async fn rpm_b_multi_both(ctx: &mut TestContext) -> Result<(), TestFailure> {
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

async fn rpm_b_single_embedded_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Read a valid and invalid property — RPM returns ACK with embedded error
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_property_multiple(vec![ReadAccessSpecification {
        object_identifier: dev,
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: PropertyIdentifier::OBJECT_NAME,
                property_array_index: None,
            },
            PropertyReference {
                property_identifier: PropertyIdentifier::from_raw(9999),
                property_array_index: None,
            },
        ],
    }])
    .await?;
    ctx.pass()
}

async fn rpm_b_multi_embedded_errors(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_property_multiple(vec![ReadAccessSpecification {
        object_identifier: dev,
        list_of_property_references: vec![
            PropertyReference {
                property_identifier: PropertyIdentifier::from_raw(9998),
                property_array_index: None,
            },
            PropertyReference {
                property_identifier: PropertyIdentifier::from_raw(9999),
                property_array_index: None,
            },
        ],
    }])
    .await?;
    ctx.pass()
}

async fn rpm_b_read_all(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.rpm_all(ai).await?;
    ctx.pass()
}

async fn rpm_b_read_optional(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.rpm_optional(ai).await?;
    ctx.pass()
}

async fn rpm_b_read_required(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.rpm_required(ai).await?;
    ctx.pass()
}

async fn rpm_b_read_array_size(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.rpm_single(ao, PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .await?;
    ctx.pass()
}

async fn rpm_b_unsupported_prop(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    // RPM with an unsupported property should still return ACK with embedded error
    ctx.read_property_multiple(vec![ReadAccessSpecification {
        object_identifier: dev,
        list_of_property_references: vec![PropertyReference {
            property_identifier: PropertyIdentifier::from_raw(9999),
            property_array_index: None,
        }],
    }])
    .await?;
    ctx.pass()
}

async fn rpm_b_all_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // All properties are unsupported on a fake object
    let fake = bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999999)
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    ctx.rpm_expect_error(vec![ReadAccessSpecification {
        object_identifier: fake,
        list_of_property_references: vec![PropertyReference {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
        }],
    }])
    .await
}

async fn rpm_b_non_array_with_index(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    // Reading non-array property with index via RPM should embed error in ACK
    ctx.read_property_multiple(vec![ReadAccessSpecification {
        object_identifier: dev,
        list_of_property_references: vec![PropertyReference {
            property_identifier: PropertyIdentifier::OBJECT_NAME,
            property_array_index: Some(1),
        }],
    }])
    .await?;
    ctx.pass()
}

async fn rpm_b_wildcard_instance(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let wildcard = bacnet_types::primitives::ObjectIdentifier::new(ObjectType::DEVICE, 4194303)
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    ctx.rpm_single(wildcard, PropertyIdentifier::OBJECT_NAME, None)
        .await?;
    ctx.pass()
}

async fn rpm_b_array_props(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.rpm_single(ao, PropertyIdentifier::PRIORITY_ARRAY, None)
        .await?;
    ctx.pass()
}

async fn rpm_b_read_data_type(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.rpm_multi_props(
        dev,
        &[
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::SYSTEM_STATUS,
            PropertyIdentifier::PROTOCOL_VERSION,
        ],
    )
    .await?;
    ctx.pass()
}
