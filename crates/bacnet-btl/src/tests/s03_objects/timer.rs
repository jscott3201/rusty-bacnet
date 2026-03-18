//! BTL Test Plan Section 3.57 — Timer.
//! BTL refs (33 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.57.1",
        name: "TMR: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::TIMER)),
    });
    registry.add(TestDef {
        id: "3.57.2",
        name: "TMR: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::TIMER,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.3",
        name: "TMR: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.31.2",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.4",
        name: "TMR: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.31.3",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.5",
        name: "TMR: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.31.4",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.6",
        name: "TMR: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.31.5",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.7",
        name: "TMR: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.31.6",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.8",
        name: "TMR: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.31.7",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.9",
        name: "TMR: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.31.8",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.10",
        name: "TMR: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.31.9",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.11",
        name: "TMR: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.31.10",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.12",
        name: "TMR: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.31.11",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.13",
        name: "TMR: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.31.12",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.14",
        name: "TMR: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.31.13",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.15",
        name: "TMR: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.31.14",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.16",
        name: "TMR: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.31.15",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.17",
        name: "TMR: Object-Specific Test 16",
        reference: "135.1-2025 - 7.3.2.31.16",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.18",
        name: "TMR: Object-Specific Test 17",
        reference: "135.1-2025 - 7.3.2.31.17",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.19",
        name: "TMR: Object-Specific Test 18",
        reference: "135.1-2025 - 7.3.2.31.18",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.20",
        name: "TMR: Object-Specific Test 19",
        reference: "135.1-2025 - 7.3.2.31.19",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.21",
        name: "TMR: Object-Specific Test 20",
        reference: "135.1-2025 - 7.3.2.31.20",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.22",
        name: "TMR: Object-Specific Test 21",
        reference: "135.1-2025 - 7.3.2.31.21",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.23",
        name: "TMR: Object-Specific Test 22",
        reference: "135.1-2025 - 7.3.2.31.22",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.24",
        name: "TMR: Object-Specific Test 23",
        reference: "135.1-2025 - 7.3.2.31.23",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.25",
        name: "TMR: Object-Specific Test 24",
        reference: "135.1-2025 - 7.3.2.31.24",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.26",
        name: "TMR: Object-Specific Test 25",
        reference: "135.1-2025 - 7.3.2.31.25",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.27",
        name: "TMR: Object-Specific Test 26",
        reference: "135.1-2025 - 7.3.2.31.26",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.28",
        name: "TMR: Object-Specific Test 27",
        reference: "135.1-2025 - 7.3.2.31.27",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.29",
        name: "TMR: Object-Specific Test 28",
        reference: "135.1-2025 - 7.3.2.31.28",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.30",
        name: "TMR: Object-Specific Test 29",
        reference: "135.1-2025 - 7.3.2.31.29",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.31",
        name: "TMR: Object-Specific Test 30",
        reference: "135.1-2025 - 7.3.2.31.30",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.32",
        name: "TMR: Object-Specific Test 31",
        reference: "135.1-2025 - 7.3.2.31.31",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.57.33",
        name: "TMR: Object-Specific Test 32",
        reference: "135.1-2025 - 7.3.2.31.32",
        section: Section::Objects,
        tags: &["objects", "tmr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(31)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::TIMER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
