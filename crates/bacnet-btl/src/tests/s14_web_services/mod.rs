//! BTL Test Plan Section 14 — BACnet Web Services BIBBs.
//!
//! 2 subsections (14.1–14.2), **0 BTL test references**.
//! Both subsections say "Contact BTL for Interim tests for this BIBB."
//!
//! We register baseline property checks so the section is represented.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "14.1.1",
        name: "WS-Client: Device Protocol_Revision",
        reference: "BTL - Contact BTL for Interim tests",
        section: Section::WebServices,
        tags: &["web-services"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ws_baseline(ctx)),
    });

    registry.add(TestDef {
        id: "14.2.1",
        name: "WS-Server: Device Protocol_Revision",
        reference: "BTL - Contact BTL for Interim tests",
        section: Section::WebServices,
        tags: &["web-services"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ws_baseline(ctx)),
    });
}

async fn ws_baseline(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::APPLICATION_SOFTWARE_VERSION)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_REVISION)
        .await?;
    ctx.pass()
}
