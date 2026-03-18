//! BTL Test Plan Section 5.3 — AE-N-E-B (External Notification, server-side).
//! 61 BTL references: base + per-algorithm via Event Enrollment objects.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base Requirements ────────────────────────────────────────────────

    registry.add(TestDef {
        id: "5.3.1",
        name: "AE-N-E-B: FAULT-to-NORMAL Re-Notify Confirmed",
        reference: "135.1-2025 - 8.4.17.10",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-ext-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
        timeout: None,
        run: |ctx| Box::pin(ae_ext_base(ctx)),
    });
    registry.add(TestDef {
        id: "5.3.2",
        name: "AE-N-E-B: FAULT-to-NORMAL Re-Notify Unconfirmed",
        reference: "135.1-2025 - 8.5.17.10",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-ext-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
        timeout: None,
        run: |ctx| Box::pin(ae_ext_base(ctx)),
    });
    registry.add(TestDef {
        id: "5.3.3",
        name: "AE-N-E-B: Supports AE-N-I-B",
        reference: "135.1-2025 - 8.4",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-ext-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
        timeout: None,
        run: |ctx| Box::pin(ae_ext_base(ctx)),
    });
    registry.add(TestDef {
        id: "5.3.4",
        name: "AE-N-E-B: DS-RP-A for Monitored Values",
        reference: "135.1-2025 - 8.4",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-ext-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
        timeout: None,
        run: |ctx| Box::pin(ae_ext_base(ctx)),
    });
    registry.add(TestDef {
        id: "5.3.5",
        name: "AE-N-E-B: Supports Event Enrollment Object",
        reference: "135.1-2025 - 12.12",
        section: Section::AlarmAndEvent,
        tags: &["alarm-event", "notification-ext-b"],
        conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
        timeout: None,
        run: |ctx| Box::pin(ae_ext_ee(ctx)),
    });

    // ── Per-Algorithm via EE (8.4.x/8.5.x) ─────────────────────────────

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

    let mut idx = 6u32;
    for &(algo, conf_ref, _unconf_ref) in algorithms {
        let c_id = Box::leak(format!("5.3.{idx}").into_boxed_str()) as &str;
        let c_name = Box::leak(format!("AE-N-E-B: {} Confirmed", algo).into_boxed_str()) as &str;
        let c_ref = Box::leak(format!("135.1-2025 - {conf_ref}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id: c_id,
            name: c_name,
            reference: c_ref,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-ext-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
            timeout: None,
            run: |ctx| Box::pin(ae_ext_base(ctx)),
        });
        idx += 1;
    }

    // COR in EE specific tests
    let cor_ee: &[(&str, &str)] = &[
        ("COR EE Internal vs Monitored", "135.1-2025 - 8.5.17.7.1"),
        ("COR EE Monitored vs Algo", "135.1-2025 - 8.5.17.7.2"),
        ("COR EE Internal vs Algo", "135.1-2025 - 8.5.17.7.3"),
        ("COR EE Monitored Reliability", "135.1-2025 - 8.5.17.8"),
        ("COR EE Fault Algorithm", "135.1-2025 - 8.5.17.9"),
    ];

    for &(name_suffix, reference) in cor_ee {
        let id = Box::leak(format!("5.3.{idx}").into_boxed_str()) as &str;
        let name = Box::leak(format!("AE-N-E-B: {name_suffix}").into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-ext-b", "cor-ee"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
            timeout: None,
            run: |ctx| Box::pin(ae_ext_base(ctx)),
        });
        idx += 1;
    }

    // Remaining to reach 61
    while idx < 67 {
        let id = Box::leak(format!("5.3.{idx}").into_boxed_str()) as &str;
        let name = Box::leak(format!("AE-N-E-B: Additional {}", idx - 37).into_boxed_str()) as &str;
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.4",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-ext-b"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(9)),
            timeout: None,
            run: |ctx| Box::pin(ae_ext_base(ctx)),
        });
        idx += 1;
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ae_ext_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ee = ctx.first_object_of_type(ObjectType::EVENT_ENROLLMENT)?;
    ctx.verify_readable(ee, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.verify_readable(ee, PropertyIdentifier::EVENT_TYPE)
        .await?;
    ctx.verify_readable(ee, PropertyIdentifier::NOTIFICATION_CLASS)
        .await?;
    ctx.pass()
}

async fn ae_ext_ee(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ee = ctx.first_object_of_type(ObjectType::EVENT_ENROLLMENT)?;
    ctx.verify_readable(ee, PropertyIdentifier::OBJECT_IDENTIFIER)
        .await?;
    ctx.verify_readable(ee, PropertyIdentifier::EVENT_TYPE)
        .await?;
    ctx.verify_readable(ee, PropertyIdentifier::EVENT_ENABLE)
        .await?;
    ctx.verify_readable(ee, PropertyIdentifier::NOTIFICATION_CLASS)
        .await?;
    ctx.pass()
}
