//! BTL Test Plan Section 3.56 — Network Port Object.
//! BTL refs (2): APDU_Length Test, REI
use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;
use crate::tests::helpers;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.56.1",
        name: "NP: APDU_Length Test",
        reference: "135.1-2025 - 7.3.2.46.5",
        section: Section::Objects,
        tags: &["objects", "np"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(56)),
        timeout: None,
        run: |ctx| Box::pin(np_apdu_length(ctx)),
    });
    registry.add(TestDef {
        id: "3.56.2",
        name: "NP: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "np", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(56)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::NETWORK_PORT,
            ))
        },
    });
}
async fn np_apdu_length(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ObjectType::NETWORK_PORT)?;
    ctx.verify_readable(oid, PropertyIdentifier::NETWORK_TYPE)
        .await?;
    ctx.verify_readable(oid, PropertyIdentifier::NETWORK_NUMBER)
        .await?;
    ctx.pass()
}
