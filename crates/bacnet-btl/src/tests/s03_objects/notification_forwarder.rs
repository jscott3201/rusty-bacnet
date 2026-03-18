//! BTL Test Plan Section 3.51 — NotificationForwarder.
//! BTL refs (16 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.51.1",
        name: "NF: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.2",
        name: "NF: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.3",
        name: "NF: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.51.2",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.4",
        name: "NF: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.51.3",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.5",
        name: "NF: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.51.4",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.6",
        name: "NF: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.51.5",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.7",
        name: "NF: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.51.6",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.8",
        name: "NF: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.51.7",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.9",
        name: "NF: Object-Specific Test 8",
        reference: "135.1-2025 - 7.3.2.51.8",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.10",
        name: "NF: Object-Specific Test 9",
        reference: "135.1-2025 - 7.3.2.51.9",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.11",
        name: "NF: Object-Specific Test 10",
        reference: "135.1-2025 - 7.3.2.51.10",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.12",
        name: "NF: Object-Specific Test 11",
        reference: "135.1-2025 - 7.3.2.51.11",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.13",
        name: "NF: Object-Specific Test 12",
        reference: "135.1-2025 - 7.3.2.51.12",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.14",
        name: "NF: Object-Specific Test 13",
        reference: "135.1-2025 - 7.3.2.51.13",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.15",
        name: "NF: Object-Specific Test 14",
        reference: "135.1-2025 - 7.3.2.51.14",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.51.16",
        name: "NF: Object-Specific Test 15",
        reference: "135.1-2025 - 7.3.2.51.15",
        section: Section::Objects,
        tags: &["objects", "nf"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::NOTIFICATION_FORWARDER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
