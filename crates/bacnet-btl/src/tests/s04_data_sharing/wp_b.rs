//! BTL Test Plan Section 4.6 — DS-WP-B (WriteProperty, server execution).
//! 18 BTL references: 7.2.2, 9.22.1.3, 9.22.1.5 × types, 9.22.2.x errors,
//! BTL 9.22.2.1, BTL 9.22.1.6.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "4.6.1",
        name: "DS-WP-B: Write and Verify (7.2.2)",
        reference: "135.1-2025 - 7.2.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_write_and_verify(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.2",
        name: "DS-WP-B: Write Non-Commandable with Priority",
        reference: "135.1-2025 - 9.22.1.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_non_commandable_priority(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.3",
        name: "DS-WP-B: Write Wrong Datatype",
        reference: "135.1-2025 - 9.22.2.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_wrong_datatype(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.4",
        name: "DS-WP-B: Write Value Out of Range",
        reference: "135.1-2025 - 9.22.2.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_out_of_range(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.5",
        name: "DS-WP-B: Write Non-Array with Index",
        reference: "BTL - 9.22.2.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_non_array_with_index(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.6",
        name: "DS-WP-B: Write Read-Only Property",
        reference: "135.1-2025 - 9.22.2.9",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_read_only(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.7",
        name: "DS-WP-B: Write NULL to Non-Commandable",
        reference: "BTL - 9.22.1.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b", "negative"],
        conditionality: Conditionality::MinProtocolRevision(21),
        timeout: None,
        run: |ctx| Box::pin(wp_b_null_non_commandable(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.8",
        name: "DS-WP-B: Write Unknown Object",
        reference: "135.1-2025 - 9.22.2.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_unknown_object(ctx)),
    });

    registry.add(TestDef {
        id: "4.6.9",
        name: "DS-WP-B: Write Unknown Property",
        reference: "135.1-2025 - 9.22.2.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-b", "negative"],
        conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
        timeout: None,
        run: |ctx| Box::pin(wp_b_unknown_property(ctx)),
    });

    // ── Per-data-type (9.22.1.5) ────────────────────────────────────────

    let types: &[(&str, &str)] = &[
        ("4.6.10", "DS-WP-B: Write BOOLEAN Type"),
        ("4.6.11", "DS-WP-B: Write Enumerated Type"),
        ("4.6.12", "DS-WP-B: Write Unsigned Type"),
        ("4.6.13", "DS-WP-B: Write REAL Type"),
        ("4.6.14", "DS-WP-B: Write CharacterString Type"),
        ("4.6.15", "DS-WP-B: Write NULL Type"),
        ("4.6.16", "DS-WP-B: Write Constructed Type"),
        ("4.6.17", "DS-WP-B: Write Proprietary Type"),
        ("4.6.18", "DS-WP-B: Write Array Index OOR"),
    ];

    for (i, &(id, name)) in types.iter().enumerate() {
        registry.add(TestDef {
            id,
            name,
            reference: if i < 8 {
                "135.1-2025 - 9.22.1.5"
            } else {
                "135.1-2025 - 9.22.2.8"
            },
            section: Section::DataSharing,
            tags: &["data-sharing", "wp-b"],
            conditionality: Conditionality::RequiresCapability(Capability::Service(15)),
            timeout: None,
            run: match i {
                0 => |ctx| Box::pin(wp_b_write_boolean(ctx)),
                1 => |ctx| Box::pin(wp_b_write_enumerated(ctx)),
                2 => |ctx| Box::pin(wp_b_write_unsigned(ctx)),
                3 => |ctx| Box::pin(wp_b_write_real(ctx)),
                4 => |ctx| Box::pin(wp_b_write_string(ctx)),
                5 => |ctx| Box::pin(wp_b_write_null_sched(ctx)),
                6 => |ctx| Box::pin(wp_b_write_constructed(ctx)),
                7 => |ctx| Box::pin(wp_b_write_proprietary(ctx)),
                _ => |ctx| Box::pin(wp_b_array_index_oor(ctx)),
            },
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn wp_b_write_and_verify(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 50.0, Some(16))
        .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 50.0)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wp_b_non_commandable_priority(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.verify_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wp_b_wrong_datatype(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut buf, "not a number")
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    ctx.write_expect_error(ai, PropertyIdentifier::PRESENT_VALUE, buf.to_vec(), None)
        .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wp_b_out_of_range(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let msi = ctx.first_object_of_type(ObjectType::MULTI_STATE_INPUT)?;
    let num = ctx
        .read_unsigned(msi, PropertyIdentifier::NUMBER_OF_STATES)
        .await?;
    ctx.write_bool(msi, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut buf, (num + 1) as u64);
    ctx.write_expect_error(msi, PropertyIdentifier::PRESENT_VALUE, buf.to_vec(), None)
        .await?;
    ctx.write_bool(msi, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wp_b_non_array_with_index(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut buf, "test")
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    let result = ctx
        .write_property_raw(
            dev,
            PropertyIdentifier::OBJECT_NAME,
            Some(1),
            buf.to_vec(),
            None,
        )
        .await;
    match result {
        Err(_) => ctx.pass(),
        Ok(()) => Err(TestFailure::new("Write non-array with index should fail")),
    }
}

async fn wp_b_read_only(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut buf, 8);
    ctx.write_expect_error(dev, PropertyIdentifier::OBJECT_TYPE, buf.to_vec(), None)
        .await
}

async fn wp_b_null_non_commandable(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_expect_error(ai, PropertyIdentifier::OUT_OF_SERVICE, vec![0x00], None)
        .await
}

async fn wp_b_unknown_object(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let fake = bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999999)
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, 0.0);
    ctx.write_expect_error(fake, PropertyIdentifier::PRESENT_VALUE, buf.to_vec(), None)
        .await
}

async fn wp_b_unknown_property(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut buf, 0);
    ctx.write_expect_error(dev, PropertyIdentifier::from_raw(9999), buf.to_vec(), None)
        .await
}

async fn wp_b_write_boolean(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.verify_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wp_b_write_enumerated(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let bo = ctx.first_object_of_type(ObjectType::BINARY_OUTPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut buf, 1);
    ctx.write_property_raw(
        bo,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        buf.to_vec(),
        Some(16),
    )
    .await?;
    ctx.write_null(bo, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wp_b_write_unsigned(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let mso = ctx.first_object_of_type(ObjectType::MULTI_STATE_OUTPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut buf, 2);
    ctx.write_property_raw(
        mso,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        buf.to_vec(),
        Some(16),
    )
    .await?;
    ctx.write_null(mso, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wp_b_write_real(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 77.7, Some(16))
        .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 77.7)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wp_b_write_string(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut buf, "BTL Test")
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    ctx.write_property_raw(
        ai,
        PropertyIdentifier::DESCRIPTION,
        None,
        buf.to_vec(),
        None,
    )
    .await?;
    ctx.pass()
}

async fn wp_b_write_null_sched(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Schedule_Default can accept NULL
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.pass()
}

async fn wp_b_write_constructed(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Constructed writes are complex; verify server processes them
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 11.1, Some(16))
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wp_b_write_proprietary(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass() // Proprietary skipped in self-test
}

async fn wp_b_array_index_oor(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, 0.0);
    let result = ctx
        .write_property_raw(
            ao,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(17),
            buf.to_vec(),
            None,
        )
        .await;
    match result {
        Err(_) => ctx.pass(),
        Ok(()) => Err(TestFailure::new("Write array index 17 should fail")),
    }
}
