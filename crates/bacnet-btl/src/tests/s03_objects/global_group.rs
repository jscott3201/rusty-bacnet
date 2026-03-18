//! BTL Test Plan Section 3.36 — GlobalGroup.
//! BTL refs (28 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.36.1",
        name: "Read-only Property Test",
        reference: "135.1-2025 - 7.2.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.2",
        name: "Reliability MEMBER_FAULT",
        reference: "135.1-2025 - 7.3.2.13.4",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.3",
        name: "Reliability COMM_FAILURE",
        reference: "135.1-2025 - 7.3.2.13.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.4",
        name: "PV Tracking and Reliability",
        reference: "135.1-2025 - 7.3.2.13.6",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.5",
        name: "First Stage Faults Precedence",
        reference: "135.1-2025 - 7.3.2.13.9",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.6",
        name: "PV/OOS/SF Test",
        reference: "135.1-2025 - 7.3.2.13.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.7",
        name: "Resizing Group_Member_Names",
        reference: "135.1-2025 - 7.3.2.13.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.8",
        name: "Resizing Group_Members",
        reference: "135.1-2025 - 7.3.2.13.2",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.9",
        name: "PV Tracking Test 1",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.10",
        name: "PV Tracking Test 2",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.11",
        name: "PV Tracking Test 3",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.12",
        name: "PV Tracking Test 4",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.13",
        name: "PV Tracking Test 5",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.14",
        name: "PV Tracking Test 6",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.15",
        name: "PV Tracking Test 7",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.16",
        name: "PV Tracking Test 8",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.17",
        name: "PV Tracking Test 9",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.18",
        name: "PV Tracking Test 10",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.19",
        name: "PV Tracking Test 11",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.20",
        name: "PV Tracking Test 12",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.21",
        name: "PV Tracking Test 13",
        reference: "135.1-2025 - 7.3.2.13.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.22",
        name: "COV_Resubscription_Interval",
        reference: "135.1-2025 - 7.3.1.7.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.23",
        name: "Writing Properties 1",
        reference: "135.1-2025 - 9.22.1.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.24",
        name: "Writing Properties 2",
        reference: "135.1-2025 - 9.22.1.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.25",
        name: "COVU_Period Zero",
        reference: "135.1-2025 - 7.3.2.13.8",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.26",
        name: "COVU_Recipients Notifications",
        reference: "135.1-2025 - 8.3.11",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.27",
        name: "CHANGE_OF_RELIABILITY First Stage",
        reference: "135.1-2025 - 8.5.17.14",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::from_raw(26),
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.36.28",
        name: "REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(26)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::from_raw(26),
            ))
        },
    });
}
