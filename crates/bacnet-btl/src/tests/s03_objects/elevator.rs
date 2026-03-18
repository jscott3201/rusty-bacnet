//! BTL Test Plan Section 3.58-3.60 — Elevator+Lift+Escalator.
//! BTL refs (13 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.58.1",
        name: "EG: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "eg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(57)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::ELEVATOR_GROUP,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.2",
        name: "EG: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "eg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(57)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ELEVATOR_GROUP,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.3",
        name: "EG: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.57.2",
        section: Section::Objects,
        tags: &["objects", "eg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(57)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ELEVATOR_GROUP,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.4",
        name: "EG: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.57.3",
        section: Section::Objects,
        tags: &["objects", "eg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(57)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ELEVATOR_GROUP,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.5",
        name: "LIFT: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "lift"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(58)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::LIFT)),
    });
    registry.add(TestDef {
        id: "3.58.6",
        name: "LIFT: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "lift"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(58)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::LIFT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.7",
        name: "LIFT: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.58.2",
        section: Section::Objects,
        tags: &["objects", "lift"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(58)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.8",
        name: "LIFT: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.58.3",
        section: Section::Objects,
        tags: &["objects", "lift"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(58)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.9",
        name: "ESC: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "esc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(59)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::ESCALATOR)),
    });
    registry.add(TestDef {
        id: "3.58.10",
        name: "ESC: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "esc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(59)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ESCALATOR,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.11",
        name: "ESC: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.59.2",
        section: Section::Objects,
        tags: &["objects", "esc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(59)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ESCALATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.12",
        name: "ESC: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.59.3",
        section: Section::Objects,
        tags: &["objects", "esc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(59)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ESCALATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.58.13",
        name: "ESC: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.59.4",
        section: Section::Objects,
        tags: &["objects", "esc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(59)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ESCALATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
