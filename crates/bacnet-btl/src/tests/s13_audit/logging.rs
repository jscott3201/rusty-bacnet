//! BTL Test Plan Section 13.1 — AR-LOG-A (Audit Logging, client-side).
//! 25 BTL references: buffer access, ReadRange, enable, combining, hierarchy.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    let tests: &[(&str, &str, &str)] = &[
        (
            "13.1.1",
            "AR-LOG-A: One Log Holds Object History",
            "135.1-2025 - 7.3.2.48.1",
        ),
        (
            "13.1.2",
            "AR-LOG-A: ReadRange All Items",
            "135.1-2025 - 9.21.1.1",
        ),
        ("13.1.3", "AR-LOG-A: Enable Test", "135.1-2025 - 7.3.2.24.1"),
        ("13.1.4", "AR-LOG-A: Buffer_Size", "135.1-2025 - 7.3.2.24.7"),
        (
            "13.1.5",
            "AR-LOG-A: Record_Count",
            "135.1-2025 - 7.3.2.24.8",
        ),
        (
            "13.1.6",
            "AR-LOG-A: Total_Record_Count",
            "135.1-2025 - 7.3.2.24.9",
        ),
        (
            "13.1.7",
            "AR-LOG-A: RR Position Positive",
            "135.1-2025 - 9.21.1.2",
        ),
        (
            "13.1.8",
            "AR-LOG-A: RR Position Negative",
            "135.1-2025 - 9.21.1.3",
        ),
        ("13.1.9", "AR-LOG-A: RR by Time", "135.1-2025 - 9.21.1.4"),
        (
            "13.1.10",
            "AR-LOG-A: RR by Time Negative",
            "135.1-2025 - 9.21.1.4.1",
        ),
        (
            "13.1.11",
            "AR-LOG-A: RR Sequence Positive",
            "135.1-2025 - 9.21.1.9",
        ),
        (
            "13.1.12",
            "AR-LOG-A: RR Sequence Negative",
            "135.1-2025 - 9.21.1.10",
        ),
        (
            "13.1.13",
            "AR-LOG-A: RR Empty Sequence",
            "135.1-2025 - 9.21.1.7",
        ),
        (
            "13.1.14",
            "AR-LOG-A: RR Empty Time",
            "135.1-2025 - 9.21.1.8",
        ),
        (
            "13.1.15",
            "AR-LOG-A: RR MOREITEMS",
            "135.1-2025 - 9.21.1.13",
        ),
        (
            "13.1.16",
            "AR-LOG-A: Accepts from Forwarder",
            "135.1-2025 - 7.3.2.48.7",
        ),
        (
            "13.1.17",
            "AR-LOG-A: Basic Combining",
            "135.1-2025 - 7.3.2.48.2",
        ),
        (
            "13.1.18",
            "AR-LOG-A: Combining Failure",
            "135.1-2025 - 7.3.2.48.3",
        ),
        (
            "13.1.19",
            "AR-LOG-A: Non-combining",
            "135.1-2025 - 7.3.2.48.4",
        ),
        (
            "13.1.20",
            "AR-LOG-A: Combining Duplicate",
            "135.1-2025 - 7.3.2.48.5",
        ),
        (
            "13.1.21",
            "AR-LOG-A: Combining Target Value",
            "135.1-2025 - 7.3.2.48.6",
        ),
        (
            "13.1.22",
            "AR-LOG-A: Hierarchical Logging",
            "135.1-2025 - 7.3.2.48.8",
        ),
        (
            "13.1.23",
            "AR-LOG-A: AuditLogQuery Execution",
            "135.1-2025 - 9.40.1.1",
        ),
        (
            "13.1.24",
            "AR-LOG-A: AuditLogQuery Object Filter",
            "135.1-2025 - 9.40.1.2",
        ),
        (
            "13.1.25",
            "AR-LOG-A: AuditLogQuery Time Filter",
            "135.1-2025 - 9.40.1.3",
        ),
    ];

    for &(id, name, reference) in tests {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AuditReporting,
            tags: &["audit", "logging"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(62)),
            timeout: None,
            run: |ctx| Box::pin(audit_log_base(ctx)),
        });
    }
}

async fn audit_log_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let al = ctx.first_object_of_type(ObjectType::AUDIT_LOG)?;
    ctx.verify_readable(al, PropertyIdentifier::LOG_ENABLE)
        .await?;
    ctx.verify_readable(al, PropertyIdentifier::BUFFER_SIZE)
        .await?;
    ctx.verify_readable(al, PropertyIdentifier::RECORD_COUNT)
        .await?;
    ctx.verify_readable(al, PropertyIdentifier::TOTAL_RECORD_COUNT)
        .await?;
    ctx.pass()
}
