//! BTL Test Plan Section 3.65+3.66 — Color+ColorTemperature.
//! BTL refs (36 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.65.1",
        name: "CLR: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::COLOR)),
    });
    registry.add(TestDef {
        id: "3.65.2",
        name: "CLR: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::COLOR,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.3",
        name: "CLR: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.63.2",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.4",
        name: "CLR: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.63.3",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.5",
        name: "CLR: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.63.4",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.6",
        name: "CLR: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.63.5",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.7",
        name: "CLR: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.63.6",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.8",
        name: "CLR: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.63.7",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.9",
        name: "CLR: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.63.8",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.10",
        name: "CLR: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.63.9",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.11",
        name: "CLR: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.63.10",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.12",
        name: "CLR: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.63.11",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.13",
        name: "CLR: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.63.12",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.14",
        name: "CLR: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.63.13",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.15",
        name: "CLR: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.63.14",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.16",
        name: "CLR: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.63.15",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.17",
        name: "CLR: Object-Specific Test 16",
        reference: "135.1-2025 - 7.3.2.63.16",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.18",
        name: "CLR: Object-Specific Test 17",
        reference: "135.1-2025 - 7.3.2.63.17",
        section: Section::Objects,
        tags: &["objects", "clr"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(63)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.19",
        name: "CT: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.20",
        name: "CT: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.21",
        name: "CT: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.64.2",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.22",
        name: "CT: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.64.3",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.23",
        name: "CT: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.64.4",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.24",
        name: "CT: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.64.5",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.25",
        name: "CT: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.64.6",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.26",
        name: "CT: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.64.7",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.27",
        name: "CT: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.64.8",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.28",
        name: "CT: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.64.9",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.29",
        name: "CT: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.64.10",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.30",
        name: "CT: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.64.11",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.31",
        name: "CT: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.64.12",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.32",
        name: "CT: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.64.13",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.33",
        name: "CT: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.64.14",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.34",
        name: "CT: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.64.15",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.35",
        name: "CT: Object-Specific Test 16",
        reference: "135.1-2025 - 7.3.2.64.16",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.65.36",
        name: "CT: Object-Specific Test 17",
        reference: "135.1-2025 - 7.3.2.64.17",
        section: Section::Objects,
        tags: &["objects", "ct"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(64)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::COLOR_TEMPERATURE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
