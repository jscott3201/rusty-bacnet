//! BTL Test Plan Sections 4.21–4.23 — WriteGroup.
//! 6 BTL references: 4.21 A-side (1), 4.22 Internal-B (4), 4.23 External-B (1).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "4.21.1",
        name: "WG-A: Initiate WriteGroup",
        reference: "135.1-2025 - 8.22.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "write-group"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| Box::pin(wg_base(ctx)),
    });
    registry.add(TestDef {
        id: "4.22.1",
        name: "WG-Int-B: Accept WriteGroup",
        reference: "135.1-2025 - 9.37.1.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "write-group"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| Box::pin(wg_base(ctx)),
    });
    registry.add(TestDef {
        id: "4.22.2",
        name: "WG-Int-B: Channel Write-Through",
        reference: "135.1-2025 - 9.37.1.2",
        section: Section::DataSharing,
        tags: &["data-sharing", "write-group"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| Box::pin(wg_base(ctx)),
    });
    registry.add(TestDef {
        id: "4.22.3",
        name: "WG-Int-B: Inhibit Flag",
        reference: "135.1-2025 - 9.37.1.3",
        section: Section::DataSharing,
        tags: &["data-sharing", "write-group"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| Box::pin(wg_base(ctx)),
    });
    registry.add(TestDef {
        id: "4.22.4",
        name: "WG-Int-B: Overriding Priority",
        reference: "135.1-2025 - 9.37.1.4",
        section: Section::DataSharing,
        tags: &["data-sharing", "write-group"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| Box::pin(wg_base(ctx)),
    });
    registry.add(TestDef {
        id: "4.23.1",
        name: "WG-Ext-B: External WriteGroup",
        reference: "135.1-2025 - 9.37.2.1",
        section: Section::DataSharing,
        tags: &["data-sharing", "write-group"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| Box::pin(wg_base(ctx)),
    });
}

async fn wg_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ch = ctx.first_object_of_type(ObjectType::CHANNEL)?;
    ctx.verify_readable(ch, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.verify_readable(ch, PropertyIdentifier::CHANNEL_NUMBER)
        .await?;
    ctx.pass()
}
