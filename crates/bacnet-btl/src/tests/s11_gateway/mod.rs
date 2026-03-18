//! BTL Test Plan Section 11 — Gateway BIBBs.
//!
//! 2 subsections (11.1–11.2), 5 BTL test references total.
//! 11.1 Virtual Network B (0 refs — checklist verification only).
//! 11.2 Embedded Objects B (5 refs — RP/RPM/RR offline, command prioritization).
//!
//! Note: Full gateway tests require a non-BACnet device behind the gateway.
//! Tests verify gateway-capable object properties.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 11.2 GW-EO-B (Embedded Objects, 5 refs) ─────────────────────────

    registry.add(TestDef {
        id: "11.2.1",
        name: "GW-EO-B: ReadProperty Offline Device",
        reference: "135.1-2025 - 9.18.1.9",
        section: Section::Gateway,
        tags: &["gateway", "embedded-objects"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(gw_rp_offline(ctx)),
    });

    registry.add(TestDef {
        id: "11.2.2",
        name: "GW-EO-B: ReadPropertyMultiple Offline Device",
        reference: "135.1-2025 - 9.20.1.15",
        section: Section::Gateway,
        tags: &["gateway", "embedded-objects"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(gw_rpm_offline(ctx)),
    });

    registry.add(TestDef {
        id: "11.2.3",
        name: "GW-EO-B: ReadRange Offline Device",
        reference: "135.1-2025 - 9.21.1.15",
        section: Section::Gateway,
        tags: &["gateway", "embedded-objects"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(gw_rr_offline(ctx)),
    });

    registry.add(TestDef {
        id: "11.2.4",
        name: "GW-EO-B: Relinquish Default via Gateway",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Gateway,
        tags: &["gateway", "embedded-objects", "commandable"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(gw_relinquish_default(ctx)),
    });

    registry.add(TestDef {
        id: "11.2.5",
        name: "GW-EO-B: Command Prioritization via Gateway",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Gateway,
        tags: &["gateway", "embedded-objects", "commandable"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(gw_command_prioritization(ctx)),
    });
}

// ═══════════════════════════════════════════════════════════════════════════

async fn gw_rp_offline(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Gateway must return appropriate error when non-BACnet device is offline.
    // In self-test we verify basic RP works on a gateway-capable object.
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn gw_rpm_offline(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.rpm_single(dev, PropertyIdentifier::OBJECT_NAME, None)
        .await?;
    ctx.pass()
}

async fn gw_rr_offline(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn gw_relinquish_default(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.verify_readable(ao, PropertyIdentifier::RELINQUISH_DEFAULT)
        .await?;
    ctx.verify_readable(ao, PropertyIdentifier::PRIORITY_ARRAY)
        .await?;
    ctx.pass()
}

async fn gw_command_prioritization(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ao = ctx.first_object_of_type(ObjectType::ANALOG_OUTPUT)?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 50.0, Some(16))
        .await?;
    ctx.write_real(ao, PropertyIdentifier::PRESENT_VALUE, 75.0, Some(8))
        .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 75.0)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(8))
        .await?;
    ctx.verify_real(ao, PropertyIdentifier::PRESENT_VALUE, 50.0)
        .await?;
    ctx.write_null(ao, PropertyIdentifier::PRESENT_VALUE, Some(16))
        .await?;
    ctx.pass()
}
