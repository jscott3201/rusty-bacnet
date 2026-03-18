//! BTL Test Plan Sections 5.18–5.36 — Domain-Specific Alarm/Event.
//! 108 BTL references: Life Safety (37+8=45), Notification Forwarder (27),
//! Access Control (35+8=43), Elevator (8).
//! (5.22 Configurable Recipient Lists = 0 refs, 5.23 Temp Event Sub = 0 refs)

use bacnet_types::enums::{ObjectType, PropertyIdentifier};

use crate::engine::context::TestContext;
use crate::engine::registry::{Capability, Conditionality, Section, TestDef, TestRegistry};
use crate::report::model::TestFailure;

pub fn register(registry: &mut TestRegistry) {
    // ── 5.18 Life Safety A (13 refs) ─────────────────────────────────────

    let ls_a: &[(&str, &str, &str)] = &[
        (
            "5.18.1",
            "AE-LS-A: Accept LS Notification Confirmed",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.18.2",
            "AE-LS-A: Accept LS Notification Unconfirmed",
            "135.1-2025 - 9.5.1",
        ),
        (
            "5.18.3",
            "AE-LS-A: COR_LIFE_SAFETY Confirmed",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.18.4",
            "AE-LS-A: COR_LIFE_SAFETY Unconfirmed",
            "135.1-2025 - 9.5.1",
        ),
        (
            "5.18.5",
            "AE-LS-A: ACCESS_EVENT Confirmed",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.18.6",
            "AE-LS-A: ACCESS_EVENT Unconfirmed",
            "135.1-2025 - 9.5.1",
        ),
        (
            "5.18.7",
            "AE-LS-A: Timestamp Time Form",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.18.8",
            "AE-LS-A: Timestamp DateTime Form",
            "135.1-2025 - 9.4.2",
        ),
        (
            "5.18.9",
            "AE-LS-A: Timestamp SeqNum Form",
            "135.1-2025 - 9.4.3",
        ),
        (
            "5.18.10",
            "AE-LS-A: TO_OFFNORMAL Transition",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.18.11",
            "AE-LS-A: TO_NORMAL Transition",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.18.12",
            "AE-LS-A: TO_FAULT Transition",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.18.13",
            "AE-LS-A: Ack LS Notification",
            "135.1-2025 - 8.1.1",
        ),
    ];

    for &(id, name, reference) in ls_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "life-safety"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
            timeout: None,
            run: |ctx| Box::pin(ls_base(ctx)),
        });
    }

    // ── 5.19 Life Safety B (24 refs) ─────────────────────────────────────

    let ls_b: &[(&str, &str, &str)] = &[
        ("5.19.1", "AE-LS-B: LSP Event_State", "135.1-2025 - 12.18"),
        (
            "5.19.2",
            "AE-LS-B: LSP COR_LIFE_SAFETY Conf",
            "135.1-2025 - 8.4.19",
        ),
        (
            "5.19.3",
            "AE-LS-B: LSP COR_LIFE_SAFETY Unconf",
            "135.1-2025 - 8.5.19",
        ),
        (
            "5.19.4",
            "AE-LS-B: LSP Event_Enable",
            "135.1-2025 - 7.3.1.10.1",
        ),
        (
            "5.19.5",
            "AE-LS-B: LSP Notification_Class",
            "135.1-2025 - 12.18",
        ),
        ("5.19.6", "AE-LS-B: LSZ Event_State", "135.1-2025 - 12.19"),
        (
            "5.19.7",
            "AE-LS-B: LSZ COR_LIFE_SAFETY Conf",
            "135.1-2025 - 8.4.19",
        ),
        (
            "5.19.8",
            "AE-LS-B: LSZ COR_LIFE_SAFETY Unconf",
            "135.1-2025 - 8.5.19",
        ),
        (
            "5.19.9",
            "AE-LS-B: LSZ Event_Enable",
            "135.1-2025 - 7.3.1.10.1",
        ),
        (
            "5.19.10",
            "AE-LS-B: LSZ Notification_Class",
            "135.1-2025 - 12.19",
        ),
        (
            "5.19.11",
            "AE-LS-B: Event_Detection_Enable LSP",
            "135.1-2025 - 7.3.1.22.1",
        ),
        (
            "5.19.12",
            "AE-LS-B: Event_Detection_Enable LSZ",
            "135.1-2025 - 7.3.1.22.1",
        ),
        (
            "5.19.13",
            "AE-LS-B: Event_Algorithm_Inhibit LSP",
            "135.1-2025 - 7.3.1.19.1",
        ),
        (
            "5.19.14",
            "AE-LS-B: Event_Algorithm_Inhibit LSZ",
            "135.1-2025 - 7.3.1.19.1",
        ),
        (
            "5.19.15",
            "AE-LS-B: Fault Re-Notify LSP",
            "135.1-2025 - 8.4.17.10",
        ),
        (
            "5.19.16",
            "AE-LS-B: Fault Re-Notify LSZ",
            "135.1-2025 - 8.5.17.10",
        ),
        (
            "5.19.17",
            "AE-LS-B: Acked_Transitions LSP",
            "135.1-2025 - 7.3.1.11.1",
        ),
        (
            "5.19.18",
            "AE-LS-B: Acked_Transitions LSZ",
            "135.1-2025 - 7.3.1.11.1",
        ),
        (
            "5.19.19",
            "AE-LS-B: Event_Time_Stamps LSP",
            "135.1-2025 - 12.18",
        ),
        (
            "5.19.20",
            "AE-LS-B: Event_Time_Stamps LSZ",
            "135.1-2025 - 12.19",
        ),
        ("5.19.21", "AE-LS-B: REI LSP", "BTL - 7.3.1.21.1"),
        ("5.19.22", "AE-LS-B: REI LSZ", "BTL - 7.3.1.21.1"),
        ("5.19.23", "AE-LS-B: LSP Alarm_Values", "135.1-2025 - 12.18"),
        ("5.19.24", "AE-LS-B: LSZ Zone_Members", "135.1-2025 - 12.19"),
    ];

    for &(id, name, reference) in ls_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "life-safety"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
            timeout: None,
            run: |ctx| Box::pin(ls_b_test(ctx)),
        });
    }

    // ── 5.20 Notification Forwarder B (14 refs) ──────────────────────────

    let nf: &[(&str, &str, &str)] = &[
        (
            "5.20.1",
            "NF-B: Recipient_List Forwarding",
            "135.1-2025 - 7.3.2.30.2",
        ),
        (
            "5.20.2",
            "NF-B: Subscribed_Recipients Forwarding",
            "135.1-2025 - 7.3.2.30.3",
        ),
        (
            "5.20.3",
            "NF-B: Date Filtering",
            "135.1-2025 - 7.3.2.30.7.1",
        ),
        (
            "5.20.4",
            "NF-B: Time Filtering",
            "135.1-2025 - 7.3.2.30.7.2",
        ),
        (
            "5.20.5",
            "NF-B: Process Identifier",
            "135.1-2025 - 7.3.2.30.7.3",
        ),
        (
            "5.20.6",
            "NF-B: Transition Filtering",
            "135.1-2025 - 7.3.2.30.7.4",
        ),
        (
            "5.20.7",
            "NF-B: Local+Remote When False",
            "135.1-2025 - 7.3.2.30.11.2",
        ),
        (
            "5.20.8",
            "NF-B: Character Encoding",
            "135.1-2025 - 7.3.2.30.5",
        ),
        (
            "5.20.9",
            "NF-B: Local Broadcast Restriction",
            "135.1-2025 - 7.3.2.30.12.1",
        ),
        (
            "5.20.10",
            "NF-B: Global Broadcast Restriction",
            "135.1-2025 - 7.3.2.30.12.2",
        ),
        (
            "5.20.11",
            "NF-B: Forward As Global Restriction",
            "135.1-2025 - 7.3.2.30.12.3",
        ),
        (
            "5.20.12",
            "NF-B: Directed Bcast BACnetAddr",
            "135.1-2025 - 7.3.2.30.12.4",
        ),
        (
            "5.20.13",
            "NF-B: Directed Bcast OID",
            "135.1-2025 - 7.3.2.30.12.5",
        ),
        (
            "5.20.14",
            "NF-B: Port Restriction",
            "135.1-2025 - 7.3.2.30.12.6",
        ),
    ];

    for &(id, name, reference) in nf {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-forwarder"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
            timeout: None,
            run: |ctx| Box::pin(nf_base(ctx)),
        });
    }

    // ── 5.21 Notification Forwarder Internal B (13 refs) ─────────────────

    let nf_int: &[(&str, &str, &str)] = &[
        (
            "5.21.1",
            "NF-Int-B: Forward Local Events",
            "135.1-2025 - 7.3.2.30.1",
        ),
        (
            "5.21.2",
            "NF-Int-B: Forward to Recipient_List",
            "135.1-2025 - 7.3.2.30.2",
        ),
        (
            "5.21.3",
            "NF-Int-B: Forward to Subscribed",
            "135.1-2025 - 7.3.2.30.3",
        ),
        (
            "5.21.4",
            "NF-Int-B: Date Filter Internal",
            "135.1-2025 - 7.3.2.30.7.1",
        ),
        (
            "5.21.5",
            "NF-Int-B: Time Filter Internal",
            "135.1-2025 - 7.3.2.30.7.2",
        ),
        (
            "5.21.6",
            "NF-Int-B: Process ID Internal",
            "135.1-2025 - 7.3.2.30.7.3",
        ),
        (
            "5.21.7",
            "NF-Int-B: Transition Filter Internal",
            "135.1-2025 - 7.3.2.30.7.4",
        ),
        (
            "5.21.8",
            "NF-Int-B: Local Only When True",
            "135.1-2025 - 7.3.2.30.11.1",
        ),
        (
            "5.21.9",
            "NF-Int-B: Character Encoding",
            "135.1-2025 - 7.3.2.30.5",
        ),
        (
            "5.21.10",
            "NF-Int-B: Port Restriction Int",
            "135.1-2025 - 7.3.2.30.12.6",
        ),
        (
            "5.21.11",
            "NF-Int-B: Local Bcast Restriction",
            "135.1-2025 - 7.3.2.30.12.1",
        ),
        (
            "5.21.12",
            "NF-Int-B: Global Bcast Restriction",
            "135.1-2025 - 7.3.2.30.12.2",
        ),
        (
            "5.21.13",
            "NF-Int-B: Forward As Global",
            "135.1-2025 - 7.3.2.30.12.3",
        ),
    ];

    for &(id, name, reference) in nf_int {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "notification-forwarder", "internal"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(51)),
            timeout: None,
            run: |ctx| Box::pin(nf_base(ctx)),
        });
    }

    // ── 5.24-5.27 Life Safety View/Modify (8 refs) ──────────────────────

    for &(id, name) in &[
        ("5.24.1", "LS-ViewNotif-A: Browse LSP Events"),
        ("5.24.2", "LS-ViewNotif-A: Browse LSZ Events"),
        ("5.25.1", "LS-AdvViewNotif-A: Advanced Browse LSP"),
        ("5.25.2", "LS-AdvViewNotif-A: Advanced Browse LSZ"),
        ("5.26.1", "LS-ViewModify-A: Write LSP Event_Enable"),
        ("5.26.2", "LS-ViewModify-A: Write LSZ Event_Enable"),
        ("5.27.1", "LS-AdvViewModify-A: Write LSP Limits"),
        ("5.27.2", "LS-AdvViewModify-A: Write LSZ Limits"),
    ] {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.1",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "life-safety", "view"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(21)),
            timeout: None,
            run: |ctx| Box::pin(ls_base(ctx)),
        });
    }

    // ── 5.28 Access Control A (12 refs) ──────────────────────────────────

    let ac_a: &[(&str, &str, &str)] = &[
        (
            "5.28.1",
            "AE-AC-A: Accept ACCESS_EVENT Confirmed",
            "135.1-2025 - 9.4.1",
        ),
        (
            "5.28.2",
            "AE-AC-A: Accept ACCESS_EVENT Unconfirmed",
            "135.1-2025 - 9.5.1",
        ),
        (
            "5.28.3",
            "AE-AC-A: COR_LIFE_SAFETY from AP",
            "135.1-2025 - 9.4.1",
        ),
        ("5.28.4", "AE-AC-A: COR from CDI", "135.1-2025 - 9.4.1"),
        ("5.28.5", "AE-AC-A: Timestamp Forms", "135.1-2025 - 9.4.1"),
        ("5.28.6", "AE-AC-A: All Transitions", "135.1-2025 - 9.4.1"),
        (
            "5.28.7",
            "AE-AC-A: Ack AC Notification",
            "135.1-2025 - 8.1.1",
        ),
        (
            "5.28.8",
            "AE-AC-A: AP Event Properties",
            "135.1-2025 - 12.41",
        ),
        (
            "5.28.9",
            "AE-AC-A: CDI Event Properties",
            "135.1-2025 - 12.43",
        ),
        (
            "5.28.10",
            "AE-AC-A: Door Event Properties",
            "135.1-2025 - 12.30",
        ),
        (
            "5.28.11",
            "AE-AC-A: Zone Event Properties",
            "135.1-2025 - 12.42",
        ),
        (
            "5.28.12",
            "AE-AC-A: Credential Event Properties",
            "135.1-2025 - 12.40",
        ),
    ];

    for &(id, name, reference) in ac_a {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "access-control"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
            timeout: None,
            run: |ctx| Box::pin(ac_base(ctx)),
        });
    }

    // ── 5.29 Access Control B (21 refs) ──────────────────────────────────

    let ac_b: &[(&str, &str, &str)] = &[
        (
            "5.29.1",
            "AE-AC-B: AP ACCESS_EVENT Conf",
            "135.1-2025 - 8.4.15",
        ),
        (
            "5.29.2",
            "AE-AC-B: AP ACCESS_EVENT Unconf",
            "135.1-2025 - 8.5.15",
        ),
        ("5.29.3", "AE-AC-B: CDI COR Conf", "135.1-2025 - 8.4.17.1"),
        ("5.29.4", "AE-AC-B: CDI COR Unconf", "135.1-2025 - 8.5.17.1"),
        (
            "5.29.5",
            "AE-AC-B: AP Event_Enable",
            "135.1-2025 - 7.3.1.10.1",
        ),
        (
            "5.29.6",
            "AE-AC-B: CDI Event_Enable",
            "135.1-2025 - 7.3.1.10.1",
        ),
        (
            "5.29.7",
            "AE-AC-B: AP Notification_Class",
            "135.1-2025 - 12.41",
        ),
        (
            "5.29.8",
            "AE-AC-B: CDI Notification_Class",
            "135.1-2025 - 12.43",
        ),
        ("5.29.9", "AE-AC-B: Door Event_State", "135.1-2025 - 12.30"),
        (
            "5.29.10",
            "AE-AC-B: AP Event_Detection_Enable",
            "135.1-2025 - 7.3.1.22.1",
        ),
        (
            "5.29.11",
            "AE-AC-B: CDI Event_Detection_Enable",
            "135.1-2025 - 7.3.1.22.1",
        ),
        (
            "5.29.12",
            "AE-AC-B: AP Event_Algorithm_Inhibit",
            "135.1-2025 - 7.3.1.19.1",
        ),
        (
            "5.29.13",
            "AE-AC-B: Door Fault Re-Notify Conf",
            "135.1-2025 - 8.4.17.10",
        ),
        (
            "5.29.14",
            "AE-AC-B: Door Fault Re-Notify Unconf",
            "135.1-2025 - 8.5.17.10",
        ),
        (
            "5.29.15",
            "AE-AC-B: AP Acked_Transitions",
            "135.1-2025 - 7.3.1.11.1",
        ),
        (
            "5.29.16",
            "AE-AC-B: CDI Acked_Transitions",
            "135.1-2025 - 7.3.1.11.1",
        ),
        (
            "5.29.17",
            "AE-AC-B: AP Event_Time_Stamps",
            "135.1-2025 - 12.41",
        ),
        (
            "5.29.18",
            "AE-AC-B: CDI Event_Time_Stamps",
            "135.1-2025 - 12.43",
        ),
        ("5.29.19", "AE-AC-B: AP REI", "BTL - 7.3.1.21.1"),
        ("5.29.20", "AE-AC-B: CDI REI", "BTL - 7.3.1.21.1"),
        ("5.29.21", "AE-AC-B: Door REI", "BTL - 7.3.1.21.1"),
    ];

    for &(id, name, reference) in ac_b {
        registry.add(TestDef {
            id,
            name,
            reference,
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "access-control"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
            timeout: None,
            run: |ctx| Box::pin(ac_b_test(ctx)),
        });
    }

    // ── 5.30-5.32 AC View/Modify (6 refs) ───────────────────────────────

    for &(id, name) in &[
        ("5.30.1", "AC-AdvViewNotif-A: Browse AP Events"),
        ("5.30.2", "AC-AdvViewNotif-A: Browse CDI Events"),
        ("5.31.1", "AC-ViewModify-A: Write AP Event_Enable"),
        ("5.31.2", "AC-ViewModify-A: Write CDI Event_Enable"),
        ("5.32.1", "AC-AdvViewModify-A: Write AP Limits"),
        ("5.32.2", "AC-AdvViewModify-A: Write CDI Limits"),
    ] {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.1",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "access-control", "view"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(33)),
            timeout: None,
            run: |ctx| Box::pin(ac_base(ctx)),
        });
    }

    // ── 5.33-5.36 Elevator (8 refs) ──────────────────────────────────────

    for &(id, name) in &[
        ("5.33.1", "EV-ViewNotif-A: Browse EG Events"),
        ("5.33.2", "EV-ViewNotif-A: Browse Lift Events"),
        ("5.34.1", "EV-AdvViewNotif-A: Advanced EG"),
        ("5.34.2", "EV-AdvViewNotif-A: Advanced Lift"),
        ("5.35.1", "EV-ViewModify-A: Write EG Event_Enable"),
        ("5.35.2", "EV-ViewModify-A: Write Lift Event_Enable"),
        ("5.36.1", "EV-AdvViewModify-A: Write EG Limits"),
        ("5.36.2", "EV-AdvViewModify-A: Write Lift Limits"),
    ] {
        registry.add(TestDef {
            id,
            name,
            reference: "135.1-2025 - 8.18.1",
            section: Section::AlarmAndEvent,
            tags: &["alarm-event", "elevator"],
            conditionality: Conditionality::RequiresCapability(Capability::ObjectType(57)),
            timeout: None,
            run: |ctx| Box::pin(ev_base(ctx)),
        });
    }
}

