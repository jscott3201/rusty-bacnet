//! Smoke tests — minimal end-to-end validation of the test engine.
//!
//! These are not BTL tests; they validate that the engine pipeline works
//! (registry → selector → runner → context → BACnet client → reporter).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "0.0.1",
        name: "Read Device Object_Identifier",
        reference: "Smoke test — validates engine pipeline",
        section: Section::BasicFunctionality,
        tags: &["smoke"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(read_device_object_identifier(ctx)),
    });

    registry.add(TestDef {
        id: "0.0.2",
        name: "Read Device Object_Name",
        reference: "Smoke test — validates string property read",
        section: Section::BasicFunctionality,
        tags: &["smoke"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(read_device_object_name(ctx)),
    });

    registry.add(TestDef {
        id: "0.0.3",
        name: "Read AI Present_Value",
        reference: "Smoke test — validates REAL property read",
        section: Section::BasicFunctionality,
        tags: &["smoke"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(read_ai_present_value(ctx)),
    });
}

/// Read the Device object's Object_Identifier and verify it matches expectations.
async fn read_device_object_identifier(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev_oid = ctx.first_object_of_type(ObjectType::DEVICE)?;

    // Read Object_Identifier — the value is an application-tagged ObjectIdentifier
    let data = ctx
        .read_property_raw(dev_oid, PropertyIdentifier::OBJECT_IDENTIFIER, None)
        .await?;

    if data.is_empty() {
        return Err(TestFailure::new("Object_Identifier returned empty"));
    }

    ctx.pass()
}

/// Read the Device object's Object_Name and verify it's non-empty.
async fn read_device_object_name(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev_oid = ctx.first_object_of_type(ObjectType::DEVICE)?;

    let data = ctx
        .read_property_raw(dev_oid, PropertyIdentifier::OBJECT_NAME, None)
        .await?;

    if data.len() < 2 {
        return Err(TestFailure::new("Object_Name is empty or too short"));
    }

    ctx.pass()
}

/// Read an AnalogInput's Present_Value and verify it decodes as a REAL.
async fn read_ai_present_value(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai_oid = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    let _value = ctx
        .read_real(ai_oid, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}
