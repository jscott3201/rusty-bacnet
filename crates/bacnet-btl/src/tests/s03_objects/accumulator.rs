//! BTL Test Plan Section 3.37+3.41 — Accumulator+PulseConverter.
//! BTL refs (15 total)

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.37.1",
        name: "ACC: PV Remains In-Range",
        reference: "135.1-2025 - 7.3.2.32.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.2",
        name: "ACC: Prescale",
        reference: "135.1-2025 - 7.3.2.32.2",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.3",
        name: "ACC: Logging_Record",
        reference: "135.1-2025 - 7.3.2.32.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.4",
        name: "ACC: Logging_Record RECOVERED",
        reference: "135.1-2025 - 7.3.2.32.4",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.5",
        name: "ACC: Logging_Record STARTING",
        reference: "135.1-2025 - 7.3.2.32.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.6",
        name: "ACC: Out_Of_Service",
        reference: "135.1-2025 - 7.3.2.32.6",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.7",
        name: "ACC: Value_Set Writing",
        reference: "135.1-2025 - 7.3.2.32.7",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.8",
        name: "ACC: Value_Before_Change Writing",
        reference: "135.1-2025 - 7.3.2.32.8",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::ACCUMULATOR,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.9",
        name: "ACC: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(23)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::ACCUMULATOR,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.10",
        name: "PC: Adjust_Value Write",
        reference: "135.1-2025 - 7.3.2.33.1",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(24)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::PULSE_CONVERTER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.11",
        name: "PC: Scale_Factor",
        reference: "135.1-2025 - 7.3.2.33.2",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(24)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::PULSE_CONVERTER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.12",
        name: "PC: Update_Time Reflects Change",
        reference: "135.1-2025 - 7.3.2.33.5",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(24)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::PULSE_CONVERTER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.13",
        name: "PC: Adjust_Value Out-of-Range",
        reference: "135.1-2025 - 7.3.2.33.6",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(24)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::PULSE_CONVERTER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.14",
        name: "PC: Out_Of_Service",
        reference: "135.1-2025 - 7.3.2.33.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(24)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_property_readable(
                ctx,
                ObjectType::PULSE_CONVERTER,
                bacnet_types::enums::PropertyIdentifier::PROPERTY_LIST,
            ))
        },
    });
    registry.add(TestDef {
        id: "3.37.15",
        name: "PC: REI",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(24)),
        timeout: None,
        run: |ctx| {
            Box::pin(helpers::test_reliability_evaluation_inhibit(
                ctx,
                ObjectType::PULSE_CONVERTER,
            ))
        },
    });
}