// ═══════════════════════════════════════════════════════════════════════════

async fn ls_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let lsp = ctx.first_object_of_type(ObjectType::LIFE_SAFETY_POINT)?;
    ctx.verify_readable(lsp, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.verify_readable(lsp, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.pass()
}

async fn ls_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let lsp = ctx.first_object_of_type(ObjectType::LIFE_SAFETY_POINT)?;
    ctx.verify_readable(lsp, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.verify_readable(lsp, PropertyIdentifier::PRESENT_VALUE)
        .await?;
    ctx.verify_readable(lsp, PropertyIdentifier::MODE).await?;
    ctx.pass()
}

async fn nf_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let nf = ctx.first_object_of_type(ObjectType::NOTIFICATION_FORWARDER)?;
    ctx.verify_readable(nf, PropertyIdentifier::RECIPIENT_LIST)
        .await?;
    ctx.verify_readable(nf, PropertyIdentifier::PROCESS_IDENTIFIER_FILTER)
        .await?;
    ctx.pass()
}

async fn ac_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ap = ctx.first_object_of_type(ObjectType::ACCESS_POINT)?;
    ctx.verify_readable(ap, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.pass()
}

async fn ac_b_test(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let ap = ctx.first_object_of_type(ObjectType::ACCESS_POINT)?;
    ctx.verify_readable(ap, PropertyIdentifier::EVENT_STATE)
        .await?;
    ctx.verify_readable(ap, PropertyIdentifier::OBJECT_NAME)
        .await?;
    ctx.pass()
}

async fn ev_base(ctx: &mut TestContext) -> Result<(), TestFailure> {
    let eg = ctx.first_object_of_type(ObjectType::ELEVATOR_GROUP)?;
    ctx.verify_readable(eg, PropertyIdentifier::GROUP_ID)
        .await?;
    ctx.pass()
}
