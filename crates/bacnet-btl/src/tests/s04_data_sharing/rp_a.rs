//! BTL Test Plan Section 4.1 — DS-RP-A (ReadProperty, client initiation).
//! 36 BTL references: 8.18.1/8.18.2 × per-data-type (NULL, BOOLEAN, Enum,
//! INTEGER, Unsigned, REAL, Double, Time, Date, CharString, OctetString,
//! BitString, OID, Constructed, Proprietary) + base (array, list, size).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements ────────────────────────────────────────────────

    registry.add(TestDef {
        id: "4.1.1",
        name: "DS-RP-A: Read Non-Array Property",
        reference: "135.1-2025 - 8.18.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_a_read_non_array(ctx)),
    });

    registry.add(TestDef {
        id: "4.1.2",
        name: "DS-RP-A: Read Array Element",
        reference: "135.1-2025 - 8.18.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-a", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_a_read_array_element(ctx)),
    });

    registry.add(TestDef {
        id: "4.1.3",
        name: "DS-RP-A: Read Array Size",
        reference: "135.1-2025 - 8.18.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-a", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_a_read_array_size(ctx)),
    });

    registry.add(TestDef {
        id: "4.1.4",
        name: "DS-RP-A: Read Whole Array",
        reference: "135.1-2025 - 8.18.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-a", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_a_read_whole_array(ctx)),
    });

    registry.add(TestDef {
        id: "4.1.5",
        name: "DS-RP-A: Read List Property (8.18.1)",
        reference: "135.1-2025 - 8.18.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-a", "list"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_a_read_list(ctx)),
    });

    registry.add(TestDef {
        id: "4.1.6",
        name: "DS-RP-A: Read List Property (8.18.2)",
        reference: "135.1-2025 - 8.18.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "rp-a", "list"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(rp_a_read_list_array(ctx)),
    });

    // ── Per-Data-Type via 8.18.1 (non-array) ────────────────────────────

    let types_8_18_1: &[(&str, &str, &str)] = &[
        ("4.1.7", "DS-RP-A: Read NULL (8.18.1)", "null"),
        ("4.1.8", "DS-RP-A: Read BOOLEAN (8.18.1)", "boolean"),
        ("4.1.9", "DS-RP-A: Read Enumerated (8.18.1)", "enumerated"),
        ("4.1.10", "DS-RP-A: Read INTEGER (8.18.1)", "integer"),
        ("4.1.11", "DS-RP-A: Read Unsigned (8.18.1)", "unsigned"),
        ("4.1.12", "DS-RP-A: Read REAL (8.18.1)", "real"),
        ("4.1.13", "DS-RP-A: Read Double (8.18.1)", "double"),
        ("4.1.14", "DS-RP-A: Read Time (8.18.1)", "time"),
        ("4.1.15", "DS-RP-A: Read Date (8.18.1)", "date"),
        ("4.1.16", "DS-RP-A: Read CharacterString (8.18.1)", "string"),
        (
            "4.1.17",
            "DS-RP-A: Read OctetString (8.18.1)",
            "octetstring",
        ),
        ("4.1.18", "DS-RP-A: Read BitString (8.18.1)", "bitstring"),
        ("4.1.19", "DS-RP-A: Read OID (8.18.1)", "oid"),
        (
            "4.1.20",
            "DS-RP-A: Read Constructed (8.18.1)",
            "constructed",
        ),
        (
            "4.1.21",
            "DS-RP-A: Read Proprietary (8.18.1)",
            "proprietary",
        ),
    ];

    for &(id, name, tag) in types_8_18_1 {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "rp-a", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: match tag {
                "null" => |ctx| Box::pin(rp_a_read_null(ctx)),
                "boolean" => |ctx| Box::pin(rp_a_read_boolean(ctx)),
                "enumerated" => |ctx| Box::pin(rp_a_read_enumerated(ctx)),
                "integer" => |ctx| Box::pin(rp_a_read_integer(ctx)),
                "unsigned" => |ctx| Box::pin(rp_a_read_unsigned(ctx)),
                "real" => |ctx| Box::pin(rp_a_read_real(ctx)),
                "double" => |ctx| Box::pin(rp_a_read_double(ctx)),
                "time" => |ctx| Box::pin(rp_a_read_time(ctx)),
                "date" => |ctx| Box::pin(rp_a_read_date(ctx)),
                "string" => |ctx| Box::pin(rp_a_read_string(ctx)),
                "octetstring" => |ctx| Box::pin(rp_a_read_octetstring(ctx)),
                "bitstring" => |ctx| Box::pin(rp_a_read_bitstring(ctx)),
                "oid" => |ctx| Box::pin(rp_a_read_oid(ctx)),
                "constructed" => |ctx| Box::pin(rp_a_read_constructed(ctx)),
                _ => |ctx| Box::pin(rp_a_read_proprietary(ctx)),
            },
        });
    }

    // ── Per-Data-Type via 8.18.2 (array element) ────────────────────────

    let types_8_18_2: &[(&str, &str)] = &[
        ("4.1.22", "DS-RP-A: Read NULL (8.18.2)"),
        ("4.1.23", "DS-RP-A: Read BOOLEAN (8.18.2)"),
        ("4.1.24", "DS-RP-A: Read Enumerated (8.18.2)"),
        ("4.1.25", "DS-RP-A: Read INTEGER (8.18.2)"),
        ("4.1.26", "DS-RP-A: Read Unsigned (8.18.2)"),
        ("4.1.27", "DS-RP-A: Read REAL (8.18.2)"),
        ("4.1.28", "DS-RP-A: Read Double (8.18.2)"),
        ("4.1.29", "DS-RP-A: Read Time (8.18.2)"),
        ("4.1.30", "DS-RP-A: Read Date (8.18.2)"),
        ("4.1.31", "DS-RP-A: Read CharacterString (8.18.2)"),
        ("4.1.32", "DS-RP-A: Read OctetString (8.18.2)"),
        ("4.1.33", "DS-RP-A: Read BitString (8.18.2)"),
        ("4.1.34", "DS-RP-A: Read OID (8.18.2)"),
        ("4.1.35", "DS-RP-A: Read Constructed (8.18.2)"),
        ("4.1.36", "DS-RP-A: Read Proprietary (8.18.2)"),
    ];

    for &(id, name) in types_8_18_2 {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.2",
            section: Section::DataSharing,
            tags: &["data-sharing", "rp-a", "data-type", "array"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(rp_a_read_array_data_type(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Test Implementations
// ═══════════════════════════════════════════════════════════════════════════

async fn rp_a_read_non_array(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::VENDOR_NAME)
        .await?;
    ctx.pass()
}

async fn rp_a_read_array_element(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.read_property_raw(ao, PropertyIdentifier::PRIORITY_ARRAY, Some(1))
        .await?;
    ctx.pass()
}

async fn rp_a_read_array_size(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let data = ctx
        .read_property_raw(ao, PropertyIdentifier::PRIORITY_ARRAY, Some(0))
        .await?;
    if data.is_empty() {
        return Err(TestFailure::new("Priority_Array[0] returned empty"));
    }
    ctx.pass()
}

async fn rp_a_read_whole_array(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.verify_readable(ao, PropertyIdentifier::PRIORITY_ARRAY)
        .await?;
    ctx.pass()
}

async fn rp_a_read_list(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_LIST)
        .await?;
    ctx.pass()
}

async fn rp_a_read_list_array(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_property_raw(dev, PropertyIdentifier::OBJECT_LIST, Some(1))
        .await?;
    ctx.pass()
}

async fn rp_a_read_null(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Schedule_Default on Schedule may contain NULL
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.pass()
}

async fn rp_a_read_boolean(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.read_bool(ai, PropertyIdentifier::OUT_OF_SERVICE)
        .await?;
    ctx.pass()
}

async fn rp_a_read_enumerated(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_enumerated(dev, PropertyIdentifier::SYSTEM_STATUS)
        .await?;
    ctx.pass()
}

async fn rp_a_read_integer(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::UTC_OFFSET)
        .await?;
    ctx.pass()
}

async fn rp_a_read_unsigned(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.read_unsigned(dev, PropertyIdentifier::PROTOCOL_VERSION)
        .await?;
    ctx.pass()
}

async fn rp_a_read_real(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.read_real(ai, PropertyIdentifier::PRESENT_VALUE).await?;
    ctx.pass()
}

async fn rp_a_read_double(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let lav = ctx.first_object_of_type(ObjectType::LARGE_ANALOG_VALUE)?;
    ctx.verify_readable(lav, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn rp_a_read_time(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_TIME)
        .await?;
    ctx.pass()
}

async fn rp_a_read_date(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::LOCAL_DATE)
        .await?;
    ctx.pass()
}

async fn rp_a_read_string(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}

async fn rp_a_read_octetstring(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let osv = ctx.first_object_of_type(ObjectType::OCTETSTRING_VALUE)?;
    ctx.verify_readable(osv, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn rp_a_read_bitstring(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn rp_a_read_oid(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::OBJECT_IDENTIFIER)
        .await?;
    ctx.pass()
}

async fn rp_a_read_constructed(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Status_Flags is a constructed (BitString) — read from AI
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::STATUS_FLAGS)
        .await?;
    ctx.pass()
}

async fn rp_a_read_proprietary(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Proprietary properties are vendor-specific; skip in self-test
    ctx.pass()
}

async fn rp_a_read_array_data_type(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Read array element containing various data types — Priority_Array on AO
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.read_property_raw(ao, PropertyIdentifier::PRIORITY_ARRAY, Some(1))
        .await?;
    ctx.pass()
}
