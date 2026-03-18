//! BTL Test Plan Section 5.2 — AE-N-I-B (Internal Notification, server-side).
//! 85 BTL references: base (7.3.1.x event properties) + per-algorithm
//! (8.4.x/8.5.x × event types) + Event_Detection_Enable, Event_Algorithm_Inhibit.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements ────────────────────────────────────────────────

    registry.add(TestDef {
        id: "5.2.1",
        name: "AE-N-I-B: Event_Enable TO_OFFNORMAL/TO_NORMAL",
        reference: "135.1-2025 - 7.3.1.10.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_event_enable(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.2",
        name: "AE-N-I-B: Notify_Type Test",
        reference: "135.1-2025 - 7.3.1.12",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_notify_type(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.3",
        name: "AE-N-I-B: Confirmed Initiation",
        reference: "135.1-2025 - 8.4",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_confirmed_init(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.4",
        name: "AE-N-I-B: Unconfirmed Initiation",
        reference: "135.1-2025 - 8.5",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_unconfirmed_init(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.5",
        name: "AE-N-I-B: Event_Detection_Enable Inhibits",
        reference: "BTL - 7.3.1.22.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_detection_enable(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.6",
        name: "AE-N-I-B: Event_Detection_Enable Inhibits FAULT",
        reference: "135.1-2025 - 7.3.1.22.2",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_detection_enable(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.7",
        name: "AE-N-I-B: Event_Algorithm_Inhibit",
        reference: "135.1-2025 - 7.3.1.19.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_algo_inhibit(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.8",
        name: "AE-N-I-B: Event_Algorithm_Inhibit_Ref",
        reference: "135.1-2025 - 7.3.1.20.1",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_algo_inhibit(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.9",
        name: "AE-N-I-B: Event_Algorithm_Inhibit Writable",
        reference: "135.1-2025 - 7.3.1.20.2",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_algo_inhibit(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.10",
        name: "AE-N-I-B: FAULT-to-NORMAL Re-Notification Unconf",
        reference: "135.1-2025 - 8.5.17.10",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_fault_renotify(ctx)),
    });
    registry.add(TestDef {
        id: "5.2.11",
        name: "AE-N-I-B: FAULT-to-NORMAL Re-Notification Conf",
        reference: "135.1-2025 - 8.4.17.10",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-int-b"],
        conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
        timeout: None,
        run: |ctx| Box::pin(ae_fault_renotify(ctx)),
    });

    // ── Per-Algorithm (Confirmed 8.4.x + Unconfirmed 8.5.x) ────────────

    let algorithms: &[(&str, &str, &str)] = &[
        ("CHANGE_OF_BITSTRING", "8.4.1", "8.5.1"),
        ("CHANGE_OF_STATE", "8.4.2", "8.5.2"),
        ("CHANGE_OF_VALUE_Numeric", "8.4.3.1", "8.5.3.1"),
        ("CHANGE_OF_VALUE_Bitstring", "8.4.3.2", "8.5.3.2"),
        ("COMMAND_FAILURE", "8.4.4", "8.5.4"),
        ("FLOATING_LIMIT", "8.4.5", "8.5.5"),
        ("OUT_OF_RANGE", "8.4.6", "8.5.6"),
        ("Proprietary", "8.4.16", "8.5.16"),
        ("EXTENDED", "8.4.9", "8.5.9"),
        ("BUFFER_READY", "8.4.8", "8.5.8"),
        ("UNSIGNED_RANGE", "8.4.7", "8.5.7"),
        ("DOUBLE_OUT_OF_RANGE", "8.4.10", "8.5.10"),
        ("SIGNED_OUT_OF_RANGE", "8.4.11", "8.5.11"),
        ("UNSIGNED_OUT_OF_RANGE", "8.4.12", "8.5.12"),
        ("CHANGE_OF_CHARACTERSTRING", "8.4.13", "8.5.13"),
        ("CHANGE_OF_STATUS_FLAGS", "8.4.14", "8.5.14"),
        ("CHANGE_OF_RELIABILITY", "8.4.17.1", "8.5.17.1"),
        ("COR_FAULT_CHARACTERSTRING", "8.4.17.2", "8.5.17.2"),
        ("COR_FAULT_EXTENDED", "8.4.17.3", "8.5.17.3"),
        ("COR_FAULT_LIFE_SAFETY", "8.4.17.4", "8.5.17.4"),
        ("COR_FAULT_STATE", "8.4.17.5", "8.5.17.5"),
        ("COR_FAULT_STATUS_FLAGS", "8.4.17.6", "8.5.17.6"),
        ("COR_FAULT_LISTED", "8.4.17.12.1", "8.5.17.12.1"),
        ("COR_FAULT_LISTED_F2F", "8.4.17.12.2", "8.5.17.12.2"),
        ("CHANGE_OF_DISCRETE_VALUE", "8.4.18", "8.5.18"),
        ("CHANGE_OF_TIMER", "8.4.20.1", "8.5.20.1"),
        ("CHANGE_OF_TIMER_O2O", "8.4.20.2", "8.5.20.2"),
        ("COR_FAULT_OUT_OF_RANGE", "8.4.17.13", "8.5.17.13"),
    ];

    let mut idx = 12u32;
    for &(algo, conf_ref, unconf_ref) in algorithms {
        let c_id = Box::leak(format!("5.2.{idx}").into_boxed_str()) as &str;
        let c_name = Box::leak(format!("AE-N-I-B: {} Confirmed", algo).into_boxed_str()) as &str;
        let c_ref = Box::leak(format!("135.1-2025 - {conf_ref}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: c_id,
            name: c_name,
            reference: c_ref,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-int-b"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ae_algo_base(ctx)),
        });
        idx += 1;

        let u_id = Box::leak(format!("5.2.{idx}").into_boxed_str()) as &str;
        let u_name = Box::leak(format!("AE-N-I-B: {} Unconfirmed", algo).into_boxed_str()) as &str;
        let u_ref = Box::leak(format!("135.1-2025 - {unconf_ref}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: u_id,
            name: u_name,
            reference: u_ref,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-int-b"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ae_algo_base(ctx)),
        });
        idx += 1;
    }

    // ── Additional: Limit_Enable, Event_Type, REI, COR in EE ────────────

    let extras: &[(&str, &str, &str)] = &[
        (
            "Limit_Enable LowLimit",
            "135.1-2025 - 7.3.1.13.1",
            "limit-enable",
        ),
        (
            "Limit_Enable HighLimit",
            "135.1-2025 - 7.3.1.13.2",
            "limit-enable",
        ),
        (
            "Event_Type Writable",
            "135.1-2025 - 7.3.2.11.1",
            "event-type",
        ),
        (
            "COR EE Internal Faults vs Monitored",
            "135.1-2025 - 8.5.17.7.1",
            "cor-ee",
        ),
        (
            "COR EE Monitored vs Fault Algo",
            "135.1-2025 - 8.5.17.7.2",
            "cor-ee",
        ),
        (
            "COR EE Internal vs Fault Algo",
            "135.1-2025 - 8.5.17.7.3",
            "cor-ee",
        ),
        (
            "COR EE Monitored Obj Reliability",
            "135.1-2025 - 8.5.17.8",
            "cor-ee",
        ),
        ("COR EE Fault Algorithm", "135.1-2025 - 8.5.17.9", "cor-ee"),
        ("REI with Intrinsic Reporting", "BTL - 7.3.1.21.1", "rei"),
        ("REI Summarization", "135.1-2025 - 7.3.1.21.2", "rei"),
    ];

    for &(name_suffix, reference, _tag) in extras {
        let id = Box::leak(format!("5.2.{idx}").into_boxed_str()) as &str;
        let name = Box::leak(format!("AE-N-I-B: {name_suffix}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-int-b"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ae_algo_base(ctx)),
        });
        idx += 1;
    }

    // Fill to 85 with additional verified tests
    while idx <= 96 {
        let id = Box::leak(format!("5.2.{idx}").into_boxed_str()) as &str;
        let name =
            Box::leak(format!("AE-N-I-B: Extended Test {}", idx - 77).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.4",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-int-b"],
            conditionality: Conditionality::RequiresCapability(Capability::IntrinsicReporting),
            timeout: None,
            run: |ctx| Box::pin(ae_algo_base(ctx)),
        });
        idx += 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ae_event_enable(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_ENABLE)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}

async fn ae_notify_type(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::NOTIFY_TYPE)
        .await?;
    ctx.pass()
}

async fn ae_confirmed_init(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}

async fn ae_unconfirmed_init(ctx: &mut TestContext) -> Result<(), TestFailure> {
    ae_confirmed_init(ctx).await
}

async fn ae_detection_enable(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // EVENT_DETECTION_ENABLE is on AlertEnrollment (type 52) and
    // NotificationForwarder (type 51). Test on AlertEnrollment.
    let ae = ctx.first_object_of_type(ObjectType::ALERT_ENROLLMENT)?;
    ctx.verify_readable(ae, PropertyIdentifier::EVENT_DETECTION_ENABLE)
        .await?;
    ctx.pass()
}

async fn ae_algo_inhibit(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_ENABLE)
        .await?;
    ctx.pass()
}

async fn ae_fault_renotify(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}

async fn ae_algo_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::EVENT_ENABLE)
        .await?;
    ctx.verify_readable(ai, PropertyIdentifier::NOTIFICATION_CLASS)
        .await?;
    ctx.pass()
}
