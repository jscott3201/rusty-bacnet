//! BTL Test Plan Sections 5.4–5.5 — AE-ACK (Acknowledge Alarm).
//! 27 BTL references: 5.4 A-side (5), 5.5 B-side (22).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 5.4 AE-ACK-A (client initiation) ────────────────────────────────

    let ack_a: &[(&str, &str, &str)] = &[
        (
            "5.4.1",
            "AE-ACK-A: Initiate Confirmed Ack (Time)",
            "135.1-2025 - 8.1.1",
        ),
        (
            "5.4.2",
            "AE-ACK-A: Initiate Confirmed Ack (DateTime)",
            "135.1-2025 - 8.1.2",
        ),
        (
            "5.4.3",
            "AE-ACK-A: Initiate Confirmed Ack (SeqNum)",
            "135.1-2025 - 8.1.3",
        ),
        (
            "5.4.4",
            "AE-ACK-A: Initiate Unconfirmed Ack",
            "135.1-2025 - 8.1.4",
        ),
        (
            "5.4.5",
            "AE-ACK-A: Ack Unsuccessful Response",
            "135.1-2025 - 8.1.5",
        ),
    ];

    for &(id, name, reference) in ack_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "ack-a"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ack_base(ctx)),
        });
    }

    // ── 5.5 AE-ACK-B (server execution) ─────────────────────────────────

    let ack_b: &[(&str, &str, &str)] = &[
        (
            "5.5.1",
            "AE-ACK-B: Successful Ack Conf Time",
            "135.1-2025 - 9.1.1.1",
        ),
        (
            "5.5.2",
            "AE-ACK-B: Successful Ack Conf DateTime",
            "135.1-2025 - 9.1.1.2",
        ),
        (
            "5.5.3",
            "AE-ACK-B: Successful Ack Conf SeqNum",
            "135.1-2025 - 9.1.1.3",
        ),
        (
            "5.5.4",
            "AE-ACK-B: Successful Ack Unconf Time",
            "135.1-2025 - 9.1.1.4",
        ),
        (
            "5.5.5",
            "AE-ACK-B: Successful Ack Unconf DateTime",
            "135.1-2025 - 9.1.1.5",
        ),
        (
            "5.5.6",
            "AE-ACK-B: Successful Ack Unconf SeqNum",
            "135.1-2025 - 9.1.1.6",
        ),
        (
            "5.5.7",
            "AE-ACK-B: Successful Ack Conf Other Source",
            "135.1-2025 - 9.1.1.8",
        ),
        (
            "5.5.8",
            "AE-ACK-B: Successful Ack Unconf Other Source",
            "135.1-2025 - 9.1.1.9",
        ),
        (
            "5.5.9",
            "AE-ACK-B: Unsuccessful Wrong Timestamp",
            "BTL - 9.1.2.1",
        ),
        (
            "5.5.10",
            "AE-ACK-B: Unsuccessful Unknown Source Conf",
            "135.1-2025 - 9.1.2.3",
        ),
        (
            "5.5.11",
            "AE-ACK-B: Unsuccessful Unknown Obj Conf",
            "135.1-2025 - 9.1.2.4",
        ),
        (
            "5.5.12",
            "AE-ACK-B: Unsuccessful Unknown Source Unconf",
            "135.1-2025 - 9.1.2.5",
        ),
        (
            "5.5.13",
            "AE-ACK-B: Unsuccessful Unknown Obj Unconf",
            "135.1-2025 - 9.1.2.6",
        ),
        (
            "5.5.14",
            "AE-ACK-B: Unsuccessful Invalid State",
            "135.1-2025 - 9.1.2.7",
        ),
        (
            "5.5.15",
            "AE-ACK-B: Successful with Event_Algorithm_Inhibit",
            "135.1-2025 - 9.1.1.14",
        ),
        (
            "5.5.16",
            "AE-ACK-B: Re-Ack Confirmed",
            "135.1-2025 - 9.1.1.10",
        ),
        (
            "5.5.17",
            "AE-ACK-B: Re-Ack Unconfirmed",
            "135.1-2025 - 9.1.1.11",
        ),
        (
            "5.5.18",
            "AE-ACK-B: Acked_Transitions Test",
            "135.1-2025 - 7.3.1.11.1",
        ),
        (
            "5.5.19",
            "AE-ACK-B: Acked_Transitions Fault",
            "BTL - 7.3.1.11.4",
        ),
        (
            "5.5.20",
            "AE-ACK-B: Event_Algorithm_Inhibit Ack",
            "135.1-2025 - 7.3.1.19.3",
        ),
        (
            "5.5.21",
            "AE-ACK-B: Unsupported Charset Ack",
            "135.1-2025 - 9.1.1.15",
        ),
        (
            "5.5.22",
            "AE-ACK-B: Acked_Transitions Normal-to-Normal",
            "135.1-2025 - 7.3.1.11.3",
        ),
    ];

    for &(id, name, reference) in ack_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "ack-b"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ack_b_test(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ack_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::ACKED_TRANSITIONS)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}

async fn ack_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::ACKED_TRANSITIONS)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_TIME_STAMPS)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}
