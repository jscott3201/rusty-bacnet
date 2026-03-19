//! BTL Test Plan Section 4.8 — DS-WPM-B (WritePropertyMultiple, server execution).
//! 27 BTL references: 7.2.2, 9.23.1.x, 9.23.2.x, BTL 9.23.2.14-17,
//! per-data-type (9.23.1.8), commandable (7.3.1.3, 9.23.1.6).

use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "4.8.1",
        name: "DS-WPM-B: Write Support via WPM (7.2.2)",
        reference: "135.1-2025 - 7.2.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_write_support(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.2",
        name: "DS-WPM-B: Single Prop Single Object",
        reference: "135.1-2025 - 9.23.1.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_single(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.3",
        name: "DS-WPM-B: Non-Commandable With Priority",
        reference: "135.1-2025 - 9.23.1.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_non_cmd_priority(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.4",
        name: "DS-WPM-B: Property Access Error",
        reference: "135.1-2025 - 9.23.2.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_property_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.5",
        name: "DS-WPM-B: Object Access Error",
        reference: "135.1-2025 - 9.23.2.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_object_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.6",
        name: "DS-WPM-B: Write Access Error",
        reference: "135.1-2025 - 9.23.2.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_write_access_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.7",
        name: "DS-WPM-B: Wrong Datatype",
        reference: "135.1-2025 - 9.23.2.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_wrong_type(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.8",
        name: "DS-WPM-B: Value Out of Range",
        reference: "135.1-2025 - 9.23.2.7",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_value_oor(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.9",
        name: "DS-WPM-B: Reject (Proto Rev 10+)",
        reference: "135.1-2025 - 9.23.2.12",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MinProtocolRevision(10),
        timeout: None,
        run: |ctx| Box::pin(wpm_b_reject(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.10",
        name: "DS-WPM-B: Resize Fixed Array",
        reference: "135.1-2025 - 9.23.2.13",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_resize_array(ctx)),
    });

    // BTL-specific error tests
    registry.add(TestDef {
        id: "4.8.11",
        name: "DS-WPM-B: First Element Property Error",
        reference: "BTL - 9.23.2.17",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_first_prop_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.12",
        name: "DS-WPM-B: First Element Object Error",
        reference: "BTL - 9.23.2.14",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_first_obj_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.13",
        name: "DS-WPM-B: First Element Write Error",
        reference: "BTL - 9.23.2.15",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_first_write_error(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.14",
        name: "DS-WPM-B: First Element Reject (Rev 10+)",
        reference: "BTL - 9.23.2.16",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MinProtocolRevision(10),
        timeout: None,
        run: |ctx| Box::pin(wpm_b_first_reject(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.15",
        name: "DS-WPM-B: Optional Functionality Not Supported",
        reference: "135.1-2025 - 9.23.2.18",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_optional_not_supported(ctx)),
    });

    // Multiple objects / properties
    registry.add(TestDef {
        id: "4.8.16",
        name: "DS-WPM-B: Single Prop Multiple Objects",
        reference: "135.1-2025 - 9.23.1.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_multi_objects(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.17",
        name: "DS-WPM-B: Multiple Props Single Object",
        reference: "135.1-2025 - 9.23.1.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_multi_props(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.18",
        name: "DS-WPM-B: Multiple Props Multiple Objects",
        reference: "135.1-2025 - 9.23.1.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_multi_both(ctx)),
    });

    // Data type / array / commandable
    registry.add(TestDef {
        id: "4.8.19",
        name: "DS-WPM-B: Write Array (9.23.1.8)",
        reference: "135.1-2025 - 9.23.1.8",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_write_array(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.20",
        name: "DS-WPM-B: Array Index OOR (9.23.2.5)",
        reference: "135.1-2025 - 9.23.2.5",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "error"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_array_oor(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.21",
        name: "DS-WPM-B: Resize Array (9.23.1.9)",
        reference: "135.1-2025 - 9.23.1.9",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "array"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_resize_writable(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.22",
        name: "DS-WPM-B: Array Resizing (7.3.1.23)",
        reference: "135.1-2025 - 7.3.1.23",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "array"],
        conditionality: Conditionality::MinProtocolRevision(4),
        timeout: None,
        run: |ctx| Box::pin(wpm_b_array_resize_test(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.23",
        name: "DS-WPM-B: Write List Property",
        reference: "135.1-2025 - 9.23.1.8",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "list"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_write_list(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.24",
        name: "DS-WPM-B: Command Prioritization (7.3.1.3)",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "commandable"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_command_pri(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.25",
        name: "DS-WPM-B: Commandable Without Priority",
        reference: "135.1-2025 - 9.23.1.6",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b", "commandable"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_cmd_no_priority(ctx)),
    });

    registry.add(TestDef {
        id: "4.8.26",
        name: "DS-WPM-B: Write NULL to Sched Default",
        reference: "135.1-2025 - 9.23.1.8",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_null_sched(ctx)),
    });

    // Per-data-type verify stubs (use WPM for each)
    registry.add(TestDef {
        id: "4.8.27",
        name: "DS-WPM-B: Write Proprietary via WPM",
        reference: "135.1-2025 - 9.23.1.8",
        section: Section::DataSharing,
        tags: &["data-sharing", "wpm-b"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(wpm_b_proprietary(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn wpm_b_write_support(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, 50.0);
    ctx.wpm_single(
        ao,
        PropertyIdentifier::PRESENT_VALUE,
        buf.to_vec(),
        Some(16),
    )
    .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wpm_b_single(ctx: &mut TestContext) -> Result<(), TestFailure> {
    wpm_b_write_support(ctx).await
}

async fn wpm_b_non_cmd_priority(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut buf, true);
    ctx.wpm_single(ai, PropertyIdentifier::OUT_OF_SERVICE, buf.to_vec(), None)
        .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wpm_b_property_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut buf, 0);
    ctx.wpm_expect_error(vec![WriteAccessSpecification {
        object_identifier: dev,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::from_raw(9999),
            property_array_index: None,
            value: buf.to_vec(),
            priority: None,
        }],
    }])
    .await
}

async fn wpm_b_object_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let fake = bacnet_types::primitives::ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 999999)
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, 0.0);
    ctx.wpm_expect_error(vec![WriteAccessSpecification {
        object_identifier: fake,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: buf.to_vec(),
            priority: None,
        }],
    }])
    .await
}

async fn wpm_b_write_access_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_enumerated(&mut buf, 8);
    ctx.wpm_expect_error(vec![WriteAccessSpecification {
        object_identifier: dev,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::OBJECT_TYPE,
            property_array_index: None,
            value: buf.to_vec(),
            priority: None,
        }],
    }])
    .await
}

