//! BTL Test Plan Section 3.3 — Analog Value Object.
//!
//! BTL references (10 total):
//!   1. BTL - 7.3.1.1.1 — Out_Of_Service, Status_Flags, Reliability
//!   2. 135.1-2025 - 7.3.1.1.2 — Out_Of_Service for Commandable Value Objects
//!   3. 135.1-2025 - 7.3.1.2 — Relinquish Default Test
//!   4. 135.1-2025 - 7.3.1.3 — Command Prioritization Test
//!   5. BTL - 7.3.1.28.2 — Non-commandable Value_Source Property Test
//!   6. BTL - 7.3.1.28.3 — Value_Source Property None Test
//!   7. BTL - 7.3.1.28.4 — Commandable Value Source Test
//!   8. BTL - 7.3.1.28.1 — Writing Value_Source by non-commanding device
//!   9. BTL - 7.3.1.28.X1 — Value Source Initiated Locally
//!  10. 135.1-2025 - 7.3.1.21.3 — Reliability_Evaluation_Inhibit

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

const OT: u32 = 2; // ANALOG_VALUE
const T: ObjectType = ObjectType::ANALOG_VALUE;

pub fn register(registry: &mut TestRegistry) {
    let c = Conditionality::RequiresCapability(Capability::ObjectType(OT));

    registry.add(TestDef {
        id: "3.3.1",
        name: "AV: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "av", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.2",
        name: "AV: OOS for Commandable Objects",
        reference: "135.1-2025 - 7.3.1.1.2",
        section: Section::Objects,
        tags: &["objects", "av", "oos-cmd"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.3",
        name: "AV: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "av", "relinquish"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_relinquish_default(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.4",
        name: "AV: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "av", "cmd-pri"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_command_prioritization(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.5",
        name: "AV: Non-commandable Value_Source",
        reference: "BTL - 7.3.1.28.2",
        section: Section::Objects,
        tags: &["objects", "av", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_non_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.6",
        name: "AV: Value_Source None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "av", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_none(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.7",
        name: "AV: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "av", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_commandable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.8",
        name: "AV: Value_Source Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "av", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_write_by_other(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.9",
        name: "AV: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "av", "vs"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_value_source_local(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.3.10",
        name: "AV: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "av", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });

    let _ = c; // suppress unused warning
}
