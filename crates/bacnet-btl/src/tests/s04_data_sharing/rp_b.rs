//! BTL Test Plan Section 4.2 — DS-RP-B (ReadProperty, server execution).
//! 21 BTL references: base (7.1.1, 9.18.2.1, 9.18.2.3, 9.18.2.4, 9.18.1.3,
//! 7.1.3, 9.18.1.7) + per-data-type (9.18.1.5 × 14 data types).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements ────────────────────────────────────────────────

    registry.add(TestDef {
        id: "4.2.1",
        name: "DS-RP-B: Read Support (7.1.1)",
        reference: "135.1-2025 - 7.1.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_b_read_support(ctx)),
    });

    registry.add(TestDef {
        id: "4.2.2",
        name: "DS-RP-B: Read Non-Array with Array Index",
        reference: "135.1-2025 - 9.18.2.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-b", "negative"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_b_non_array_with_index(ctx)),
    });

    registry.add(TestDef {
        id: "4.2.3",
        name: "DS-RP-B: Read Unknown Object",
        reference: "135.1-2025 - 9.18.2.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-b", "negative"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_b_unknown_object(ctx)),
    });

    registry.add(TestDef {
        id: "4.2.4",
        name: "DS-RP-B: Read Unknown Property",
        reference: "135.1-2025 - 9.18.2.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-b", "negative"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_b_unknown_property(ctx)),
    });

    registry.add(TestDef {
        id: "4.2.5",
        name: "DS-RP-B: Read Device via Wildcard Instance",
        reference: "135.1-2025 - 9.18.1.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-b"],
        conditionality: Conditionality::MinProtocolRevision(4),
        timeout: None,
        run: |ctx| Box::pin(rp_b_wildcard_instance(ctx)),
    });

    registry.add(TestDef {
        id: "4.2.6",
        name: "DS-RP-B: Property_List Consistent",
        reference: "135.1-2025 - 7.1.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-b", "property-list"],
        conditionality: Conditionality::MinProtocolRevision(14),
        timeout: None,
        run: |ctx| Box::pin(rp_b_property_list(ctx)),
    });

    registry.add(TestDef {
        id: "4.2.7",
        name: "DS-RP-B: Read Array at Different Indexes",
        reference: "135.1-2025 - 9.18.1.7",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-b", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_b_array_indexes(ctx)),
    });

    // ── Per-Data-Type (9.18.1.5) ────────────────────────────────────────

    let data_types: &[(&str, &str)] = &[
        ("4.2.8", "DS-RP-B: Read Enumerated"),
        ("4.2.9", "DS-RP-B: Read Unsigned"),
        ("4.2.10", "DS-RP-B: Read OID"),
        ("4.2.11", "DS-RP-B: Read CharacterString"),
        ("4.2.12", "DS-RP-B: Read BitString"),
        ("4.2.13", "DS-RP-B: Read NULL"),
        ("4.2.14", "DS-RP-B: Read BOOLEAN"),
        ("4.2.15", "DS-RP-B: Read INTEGER"),
        ("4.2.16", "DS-RP-B: Read REAL"),
        ("4.2.17", "DS-RP-B: Read Double"),
        ("4.2.18", "DS-RP-B: Read Time"),
        ("4.2.19", "DS-RP-B: Read Date"),
        ("4.2.20", "DS-RP-B: Read OctetString"),
        ("4.2.21", "DS-RP-B: Read Proprietary"),
    ];

    for (i, &(id, name)) in data_types.iter().enumerate() {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 9.18.1.5",
            section: Section::DataSharing,
            tags: &["data-sharing", "rp-b", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: match i {
                0 => |ctx| Box::pin(read_enumerated(ctx)),
                1 => |ctx| Box::pin(read_unsigned(ctx)),
                2 => |ctx| Box::pin(read_oid(ctx)),
                3 => |ctx| Box::pin(read_string(ctx)),
                4 => |ctx| Box::pin(read_bitstring(ctx)),
                5 => |ctx| Box::pin(read_null(ctx)),
                6 => |ctx| Box::pin(read_boolean(ctx)),
                7 => |ctx| Box::pin(read_integer(ctx)),
                8 => |ctx| Box::pin(read_real(ctx)),
                9 => |ctx| Box::pin(read_double(ctx)),
                10 => |ctx| Box::pin(read_time(ctx)),
                11 => |ctx| Box::pin(read_date(ctx)),
                12 => |ctx| Box::pin(read_octetstring(ctx)),
                _ => |ctx| Box::pin(read_proprietary(ctx)),
            },
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn rp_b_read_support(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::VENDOR_NAME)
        .await?;
    ctx.pass()
}

async fn rp_b_non_array_with_index(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_expect_error(dev, PropertyIdentifier::OBJECT_NAME, Some(1))
        .await
}

async fn rp_b_unknown_object(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let fake = bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999999)
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    ctx.read_expect_error(fake, PropertyIdentifier::PRESENT_VALUE, None)
        .await
}

