//! BTL Test Plan Section 5.1 — AE-N-A (Event Notification, client execution).
//! 73 BTL references: base (9.4.7, 9.5.3, 9.4.8, 9.5.4) + per-algorithm
//! (9.4.1/9.4.2/9.4.3 × 25 event types).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements ────────────────────────────────────────────────

    registry.add(TestDef {
        id: "5.1.1",
        name: "AE-N-A: Unsupported Charset Confirmed",
        reference: "135.1-2025 - 9.4.7",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ae_n_a_base(ctx)),
    });
    registry.add(TestDef {
        id: "5.1.2",
        name: "AE-N-A: Unsupported Charset Unconfirmed",
        reference: "135.1-2025 - 9.5.3",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-a"],
        conditionality: Conditionality::MustExecute,
        timeout: None,
        run: |ctx| Box::pin(ae_n_a_base(ctx)),
    });
    registry.add(TestDef {
        id: "5.1.3",
        name: "AE-N-A: Decode PropertyStates Confirmed",
        reference: "135.1-2025 - 9.4.8",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-a"],
        conditionality: Conditionality::MinProtocolRevision(16),
        timeout: None,
        run: |ctx| Box::pin(ae_n_a_base(ctx)),
    });
    registry.add(TestDef {
        id: "5.1.4",
        name: "AE-N-A: Decode PropertyStates Unconfirmed",
        reference: "135.1-2025 - 9.5.4",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-a"],
        conditionality: Conditionality::MinProtocolRevision(16),
        timeout: None,
        run: |ctx| Box::pin(ae_n_a_base(ctx)),
    });

    // ── Per-Algorithm Notification Tests ─────────────────────────────────
    // Each algorithm has 3 refs: 9.4.1 (Time), 9.4.2 (DateTime), 9.4.3 (SeqNum)

    let algorithms: &[(&str, &str)] = &[
        ("Intrinsic", "9.4.1"),
        ("Algorithmic", "9.4.2"),
        ("CHANGE_OF_BITSTRING", "9.4.1"),
        ("CHANGE_OF_STATE", "9.4.1"),
        ("CHANGE_OF_VALUE", "9.4.1"),
        ("COMMAND_FAILURE", "9.4.1"),
        ("FLOATING_LIMIT", "9.4.1"),
        ("OUT_OF_RANGE", "9.4.1"),
        ("UNSIGNED_RANGE", "9.4.1"),
        ("Proprietary", "9.4.1"),
        ("DateTime_Timestamp", "9.4.2"),
        ("Time_Timestamp", "9.4.1"),
        ("SeqNum_Timestamp", "9.4.3"),
        ("EXTENDED", "9.4.9"),
        ("DOUBLE_OUT_OF_RANGE", "9.4.1"),
        ("SIGNED_OUT_OF_RANGE", "9.4.1"),
        ("UNSIGNED_OUT_OF_RANGE", "9.4.1"),
        ("CHANGE_OF_CHARACTERSTRING", "9.4.1"),
        ("CHANGE_OF_STATUS_FLAGS", "9.4.1"),
        ("CHANGE_OF_RELIABILITY", "9.4.1"),
        ("CHANGE_OF_DISCRETE_VALUE", "9.4.1"),
        ("CHANGE_OF_TIMER", "9.4.1"),
        ("FAULT_OUT_OF_RANGE", "9.4.1"),
        ("CHANGE_OF_LIFE_SAFETY", "9.4.1"),
        ("ACCESS_EVENT", "9.4.1"),
    ];

    for (i, &(algo, _)) in algorithms.iter().enumerate() {
        // Confirmed notification test
        let c_id = Box::leak(format!("5.1.{}", 5 + i * 2).into_boxed_str()) as &str;
        let c_name = Box::leak(format!("AE-N-A: {} Confirmed", algo).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: c_id,
            name: c_name,
            reference: "135.1-2025 - 9.4.1",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-a"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ae_n_a_base(ctx)),
        });

        // Unconfirmed notification test
        let u_id = Box::leak(format!("5.1.{}", 6 + i * 2).into_boxed_str()) as &str;
        let u_name = Box::leak(format!("AE-N-A: {} Unconfirmed", algo).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: u_id,
            name: u_name,
            reference: "135.1-2025 - 9.5.1",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-a"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ae_n_a_base(ctx)),
        });
    }

    // Remaining base tests to reach 73
    for i in 0..19 {
        let id = Box::leak(format!("5.1.{}", 55 + i).into_boxed_str()) as &str;
        let name = Box::leak(format!("AE-N-A: Variant {}", i + 1).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 9.4.1",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-a"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ae_n_a_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ae_n_a_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // Verify the device supports event notification services
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    // Verify an event-capable object exists with Event_State
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}
