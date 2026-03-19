//! BTL Test Plan Section 3.39+3.40 — LifeSafetyPoint+Zone.
//! BTL refs (20 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.39.1",
        name: "LSP: Writable Mode",
        reference: "135.1-2025 - 7.3.2.15.6",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.2",
        name: "LSP: Writable Tracking_Value",
        reference: "135.1-2025 - 7.3.2.15.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.3",
        name: "LSP: Silenced",
        reference: "135.1-2025 - 7.3.2.15.9",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.4",
        name: "LSP: Operation_Expected",
        reference: "135.1-2025 - 7.3.2.15.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.5",
        name: "LSP: Writable Member_Of",
        reference: "135.1-2025 - 7.3.2.15.8",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.6",
        name: "LSP: Value_Source",
        reference: "BTL - 7.3.1.28.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.7",
        name: "LSP: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.8",
        name: "LSP: VS Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.9",
        name: "LSP: VS Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.10",
        name: "LSP: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::LIFE_SAFETY_POINT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.11",
        name: "LSZ: Writable Mode",
        reference: "135.1-2025 - 7.3.2.15.6",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.12",
        name: "LSZ: Writable Tracking_Value",
        reference: "135.1-2025 - 7.3.2.15.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.13",
        name: "LSZ: Silenced",
        reference: "135.1-2025 - 7.3.2.15.9",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.14",
        name: "LSZ: Operation_Expected",
        reference: "135.1-2025 - 7.3.2.15.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.15",
        name: "LSZ: Writable Member_Of",
        reference: "135.1-2025 - 7.3.2.15.8",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.16",
        name: "LSZ: Value_Source",
        reference: "BTL - 7.3.1.28.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.17",
        name: "LSZ: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.18",
        name: "LSZ: VS Write By Other",
        reference: "BTL - 7.3.1.28.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.19",
        name: "LSZ: VS Initiated Locally",
        reference: "BTL - 7.3.1.28.X1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_value_source_write_by_other(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.39.20",
        name: "LSZ: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(22)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::LIFE_SAFETY_ZONE,
            ))
        },
    });
}
