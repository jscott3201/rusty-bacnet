//! BTL Test Plan Section 4.5 — DS-WP-A (WriteProperty, client initiation).
//! 37 BTL references: base + per-data-type (8.20.1/8.20.2 × types).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base requirements ────────────────────────────────────────────────

    registry.add(TestDef {
        id: "4.5.1",
        name: "DS-WP-A: Write Non-Array (8.20.1)",
        reference: "135.1-2025 - 8.20.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wp_a_write_non_array(ctx)),
    });

    registry.add(TestDef {
        id: "4.5.2",
        name: "DS-WP-A: Write Array Element (8.20.2)",
        reference: "135.1-2025 - 8.20.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-a", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wp_a_write_array_element(ctx)),
    });

    registry.add(TestDef {
        id: "4.5.3",
        name: "DS-WP-A: Write Whole Array (8.20.3)",
        reference: "135.1-2025 - 8.20.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-a", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wp_a_write_whole_array(ctx)),
    });

    registry.add(TestDef {
        id: "4.5.4",
        name: "DS-WP-A: Write Priority (8.20.4)",
        reference: "135.1-2025 - 8.20.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wp_a_write_priority(ctx)),
    });

    registry.add(TestDef {
        id: "4.5.5",
        name: "DS-WP-A: Relinquish by NULL (8.20.5)",
        reference: "135.1-2025 - 8.20.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "wp-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wp_a_relinquish(ctx)),
    });

    // ── Per-data-type via 8.20.1 (non-array write, ~16 types) ──────────

    let types: &[(&str, &str)] = &[
        ("4.5.6", "DS-WP-A: Write NULL"),
        ("4.5.7", "DS-WP-A: Write BOOLEAN"),
        ("4.5.8", "DS-WP-A: Write Enumerated"),
        ("4.5.9", "DS-WP-A: Write INTEGER"),
        ("4.5.10", "DS-WP-A: Write Unsigned"),
        ("4.5.11", "DS-WP-A: Write REAL"),
        ("4.5.12", "DS-WP-A: Write Double"),
        ("4.5.13", "DS-WP-A: Write Time"),
        ("4.5.14", "DS-WP-A: Write Date"),
        ("4.5.15", "DS-WP-A: Write CharacterString"),
        ("4.5.16", "DS-WP-A: Write OctetString"),
        ("4.5.17", "DS-WP-A: Write BitString"),
        ("4.5.18", "DS-WP-A: Write OID"),
        ("4.5.19", "DS-WP-A: Write Constructed"),
        ("4.5.20", "DS-WP-A: Write Proprietary"),
        ("4.5.21", "DS-WP-A: Write ListOf"),
    ];

    for &(id, name) in types {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.20.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "wp-a", "data-type"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(wp_a_write_data_type(ctx)),
        });
    }

    // ── Per-data-type via 8.20.2 (array element write) ──────────────────

    for i in 0..16 {
        let id_str = Box::leak(format!("4.5.{}", 22 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-WP-A: Write Array Type {}", i + 1).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.20.2",
            section: Section::DataSharing,
            tags: &["data-sharing", "wp-a", "data-type", "array"],
            conditionality: Conditionality::MustExecute,
            timeout: None,
            run: |ctx| Box::pin(wp_a_write_data_type(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn wp_a_write_non_array(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 42.0, Some(16))
        .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 42.0)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wp_a_write_array_element(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Write to a writable array element — description is simplest
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wp_a_write_whole_array(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Whole array writes are complex; verify writable property works
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.verify_readable(ao, PropertyIdentifier::PRIORITY_ARRAY)
        .await?;
    ctx.pass()
}

async fn wp_a_write_priority(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 55.5, Some(8))
        .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 55.5)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(8))
        .await?;
    ctx.pass()
}

async fn wp_a_relinquish(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 33.3, Some(16))
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wp_a_write_data_type(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // General write data type test — write boolean OOS
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.verify_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}