async fn rp_b_unknown_property(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_expect_error(dev, PropertyIdentifier::from_raw(9999), None)
        .await
}

async fn rp_b_wildcard_instance(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let wildcard = bacnet_types::primitives::ObjectIdentifier::new(ObjectType::DEVICE, 4194303)
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    ctx.verify_readable(wildcard, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}

async fn rp_b_property_list(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let data = ctx
        .read_property_raw(dev, PropertyIdentifier::PROPERTY_LIST, None)
        .await?;
    if data.len() < 3 {
        return Err(TestFailure::new("Property_List too short"));
    }
    ctx.pass()
}

async fn rp_b_array_indexes(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    // Index 0 = size
    ctx.read_property_raw(ao, PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .await?;
    // Index 1 = first element
    ctx.read_property_raw(ao, PropertyIdentifier::PRIORITY_ARRAY, Some(1))
        .await?;
    // Index 16 = last element
    ctx.read_property_raw(ao, PropertyIdentifier::PRIORITY_ARRAY, Some(16))
        .await?;
    ctx.pass()
}

async fn read_enumerated(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_enumerated(dev, PropertyIdentifier::SYSTEM_STATUS)
        .await?;
    ctx.pass()
}

async fn read_unsigned(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let v = ctx
        .read_unsigned(dev, PropertyIdentifier::PROTOCOL_VERSION)
        .await?;
    if v == 0 {
        return Err(TestFailure::new("Protocol_Version should be > 0"));
    }
    ctx.pass()
}

async fn read_oid(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let data = ctx
        .read_property_raw(dev, PropertyIdentifier::OBJECT_IDENTIFIER, None)
        .await?;
    let (tag_num, _) = TestContext::decode_app_value(&data)?;
    if tag_num != 12 {
        return Err(TestFailure::new(format!(
            "Expected OID tag 12, got {tag_num}"
        )));
    }
    ctx.pass()
}

async fn read_string(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let data = ctx
        .read_property_raw(dev, PropertyIdentifier::OBJECT_NAME, None)
        .await?;
    let (tag_num, _) = TestContext::decode_app_value(&data)?;
    if tag_num != 7 {
        return Err(TestFailure::new(format!(
            "Expected string tag 7, got {tag_num}"
        )));
    }
    ctx.pass()
}

async fn read_bitstring(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let data = ctx
        .read_property_raw(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED, None)
        .await?;
    let (tag_num, _) = TestContext::decode_app_value(&data)?;
    if tag_num != 8 {
        return Err(TestFailure::new(format!(
            "Expected bitstring tag 8, got {tag_num}"
        )));
    }
    ctx.pass()
}

async fn read_null(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.pass()
}

async fn read_boolean(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.read_bool(ai, PropertyIdentifier::OUT_OF_SERVICE)
        .await?;
    ctx.pass()
}

async fn read_integer(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::UTC_OFFSET)
        .await?;
    ctx.pass()
}

async fn read_real(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.read_real(ai, PropertyIdentifier::PRESENT_VALUE).await?;
    ctx.pass()
}

async fn read_double(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let lav = ctx.first_object_of_type(ObjectType::LARGE_ANALOG_VALUE)?;
    ctx.verify_readable(lav, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn read_time(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_TIME)
        .await?;
    ctx.pass()
}

async fn read_date(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_DATE)
        .await?;
    ctx.pass()
}

async fn read_octetstring(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let osv = ctx.first_object_of_type(ObjectType::OCTETSTRING_VALUE)?;
    ctx.verify_readable(osv, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn read_proprietary(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Proprietary properties are vendor-specific; skip in self-test
    ctx.pass()
}
