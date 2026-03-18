//! BTL Test Plan Section 3.43 — LoadControl.
//! BTL refs (9 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.43.1",
        name: "LC: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::LOAD_CONTROL,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.2",
        name: "LC: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::LOAD_CONTROL,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.3",
        name: "LC: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.28.2",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LOAD_CONTROL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.4",
        name: "LC: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.28.3",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LOAD_CONTROL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.5",
        name: "LC: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.28.4",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LOAD_CONTROL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.6",
        name: "LC: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.28.5",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LOAD_CONTROL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.7",
        name: "LC: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.28.6",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LOAD_CONTROL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.8",
        name: "LC: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.28.7",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LOAD_CONTROL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.43.9",
        name: "LC: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.28.8",
        section: Section::Objects,
        tags: &["objects", "lc"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(28)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LOAD_CONTROL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
