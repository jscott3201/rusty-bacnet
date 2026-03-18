//! BTL Test Plan Section 3.54+3.55 — Lighting+BinaryLighting.
//! BTL refs (59 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.54.1",
        name: "LO: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.2",
        name: "LO: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.3",
        name: "LO: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.54.2",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.4",
        name: "LO: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.54.3",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.5",
        name: "LO: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.54.4",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.6",
        name: "LO: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.54.5",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.7",
        name: "LO: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.54.6",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.8",
        name: "LO: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.54.7",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.9",
        name: "LO: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.54.8",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.10",
        name: "LO: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.54.9",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.11",
        name: "LO: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.54.10",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.12",
        name: "LO: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.54.11",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.13",
        name: "LO: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.54.12",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.14",
        name: "LO: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.54.13",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.15",
        name: "LO: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.54.14",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.16",
        name: "LO: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.54.15",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.17",
        name: "LO: Object-Specific Test 16",
        reference: "135.1-2025 - 7.3.2.54.16",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.18",
        name: "LO: Object-Specific Test 17",
        reference: "135.1-2025 - 7.3.2.54.17",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.19",
        name: "LO: Object-Specific Test 18",
        reference: "135.1-2025 - 7.3.2.54.18",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.20",
        name: "LO: Object-Specific Test 19",
        reference: "135.1-2025 - 7.3.2.54.19",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.21",
        name: "LO: Object-Specific Test 20",
        reference: "135.1-2025 - 7.3.2.54.20",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.22",
        name: "LO: Object-Specific Test 21",
        reference: "135.1-2025 - 7.3.2.54.21",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.23",
        name: "LO: Object-Specific Test 22",
        reference: "135.1-2025 - 7.3.2.54.22",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.24",
        name: "LO: Object-Specific Test 23",
        reference: "135.1-2025 - 7.3.2.54.23",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.25",
        name: "LO: Object-Specific Test 24",
        reference: "135.1-2025 - 7.3.2.54.24",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.26",
        name: "LO: Object-Specific Test 25",
        reference: "135.1-2025 - 7.3.2.54.25",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.27",
        name: "LO: Object-Specific Test 26",
        reference: "135.1-2025 - 7.3.2.54.26",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.28",
        name: "LO: Object-Specific Test 27",
        reference: "135.1-2025 - 7.3.2.54.27",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.29",
        name: "LO: Object-Specific Test 28",
        reference: "135.1-2025 - 7.3.2.54.28",
        section: Section::Objects,
        tags: &["objects", "lo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(54)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.30",
        name: "BLO: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.31",
        name: "BLO: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.32",
        name: "BLO: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.55.2",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.33",
        name: "BLO: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.55.3",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.34",
        name: "BLO: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.55.4",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.35",
        name: "BLO: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.55.5",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.36",
        name: "BLO: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.55.6",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.37",
        name: "BLO: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.55.7",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.38",
        name: "BLO: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.55.8",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.39",
        name: "BLO: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.55.9",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.40",
        name: "BLO: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.55.10",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.41",
        name: "BLO: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.55.11",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.42",
        name: "BLO: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.55.12",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.43",
        name: "BLO: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.55.13",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.44",
        name: "BLO: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.55.14",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.45",
        name: "BLO: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.55.15",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.46",
        name: "BLO: Object-Specific Test 16",
        reference: "135.1-2025 - 7.3.2.55.16",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.47",
        name: "BLO: Object-Specific Test 17",
        reference: "135.1-2025 - 7.3.2.55.17",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.48",
        name: "BLO: Object-Specific Test 18",
        reference: "135.1-2025 - 7.3.2.55.18",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.49",
        name: "BLO: Object-Specific Test 19",
        reference: "135.1-2025 - 7.3.2.55.19",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.50",
        name: "BLO: Object-Specific Test 20",
        reference: "135.1-2025 - 7.3.2.55.20",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.51",
        name: "BLO: Object-Specific Test 21",
        reference: "135.1-2025 - 7.3.2.55.21",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.52",
        name: "BLO: Object-Specific Test 22",
        reference: "135.1-2025 - 7.3.2.55.22",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.53",
        name: "BLO: Object-Specific Test 23",
        reference: "135.1-2025 - 7.3.2.55.23",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.54",
        name: "BLO: Object-Specific Test 24",
        reference: "135.1-2025 - 7.3.2.55.24",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.55",
        name: "BLO: Object-Specific Test 25",
        reference: "135.1-2025 - 7.3.2.55.25",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.56",
        name: "BLO: Object-Specific Test 26",
        reference: "135.1-2025 - 7.3.2.55.26",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.57",
        name: "BLO: Object-Specific Test 27",
        reference: "135.1-2025 - 7.3.2.55.27",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.58",
        name: "BLO: Object-Specific Test 28",
        reference: "135.1-2025 - 7.3.2.55.28",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.54.59",
        name: "BLO: Object-Specific Test 29",
        reference: "135.1-2025 - 7.3.2.55.29",
        section: Section::Objects,
        tags: &["objects", "blo"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(55)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::BINARY_LIGHTING_OUTPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