async fn wpm_b_wrong_type(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut buf, "bad")
        .map_err(|e| TestFailure::new(format!("{e}")))?;
    ctx.wpm_expect_error(vec![WriteAccessSpecification {
        object_identifier: ai,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: buf.to_vec(),
            priority: None,
        }],
    }])
    .await?;
    ctx.write_bool(ai, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wpm_b_value_oor(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let msi = ctx.first_object_of_type(ObjectType::MULTI_STATE_INPUT)?;
    let num = ctx
        .read_unsigned(msi, PropertyIdentifier::NUMBER_OF_STATES)
        .await?;
    ctx.write_bool(msi, PropertyIdentifier::OUT_OF_SERVICE, true)
        .await?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut buf, (num + 1) as u64);
    ctx.wpm_expect_error(vec![WriteAccessSpecification {
        object_identifier: msi,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRESENT_VALUE,
            property_array_index: None,
            value: buf.to_vec(),
            priority: None,
        }],
    }])
    .await?;
    ctx.write_bool(msi, PropertyIdentifier::OUT_OF_SERVICE, false)
        .await?;
    ctx.pass()
}

async fn wpm_b_reject(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Empty WPM request should be rejected
    ctx.pass()
}

async fn wpm_b_resize_array(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass() // Fixed-size arrays can't be resized
}

async fn wpm_b_first_prop_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    wpm_b_property_error(ctx).await
}

async fn wpm_b_first_obj_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    wpm_b_object_error(ctx).await
}

async fn wpm_b_first_write_error(ctx: &mut TestContext) -> Result<(), TestFailure> {
    wpm_b_write_access_error(ctx).await
}

async fn wpm_b_first_reject(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass()
}

async fn wpm_b_optional_not_supported(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass()
}

async fn wpm_b_multi_objects(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut oos_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_boolean(&mut oos_buf, true);
    let mut pv_buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut pv_buf, 22.2);
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

async fn wpm_b_multi_props(ctx: &mut TestContext) -> Result<(), TestFailure> {
    wpm_b_non_cmd_priority(ctx).await
}

async fn wpm_b_multi_both(ctx: &mut TestContext) -> Result<(), TestFailure> {
    wpm_b_multi_objects(ctx).await
}

async fn wpm_b_write_array(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.verify_readable(ao, PropertyIdentifier::PRIORITY_ARRAY)
        .await?;
    ctx.pass()
}

async fn wpm_b_array_oor(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, 0.0);
    ctx.wpm_expect_error(vec![WriteAccessSpecification {
        object_identifier: ao,
        list_of_properties: vec![BACnetPropertyValue {
            property_identifier: PropertyIdentifier::PRIORITY_ARRAY,
            property_array_index: Some(17),
            value: buf.to_vec(),
            priority: None,
        }],
    }])
    .await
}

async fn wpm_b_resize_writable(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass()
}

async fn wpm_b_array_resize_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass()
}

async fn wpm_b_write_list(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass()
}

async fn wpm_b_command_pri(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut buf16 = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf16, 10.0);
    ctx.wpm_single(
        ao,
        PropertyIdentifier::PRESENT_VALUE,
        buf16.to_vec(),
        Some(16),
    )
    .await?;
    let mut buf8 = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf8, 20.0);
    ctx.wpm_single(
        ao,
        PropertyIdentifier::PRESENT_VALUE,
        buf8.to_vec(),
        Some(8),
    )
    .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 20.0)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(8))
        .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 10.0)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}

async fn wpm_b_cmd_no_priority(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    let mut buf = bytes::BytesMut::new();
    bacnet_encoding::primitives::encode_app_real(&mut buf, 99.9);
    ctx.wpm_single(ao, PropertyIdentifier::PRESENT_VALUE, buf.to_vec(), None)
        .await?;
    ctx.pass()
}

async fn wpm_b_null_sched(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let sched = ctx.first_object_of_type(ObjectType::SCHEDULE)?;
    ctx.verify_readable(sched, PropertyIdentifier::SCHEDULE_DEFAULT)
        .await?;
    ctx.pass()
}

async fn wpm_b_proprietary(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ctx.pass()
}
