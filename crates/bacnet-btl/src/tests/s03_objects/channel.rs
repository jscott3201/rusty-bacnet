//! BTL Test Plan Section 3.53 — Channel.
//! BTL refs (51 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.53.1",
        name: "CH: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::CHANNEL)),
    });
    registry.add(TestDef {
        id: "3.53.2",
        name: "CH: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::CHANNEL,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.3",
        name: "CH: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.53.2",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.4",
        name: "CH: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.53.3",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.5",
        name: "CH: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.53.4",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.6",
        name: "CH: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.53.5",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.7",
        name: "CH: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.53.6",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.8",
        name: "CH: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.53.7",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.9",
        name: "CH: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.53.8",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.10",
        name: "CH: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.53.9",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.11",
        name: "CH: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.53.10",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.12",
        name: "CH: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.53.11",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.13",
        name: "CH: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.53.12",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.14",
        name: "CH: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.53.13",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.15",
        name: "CH: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.53.14",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.16",
        name: "CH: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.53.15",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.17",
        name: "CH: Object-Specific Test 16",
        reference: "135.1-2025 - 7.3.2.53.16",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.18",
        name: "CH: Object-Specific Test 17",
        reference: "135.1-2025 - 7.3.2.53.17",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.19",
        name: "CH: Object-Specific Test 18",
        reference: "135.1-2025 - 7.3.2.53.18",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.20",
        name: "CH: Object-Specific Test 19",
        reference: "135.1-2025 - 7.3.2.53.19",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.21",
        name: "CH: Object-Specific Test 20",
        reference: "135.1-2025 - 7.3.2.53.20",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.22",
        name: "CH: Object-Specific Test 21",
        reference: "135.1-2025 - 7.3.2.53.21",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.23",
        name: "CH: Object-Specific Test 22",
        reference: "135.1-2025 - 7.3.2.53.22",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.24",
        name: "CH: Object-Specific Test 23",
        reference: "135.1-2025 - 7.3.2.53.23",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.25",
        name: "CH: Object-Specific Test 24",
        reference: "135.1-2025 - 7.3.2.53.24",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.26",
        name: "CH: Object-Specific Test 25",
        reference: "135.1-2025 - 7.3.2.53.25",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.27",
        name: "CH: Object-Specific Test 26",
        reference: "135.1-2025 - 7.3.2.53.26",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.28",
        name: "CH: Object-Specific Test 27",
        reference: "135.1-2025 - 7.3.2.53.27",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.29",
        name: "CH: Object-Specific Test 28",
        reference: "135.1-2025 - 7.3.2.53.28",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.30",
        name: "CH: Object-Specific Test 29",
        reference: "135.1-2025 - 7.3.2.53.29",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.31",
        name: "CH: Object-Specific Test 30",
        reference: "135.1-2025 - 7.3.2.53.30",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.32",
        name: "CH: Object-Specific Test 31",
        reference: "135.1-2025 - 7.3.2.53.31",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.33",
        name: "CH: Object-Specific Test 32",
        reference: "135.1-2025 - 7.3.2.53.32",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.34",
        name: "CH: Object-Specific Test 33",
        reference: "135.1-2025 - 7.3.2.53.33",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.35",
        name: "CH: Object-Specific Test 34",
        reference: "135.1-2025 - 7.3.2.53.34",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.36",
        name: "CH: Object-Specific Test 35",
        reference: "135.1-2025 - 7.3.2.53.35",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.37",
        name: "CH: Object-Specific Test 36",
        reference: "135.1-2025 - 7.3.2.53.36",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.38",
        name: "CH: Object-Specific Test 37",
        reference: "135.1-2025 - 7.3.2.53.37",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.39",
        name: "CH: Object-Specific Test 38",
        reference: "135.1-2025 - 7.3.2.53.38",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.40",
        name: "CH: Object-Specific Test 39",
        reference: "135.1-2025 - 7.3.2.53.39",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.41",
        name: "CH: Object-Specific Test 40",
        reference: "135.1-2025 - 7.3.2.53.40",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.42",
        name: "CH: Object-Specific Test 41",
        reference: "135.1-2025 - 7.3.2.53.41",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.43",
        name: "CH: Object-Specific Test 42",
        reference: "135.1-2025 - 7.3.2.53.42",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.44",
        name: "CH: Object-Specific Test 43",
        reference: "135.1-2025 - 7.3.2.53.43",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.45",
        name: "CH: Object-Specific Test 44",
        reference: "135.1-2025 - 7.3.2.53.44",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.46",
        name: "CH: Object-Specific Test 45",
        reference: "135.1-2025 - 7.3.2.53.45",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.47",
        name: "CH: Object-Specific Test 46",
        reference: "135.1-2025 - 7.3.2.53.46",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.48",
        name: "CH: Object-Specific Test 47",
        reference: "135.1-2025 - 7.3.2.53.47",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.49",
        name: "CH: Object-Specific Test 48",
        reference: "135.1-2025 - 7.3.2.53.48",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.50",
        name: "CH: Object-Specific Test 49",
        reference: "135.1-2025 - 7.3.2.53.49",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.53.51",
        name: "CH: Object-Specific Test 50",
        reference: "135.1-2025 - 7.3.2.53.50",
        section: Section::Objects,
        tags: &["objects", "ch"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(53)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CHANNEL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
