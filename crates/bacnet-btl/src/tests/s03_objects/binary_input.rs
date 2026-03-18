//! BTL Test Plan Section 3.5 — Binary Input Object.
//!
//! BTL references (7 total):
//!   1. BTL - 7.3.1.1.1 — OOS/Status_Flags/Reliability
//!   2. 135.1-2025 - 7.3.2.5.3 — Polarity Property Tests
//!   3. 135.1-2025 - 7.3.1.8 — Change of State Test
//!   4. 135.1-2025 - 7.3.1.24 — Non-zero Writable State Count Test
//!   5. 135.1-2025 - 7.3.1.9 — Elapsed Active Time Tests
//!   6. 135.1-2025 - 7.3.1.25 — Non-zero Writable Elapsed Active Time Test
//!   7. 135.1-2025 - 7.3.1.21.3 — Reliability_Evaluation_Inhibit

use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::tests::helpers;
use bacnet_types::enums::ObjectType;

const OT: u32 = 3;
const T: ObjectType = ObjectType::BINARY_INPUT;

pub fn register(registry: &mut TestRegistry) {
    registry.add(TestDef {
        id: "3.5.1",
        name: "BI: OOS/Status_Flags/Reliability",
        reference: "BTL - 7.3.1.1.1",
        section: Section::Objects,
        tags: &["objects", "bi", "oos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_oos_status_flags(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.5.2",
        name: "BI: Polarity Property",
        reference: "135.1-2025 - 7.3.2.5.3",
        section: Section::Objects,
        tags: &["objects", "bi", "polarity"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_polarity(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.5.3",
        name: "BI: Change of State",
        reference: "135.1-2025 - 7.3.1.8",
        section: Section::Objects,
        tags: &["objects", "bi", "cos"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_change_of_state(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.5.4",
        name: "BI: Non-zero Writable State Count",
        reference: "135.1-2025 - 7.3.1.24",
        section: Section::Objects,
        tags: &["objects", "bi", "state-count"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_state_count_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.5.5",
        name: "BI: Elapsed Active Time",
        reference: "135.1-2025 - 7.3.1.9",
        section: Section::Objects,
        tags: &["objects", "bi", "elapsed"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_elapsed_active_time(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.5.6",
        name: "BI: Writable Elapsed Active Time",
        reference: "135.1-2025 - 7.3.1.25",
        section: Section::Objects,
        tags: &["objects", "bi", "elapsed"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_elapsed_active_time_writable(ctx, T)),
    });
    registry.add(TestDef {
        id: "3.5.7",
        name: "BI: Reliability_Evaluation_Inhibit",
        reference: "135.1-2025 - 7.3.1.21.3",
        section: Section::Objects,
        tags: &["objects", "bi", "rei"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(OT)),
        timeout: None,
        run: |ctx| Box::pin(helpers::test_reliability_evaluation_inhibit(ctx, T)),
    });
}
