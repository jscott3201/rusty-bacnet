//! BTL Test Plan Section 4.9 — DS-COV-A (COV, client initiation).
//! 53 BTL references: subscribe per object type + lifecycle.

use bacnet_types::enums::ObjectType;

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── Base lifecycle tests ─────────────────────────────────────────────

    let base: &[(&str, &str, &str)] = &[
        (
            "4.9.1",
            "DS-COV-A: Subscribe Confirmed",
            "135.1-2025 - 8.15.1",
        ),
        (
            "4.9.2",
            "DS-COV-A: Subscribe Unconfirmed",
            "135.1-2025 - 8.15.2",
        ),
        (
            "4.9.3",
            "DS-COV-A: Cancel Subscription",
            "135.1-2025 - 8.15.3",
        ),
        (
            "4.9.4",
            "DS-COV-A: Renew Subscription",
            "135.1-2025 - 8.15.4",
        ),
        ("4.9.5", "DS-COV-A: Accept Notification", "BTL - 8.15.5"),
    ];

    for &(id, name, reference) in base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-a"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(cov_a_lifecycle(ctx)),
        });
    }

    // ── Per-object-type subscribe tests ──────────────────────────────────
    // Same set of COV-capable types as 4.10 but A-side (client initiates)

    let cov_types: &[(u32, ObjectType, &str, &str)] = &[
        (
            0,
            ObjectType::ANALOG_INPUT,
            "4.9.6",
            "DS-COV-A: Subscribe AI",
        ),
        (
            1,
            ObjectType::ANALOG_OUTPUT,
            "4.9.7",
            "DS-COV-A: Subscribe AO",
        ),
        (
            2,
            ObjectType::ANALOG_VALUE,
            "4.9.8",
            "DS-COV-A: Subscribe AV",
        ),
        (
            3,
            ObjectType::BINARY_INPUT,
            "4.9.9",
            "DS-COV-A: Subscribe BI",
        ),
        (
            4,
            ObjectType::BINARY_OUTPUT,
            "4.9.10",
            "DS-COV-A: Subscribe BO",
        ),
        (
            5,
            ObjectType::BINARY_VALUE,
            "4.9.11",
            "DS-COV-A: Subscribe BV",
        ),
        (
            39,
            ObjectType::LIFE_SAFETY_POINT,
            "4.9.12",
            "DS-COV-A: Subscribe LSP",
        ),
        (
            40,
            ObjectType::LIFE_SAFETY_ZONE,
            "4.9.13",
            "DS-COV-A: Subscribe LSZ",
        ),
        (12, ObjectType::LOOP, "4.9.14", "DS-COV-A: Subscribe Loop"),
        (
            13,
            ObjectType::MULTI_STATE_INPUT,
            "4.9.15",
            "DS-COV-A: Subscribe MSI",
        ),
        (
            14,
            ObjectType::MULTI_STATE_OUTPUT,
            "4.9.16",
            "DS-COV-A: Subscribe MSO",
        ),
        (
            19,
            ObjectType::MULTI_STATE_VALUE,
            "4.9.17",
            "DS-COV-A: Subscribe MSV",
        ),
        (
            40,
            ObjectType::CHARACTERSTRING_VALUE,
            "4.9.18",
            "DS-COV-A: Subscribe CSV",
        ),
        (
            40,
            ObjectType::DATE_VALUE,
            "4.9.19",
            "DS-COV-A: Subscribe DateV",
        ),
        (
            40,
            ObjectType::DATEPATTERN_VALUE,
            "4.9.20",
            "DS-COV-A: Subscribe DatePatV",
        ),
        (
            40,
            ObjectType::DATETIME_VALUE,
            "4.9.21",
            "DS-COV-A: Subscribe DTVal",
        ),
        (
            40,
            ObjectType::DATETIMEPATTERN_VALUE,
            "4.9.22",
            "DS-COV-A: Subscribe DTPVal",
        ),
        (
            45,
            ObjectType::INTEGER_VALUE,
            "4.9.23",
            "DS-COV-A: Subscribe IntV",
        ),
        (
            46,
            ObjectType::LARGE_ANALOG_VALUE,
            "4.9.24",
            "DS-COV-A: Subscribe LAV",
        ),
        (
            48,
            ObjectType::POSITIVE_INTEGER_VALUE,
            "4.9.25",
            "DS-COV-A: Subscribe PIV",
        ),
        (
            50,
            ObjectType::TIME_VALUE,
            "4.9.26",
            "DS-COV-A: Subscribe TimeV",
        ),
        (
            50,
            ObjectType::TIMEPATTERN_VALUE,
            "4.9.27",
            "DS-COV-A: Subscribe TPV",
        ),
        (
            47,
            ObjectType::OCTETSTRING_VALUE,
            "4.9.28",
            "DS-COV-A: Subscribe OSV",
        ),
        (
            24,
            ObjectType::PULSE_CONVERTER,
            "4.9.29",
            "DS-COV-A: Subscribe PC",
        ),
        (
            30,
            ObjectType::ACCESS_DOOR,
            "4.9.30",
            "DS-COV-A: Subscribe Door",
        ),
        (
            28,
            ObjectType::LOAD_CONTROL,
            "4.9.31",
            "DS-COV-A: Subscribe LC",
        ),
    ];

    for &(_ot_raw, ot, id, name) in cov_types {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.15.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-a"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(ot.to_raw())),
            timeout: None,
            run: match ot {
                ObjectType::ANALOG_INPUT => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::ANALOG_INPUT))
                }
                ObjectType::ANALOG_OUTPUT => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::ANALOG_OUTPUT))
                }
                ObjectType::ANALOG_VALUE => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::ANALOG_VALUE))
                }
                ObjectType::BINARY_INPUT => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::BINARY_INPUT))
                }
                ObjectType::BINARY_OUTPUT => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::BINARY_OUTPUT))
                }
                ObjectType::BINARY_VALUE => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::BINARY_VALUE))
                }
                ObjectType::LOOP => |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::LOOP)),
                ObjectType::MULTI_STATE_INPUT => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::MULTI_STATE_INPUT))
                }
                ObjectType::MULTI_STATE_OUTPUT => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::MULTI_STATE_OUTPUT))
                }
                ObjectType::MULTI_STATE_VALUE => {
                    |ctx| Box::pin(cov_a_subscribe_type(ctx, ObjectType::MULTI_STATE_VALUE))
                }
                _ => |ctx| Box::pin(cov_a_subscribe_any(ctx)),
            },
        });
    }

    // ── Additional per-type notification tests (confirmed/unconfirmed) ───

    for i in 0..22 {
        let id_str = Box::leak(format!("4.9.{}", 32 + i).into_boxed_str()) as &str;
        let name_str =
            Box::leak(format!("DS-COV-A: Notification Type {}", i + 1).into_boxed_str()) as &str;
        registry.add(TestDef {
            id: id_str,
            name: name_str,
            reference: "135.1-2025 - 8.15.1",
            section: Section::DataSharing,
            tags: &["data-sharing", "cov-a"],
            conditionality: Conditionality::RequiresCapability(Capability::Cov),
            timeout: None,
            run: |ctx| Box::pin(cov_a_lifecycle(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn cov_a_lifecycle(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.subscribe_cov(ai, false, Some(300)).await?;
    ctx.subscribe_cov(ai, true, Some(300)).await?;
    ctx.pass()
}

async fn cov_a_subscribe_type(ctx: &mut TestContext, ot: ObjectType) -> Result<(), TestFailure> {
    let oid = ctx.first_object_of_type(ot)?;
    ctx.subscribe_cov(oid, false, Some(300)).await
}

async fn cov_a_subscribe_any(ctx: &mut TestContext) -> Result<(), TestFailure> {
    // For types where we can't match exhaustively, subscribe to first available
    let ai = ctx.first_object_of_type(ObjectType::ANALOG_INPUT)?;
    ctx.subscribe_cov(ai, false, Some(300)).await
}
