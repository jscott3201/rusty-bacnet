//! BTL Test Plan Section 3.62 — Staging.
//! BTL refs (24 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.62.1",
        name: "STG: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::STAGING)),
    });
    registry.add(TestDef {
        id: "3.62.2",
        name: "STG: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::STAGING,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.3",
        name: "STG: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.60.2",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.4",
        name: "STG: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.60.3",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.5",
        name: "STG: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.60.4",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.6",
        name: "STG: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.60.5",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.7",
        name: "STG: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.60.6",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.8",
        name: "STG: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.60.7",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.9",
        name: "STG: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.60.8",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.10",
        name: "STG: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.60.9",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.11",
        name: "STG: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.60.10",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.12",
        name: "STG: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.60.11",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.13",
        name: "STG: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.60.12",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.14",
        name: "STG: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.60.13",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.15",
        name: "STG: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.60.14",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.16",
        name: "STG: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.60.15",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.17",
        name: "STG: Object-Specific Test 16",
        reference: "135.1-2025 - 7.3.2.60.16",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.18",
        name: "STG: Object-Specific Test 17",
        reference: "135.1-2025 - 7.3.2.60.17",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.19",
        name: "STG: Object-Specific Test 18",
        reference: "135.1-2025 - 7.3.2.60.18",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.20",
        name: "STG: Object-Specific Test 19",
        reference: "135.1-2025 - 7.3.2.60.19",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.21",
        name: "STG: Object-Specific Test 20",
        reference: "135.1-2025 - 7.3.2.60.20",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.22",
        name: "STG: Object-Specific Test 21",
        reference: "135.1-2025 - 7.3.2.60.21",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.23",
        name: "STG: Object-Specific Test 22",
        reference: "135.1-2025 - 7.3.2.60.22",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.62.24",
        name: "STG: Object-Specific Test 23",
        reference: "135.1-2025 - 7.3.2.60.23",
        section: Section::Objects,
        tags: &["objects", "stg"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(60)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::STAGING,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
