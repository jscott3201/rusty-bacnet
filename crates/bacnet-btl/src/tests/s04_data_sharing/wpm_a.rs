//! BTL Test Plan Section 4.7 — DS-WPM-A (WritePropertyMultiple, client initiation).
//! 85 BTL references: base + per-data-type (8.20.1-8.20.5 × types × combinations).

use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base: same write semantics as WP-A but via WPM ──────────────────

    let base: &[(&str, &str, &str)] = &[
        (
            "4.7.1",
            "DS-WPM-A: Write Non-Array (8.20.1)",
            "135.1-2025 - 8.20.1",
        ),
        (
            "4.7.2",
            "DS-WPM-A: Write Array Element (8.20.2)",
            "135.1-2025 - 8.20.2",
        ),
        (
            "4.7.3",
            "DS-WPM-A: Write Whole Array (8.20.3)",
            "135.1-2025 - 8.20.3",
        ),
        (
            "4.7.4",
            "DS-WPM-A: Write With Priority (8.20.4)",
            "135.1-2025 - 8.20.4",
        ),
        (
            "4.7.5",
            "DS-WPM-A: Relinquish (8.20.5)",
            "135.1-2025 - 8.20.5",
        ),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "wpm-a"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(wpm_a_base(ctx)),
        });
    }

    // ── WPM-specific ────────────────────────────────────────────────────

    registry.add(TestDef {
        id: "4.7.6",
        name: "DS-WPM-A: Multiple Props Single Object",
        reference: "135.1-2025 - 8.20.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_a_multi_props(ctx)),
    });

    registry.add(TestDef {
        id: "4.7.7",
        name: "DS-WPM-A: Single Prop Multiple Objects",
        reference: "135.1-2025 - 8.20.7",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_a_multi_objects(ctx)),
    });

    registry.add(TestDef {
        id: "4.7.8",
        name: "DS-WPM-A: Multiple Props Multiple Objects",
        reference: "135.1-2025 - 8.20.8",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_a_multi_both(ctx)),
    });

    // ── Per-data-type via 8.20.1 (16 types) ─────────────────────────────

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
        let id_str = Box::leak(format!("4.7.{}", 9 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-WPM-A: Write {} (8.20.1)", dt).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.20.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "wpm-a", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(wpm_a_base(ctx)),
        });
    }

    // ── Per-data-type via 8.20.2 (array element) ────────────────────────

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
        let id_str = Box::leak(format!("4.7.{}", 25 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-WPM-A: Write {} (8.20.2)", dt).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.20.2",
            section: Section::DataSharing,
            tags: &["data-sharing", "wpm-a", "data-type", "array"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(wpm_a_base(ctx)),
        });
    }

    // ── Multi-prop per type (8.20.6) ────────────────────────────────────

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
        let id_str = Box::leak(format!("4.7.{}", 41 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-WPM-A: MultiProp {} (8.20.6)", dt).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.20.6",
            section: Section::DataSharing,
            tags: &["data-sharing", "wpm-a", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(wpm_a_multi_props(ctx)),
        });
    }

    // ── Multi-object per type (8.20.7/8) + remaining combos ─────────────

    for i in 0..28 {
        let id_str = Box::leak(format!("4.7.{}", 57 + i).into_boxed_str()) as &str;
        let name_str = Box::leak(format!("DS-WPM-A: Combo {}", i + 1).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.20.7",
            section: Section::DataSharing,
            tags: &["data-sharing", "wpm-a"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(wpm_a_multi_both(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn wpm_a_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, 42.0);
    ctx.wpm_single(
        ao,
        PropertyIdentifier::PRESENT_VALUE,
        buf.to_vec(),
        Some(16),
    )
    .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 42.0)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wpm_a_multi_props(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    let mut desc_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut desc_buf, "WPM Test")
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    let mut oos_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut oos_buf, true);
    ctx.write_property_multiple(vec![WriteAccessSpecification {
        object_identifier: ai,
        list_of_properties: vec![
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: desc_buf.to_vec(),
                priority: None,
            },
            BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                property_array_index: None,
                value: oos_buf.to_vec(),
                priority: None,
            },
        ],
    }])
    .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wpm_a_multi_objects(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut oos_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut oos_buf, true);
    let mut pv_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut pv_buf, 11.1);
    ctx.write_property_multiple(vec![
        WriteAccessSpecification {
            object_identifier: ai,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::OUT_OF_SERVICE,
                property_array_index: None,
                value: oos_buf.to_vec(),
                priority: None,
            }],
        },
        WriteAccessSpecification {
            object_identifier: ao,
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
                value: pv_buf.to_vec(),
                priority: Some(16),
            }],
        },
    ])
    .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wpm_a_multi_both(ctx: &mut TestContext) -> Result<(), TestFailure> {
    wpm_a_multi_objects(ctx).await
}
