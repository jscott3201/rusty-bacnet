//! BTL Test Plan Section 3.42 — AccessDoor.
//! BTL refs (16 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.42.1",
        name: "AD: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::ACCESS_DOOR)),
    });
    registry.add(TestDef {
        id: "3.42.2",
        name: "AD: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ACCESS_DOOR,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.3",
        name: "AD: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.30.2",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.4",
        name: "AD: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.30.3",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.5",
        name: "AD: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.30.4",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.6",
        name: "AD: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.30.5",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.7",
        name: "AD: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.30.6",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.8",
        name: "AD: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.30.7",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.9",
        name: "AD: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.30.8",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.10",
        name: "AD: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.30.9",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.11",
        name: "AD: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.30.10",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.12",
        name: "AD: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.30.11",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.13",
        name: "AD: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.30.12",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.14",
        name: "AD: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.30.13",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.15",
        name: "AD: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.30.14",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.42.16",
        name: "AD: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.30.15",
        section: Section::Objects,
        tags: &["objects", "ad"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(30)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_DOOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
