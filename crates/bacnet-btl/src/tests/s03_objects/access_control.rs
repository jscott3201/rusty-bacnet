//! BTL Test Plan Section 3.44-3.49 — AccessControl (6 types).
//! BTL refs (38 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.44.1",
        name: "AP: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ap"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::ACCESS_POINT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.2",
        name: "AP: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ap"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ACCESS_POINT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.3",
        name: "AP: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.33.2",
        section: Section::Objects,
        tags: &["objects", "ap"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.4",
        name: "AP: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.33.3",
        section: Section::Objects,
        tags: &["objects", "ap"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.5",
        name: "AP: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.33.4",
        section: Section::Objects,
        tags: &["objects", "ap"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.6",
        name: "AP: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.33.5",
        section: Section::Objects,
        tags: &["objects", "ap"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_POINT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.7",
        name: "AZ: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "az"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(34)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::ACCESS_ZONE)),
    });
    registry.add(TestDef {
        id: "3.44.8",
        name: "AZ: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "az"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(34)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ACCESS_ZONE,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.9",
        name: "AZ: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.34.2",
        section: Section::Objects,
        tags: &["objects", "az"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(34)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.10",
        name: "AZ: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.34.3",
        section: Section::Objects,
        tags: &["objects", "az"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(34)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.11",
        name: "AZ: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.34.4",
        section: Section::Objects,
        tags: &["objects", "az"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(34)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.12",
        name: "AZ: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.34.5",
        section: Section::Objects,
        tags: &["objects", "az"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(34)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_ZONE,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.13",
        name: "AU: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "au"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(35)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, ObjectType::ACCESS_USER)),
    });
    registry.add(TestDef {
        id: "3.44.14",
        name: "AU: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "au"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(35)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ACCESS_USER,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.15",
        name: "AU: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.35.2",
        section: Section::Objects,
        tags: &["objects", "au"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(35)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_USER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.16",
        name: "AU: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.35.3",
        section: Section::Objects,
        tags: &["objects", "au"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(35)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_USER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.17",
        name: "AU: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.35.4",
        section: Section::Objects,
        tags: &["objects", "au"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(35)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_USER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.18",
        name: "AU: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.35.5",
        section: Section::Objects,
        tags: &["objects", "au"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(35)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_USER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.19",
        name: "AR: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(36)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::ACCESS_RIGHTS,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.20",
        name: "AR: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(36)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ACCESS_RIGHTS,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.21",
        name: "AR: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.36.2",
        section: Section::Objects,
        tags: &["objects", "ar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(36)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_RIGHTS,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.22",
        name: "AR: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.36.3",
        section: Section::Objects,
        tags: &["objects", "ar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(36)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_RIGHTS,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.23",
        name: "AR: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.36.4",
        section: Section::Objects,
        tags: &["objects", "ar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(36)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_RIGHTS,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.24",
        name: "AR: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.36.5",
        section: Section::Objects,
        tags: &["objects", "ar"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(36)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_RIGHTS,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.25",
        name: "AC: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "ac"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(32)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::ACCESS_CREDENTIAL,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.26",
        name: "AC: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "ac"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(32)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ACCESS_CREDENTIAL,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.27",
        name: "AC: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.32.2",
        section: Section::Objects,
        tags: &["objects", "ac"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(32)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_CREDENTIAL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.28",
        name: "AC: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.32.3",
        section: Section::Objects,
        tags: &["objects", "ac"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(32)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_CREDENTIAL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.29",
        name: "AC: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.32.4",
        section: Section::Objects,
        tags: &["objects", "ac"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(32)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_CREDENTIAL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.30",
        name: "AC: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.32.5",
        section: Section::Objects,
        tags: &["objects", "ac"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(32)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCESS_CREDENTIAL,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.31",
        name: "CDI: OOS/SF/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_oos_status_flags(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.32",
        name: "CDI: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.33",
        name: "CDI: Object-Specific Test 2",
        reference: "135.1-2025 - 7.3.2.37.2",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.34",
        name: "CDI: Object-Specific Test 3",
        reference: "135.1-2025 - 7.3.2.37.3",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.35",
        name: "CDI: Object-Specific Test 4",
        reference: "135.1-2025 - 7.3.2.37.4",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.36",
        name: "CDI: Object-Specific Test 5",
        reference: "135.1-2025 - 7.3.2.37.5",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.37",
        name: "CDI: Object-Specific Test 6",
        reference: "135.1-2025 - 7.3.2.37.6",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.44.38",
        name: "CDI: Object-Specific Test 7",
        reference: "135.1-2025 - 7.3.2.37.7",
        section: Section::Objects,
        tags: &["objects", "cdi"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(37)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::CREDENTIAL_DATA_INPUT,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
}
