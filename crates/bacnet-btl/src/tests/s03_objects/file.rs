//! BTL Test Plan Section 3.61 — File.
//! BTL refs (10 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.61.1",
        name: "FILE: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::FILE)),
    });
    registry.add(TestDef {
        id: "3.61.2",
        name: "FILE: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::FILE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.3",
        name: "FILE: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.10.2",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.4",
        name: "FILE: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.10.3",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.5",
        name: "FILE: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.10.4",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.6",
        name: "FILE: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.10.5",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.7",
        name: "FILE: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.10.6",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.8",
        name: "FILE: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.10.7",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.9",
        name: "FILE: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.10.8",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.61.10",
        name: "FILE: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.10.9",
        section: Section::Objects,
        tags: &["objects", "file"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(10)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::FILE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
