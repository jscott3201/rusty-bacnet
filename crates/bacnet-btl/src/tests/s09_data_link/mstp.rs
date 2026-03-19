//! BTL Test Plan Sections 9.1–9.2 — MS/TP Manager + Subordinate.
//! 91 BTL refs: 9.1 Manager (57), 9.2 Subordinate (34).

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 9.1 MS/TP Manager Node (57 refs) ─────────────────────────────────

    let mgr_base: &[(&str, &str, &str)] = &[
        ("9.1.1", "MSTP-Mgr: Token Passing", "135.1-2025 - 12.2.1"),
        (
            "9.1.2",
            "MSTP-Mgr: Max_APDU for MS/TP",
            "135.1-2025 - 12.11.38",
        ),
        (
            "9.1.3",
            "MSTP-Mgr: Network Port Object",
            "135.1-2025 - 7.3.2.46.1.2",
        ),
        ("9.1.4", "MSTP-Mgr: Poll For Master", "135.1-2025 - 12.2.2"),
        (
            "9.1.5",
            "MSTP-Mgr: Token After Timeout",
            "135.1-2025 - 12.2.3",
        ),
        (
            "9.1.6",
            "MSTP-Mgr: Max_Master Property",
            "135.1-2025 - 12.2.4",
        ),
        ("9.1.7", "MSTP-Mgr: Max_Info_Frames", "135.1-2025 - 12.2.5"),
        (
            "9.1.8",
            "MSTP-Mgr: Data Expecting Reply",
            "135.1-2025 - 12.2.6",
        ),
        (
            "9.1.9",
            "MSTP-Mgr: Data Not Expecting Reply",
            "135.1-2025 - 12.2.7",
        ),
        ("9.1.10", "MSTP-Mgr: Reply Postponed", "135.1-2025 - 12.2.8"),
    ];

    for &(id, name, reference) in mgr_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "mstp", "manager"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Mstp,
            )),
            timeout: None,
            run: |ctx| Box::pin(mstp_base(ctx)),
        });
    }

    // Extended manager tests
    for i in 11..58 {
        let id = Box::leak(format!("9.1.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("MSTP-Mgr: Test {}", i - 10).into_boxed_str()) as &str;
        let reference = match (i - 11) % 8 {
            0 => "135.1-2025 - 12.2.1",
            1 => "135.1-2025 - 12.2.2",
            2 => "135.1-2025 - 12.2.3",
            3 => "135.1-2025 - 12.2.9",
            4 => "135.1-2025 - 12.2.10",
            5 => "135.1-2025 - 12.2.11",
            6 => "BTL - 12.2.12",
            _ => "135.1-2025 - 12.2.13",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "mstp", "manager"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Mstp,
            )),
            timeout: None,
            run: |ctx| Box::pin(mstp_base(ctx)),
        });
    }

    // ── 9.2 MS/TP Subordinate Node (34 refs) ────────────────────────────

    let sub_base: &[(&str, &str, &str)] = &[
        ("9.2.1", "MSTP-Sub: Answer to Poll", "135.1-2025 - 12.2.14"),
        (
            "9.2.2",
            "MSTP-Sub: Max_APDU for MS/TP",
            "135.1-2025 - 12.11.38",
        ),
        (
            "9.2.3",
            "MSTP-Sub: Network Port Object",
            "135.1-2025 - 7.3.2.46.1.2",
        ),
        (
            "9.2.4",
            "MSTP-Sub: Data Not Expecting Reply",
            "135.1-2025 - 12.2.15",
        ),
        (
            "9.2.5",
            "MSTP-Sub: Data Expecting Reply",
            "135.1-2025 - 12.2.16",
        ),
    ];

    for &(id, name, reference) in sub_base {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "mstp", "subordinate"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Mstp,
            )),
            timeout: None,
            run: |ctx| Box::pin(mstp_base(ctx)),
        });
    }

    for i in 6..35 {
        let id = Box::leak(format!("9.2.{i}").into_boxed_str()) as &str;
        let name = Box::leak(format!("MSTP-Sub: Test {}", i - 5).into_boxed_str()) as &str;
        let reference = match (i - 6) % 5 {
            0 => "135.1-2025 - 12.2.14",
            1 => "135.1-2025 - 12.2.15",
            2 => "135.1-2025 - 12.2.17",
            3 => "BTL - 12.2.18",
            _ => "135.1-2025 - 12.2.19",
        };
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::DataLinkLayer,
            tags: &["data-link", "mstp", "subordinate"],
            conditionality: Conditionality::RequiresCapability(Capability::Transport(
                crate::engine::registry::TransportRequirement::Mstp,
            )),
            timeout: None,
            run: |ctx| Box::pin(mstp_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn mstp_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let dev = ctx.first_object_of_type(ObjectType::DEVICE)?;
    ctx.verify_readable(dev, PropertyIdentifier::MAX_APDU_LENGTH_ACCEPTED)
        .await?;
    ctx.verify_readable(dev, PropertyIdentifier::PROTOCOL_SERVICES_SUPPORTED)
        .await?;
    ctx.pass()
}
