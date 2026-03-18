//! BTL Test Plan Section 3.2 — Analog Output Object.
//!
//! BTL references (8 total):
//!   1. 135.1-2025 - 7.3.1.2 — Relinquish Default Test
//!   2. 135.1-2025 - 7.3.1.3 — Command Prioritization Test
//!   3. BTL - 7.3.1.1.1 — Out_Of_Service, Status_Flags, Reliability
//!   4. BTL - 7.3.1.28.3 — Value_Source Property None Test
//!   5. BTL - 7.3.1.28.4 — Commandable Value Source Test
//!   6. BTL - 7.3.1.28.1 — Writing Value_Source by non-commanding device
//!   7. BTL - 7.3.1.28.X1 — Value Source Initiated Locally
//!   8. 135.1-2025 - 7.3.1.21.3 — Reliability_Evaluation_Inhibit

use bacnet_types::enums::ObjectType;

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;

const OT: u32 = 1; // ANALOG_OUTPUT

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.2.1",
        name: "AO: Relinquish Default",
        reference: "135.1-2025 - 7.3.1.2",
        section: Section::Objects,
        tags: &["objects", "ao", "relinquish"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_relinquish_default(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.2.2",
        name: "AO: Command Prioritization",
        reference: "135.1-2025 - 7.3.1.3",
        section: Section::Objects,
        tags: &["objects", "ao", "command-priority"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_command_prioritization(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.2.3",
        name: "AO: Out_Of_Service, Status_Flags, Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ao", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.2.4",
        name: "AO: Value_Source Property None",
        reference: "BTL - 7.3.1.28.3",
        section: Section::Objects,
        tags: &["objects", "ao", "value-source"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_none(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.2.5",
        name: "AO: Commandable Value Source",
        reference: "BTL - 7.3.1.28.4",
        section: Section::Objects,
        tags: &["objects", "ao", "value-source"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_commandable(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.2.6",
        name: "AO: Value_Source Write By Other Device",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects", "ao", "value-source"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.2.7",
        name: "AO: Value Source Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects", "ao", "value-source"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_local(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });

    registry.add(TestDef {
        id: "3.2.8",
        name: "AO: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ao", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ANALOG_OUTPUT,
            ))
        },
    });
}
