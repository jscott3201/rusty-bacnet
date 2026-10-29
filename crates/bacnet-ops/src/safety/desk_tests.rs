//! The desk without a network: what it refuses and why, the workspace's
//! lock, keeping entries, settling an interrupted write, and a ledger a
//! later release wrote.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;

use super::*;
use crate::safety::ledger::State;
use crate::safety::{Effect, FrontEnd, Occupancy, Stored, Tier};

/// A caller at `level`, refusing with `refusal`.
struct Fixed {
    level: Level,
    refusal: Option<String>,
}

impl Caller for Fixed {
    fn level(&self) -> Level {
        self.level
    }
    fn refusal(&self) -> Option<String> {
        self.refusal.clone()
    }
}

const ARMED: Fixed = Fixed {
    level: Level::Armed { dangerous: false },
    refusal: None,
};

fn context() -> Context {
    Context {
        profile: Some("lab".into()),
        transport: "BIP",
        local: "10.0.0.1:47808".into(),
    }
}

fn planned(priority: u8, gate: Gate) -> Planned {
    Planned {
        spec: WriteSpec {
            device: 7,
            object: ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap(),
            property: PropertyIdentifier::PRESENT_VALUE,
            index: None,
            value: Some("1".into()),
            priority: Some(priority),
        },
        priority: Some(priority),
        value: Stored {
            bytes: vec![0x44, 0x3f, 0x80, 0, 0],
            text: "1".into(),
        },
        commandable: true,
        before: None,
        occupancy: Occupancy::Empty,
        effect: Effect::NotPresentValue,
        out_of_service: None,
        notes: Vec::new(),
        gate,
    }
}

fn preview(item: Planned) -> Preview {
    Preview {
        device: 7,
        gate: item.gate.clone(),
        items: vec![item],
        batch: false,
    }
}

/// No workspace: no ledger, and every write refused; the caller's own
/// refusal comes first, then the missing ledger, then the preview's.
#[test]
fn refusals_come_in_order() {
    let desk = Desk::open(None, context(), Origin::default());
    assert_eq!(desk.missing(), Some(NO_WORKSPACE));
    let fine = preview(planned(12, Gate::Allowed(Tier::T1)));
    let passive = Fixed {
        level: Level::Passive,
        refusal: Some("passive".into()),
    };
    assert_eq!(
        desk.write_refusal(&passive, &fine, false).as_deref(),
        Some("passive")
    );
    assert_eq!(
        desk.write_refusal(&ARMED, &fine, false).as_deref(),
        Some(NO_WORKSPACE)
    );

    let dir = tempfile::tempdir().unwrap();
    let desk = Desk::open(Some(dir.path()), context(), Origin::default());
    assert_eq!(desk.missing(), None);
    assert_eq!(desk.write_refusal(&ARMED, &fine, false), None);
    let six = preview(planned(6, Gate::Allowed(Tier::T3)));
    assert!(
        desk.write_refusal(&ARMED, &six, false).is_some(),
        "priority 6"
    );
    assert_eq!(desk.write_refusal(&ARMED, &six, true), None, "dangerous");
}

/// Another holder of the workspace leaves a desk without a ledger that
/// still records, and can't flag entries kept.
#[tokio::test]
async fn a_held_workspace_records_but_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let held = Desk::open(Some(dir.path()), context(), Origin::default());
    let mut second = Desk::open(Some(dir.path()), context(), Origin::mcp("agent"));
    assert!(second.missing().unwrap().contains("in use"));
    assert!(!second.keep_all());
    second.record(&ARMED, "write", serde_json::json!({})).await;
    let text = std::fs::read_to_string(dir.path().join(crate::safety::audit::FILE)).unwrap();
    assert!(text.contains("\"front_end\":\"mcp\""), "{text}");
    drop(held);
}

/// A write whose task failed after recording it as `sending` is settled
/// as unknown by its own run; an earlier run's sweep leaves it alone.
#[tokio::test]
async fn an_interrupted_write_settles_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut ledger = Ledger::open(dir.path()).unwrap();
        ledger.begin(&planned(12, Gate::Allowed(Tier::T1)), run_id(), "t");
        ledger.save().unwrap();
    }
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    desk.settle_abandoned(&ARMED).await;
    assert_eq!(desk.entries()[0].state, State::Sending, "this run's own");
    desk.settle_interrupted(&ARMED).await;
    assert_eq!(desk.entries()[0].state, State::Unknown);
    drop(desk);
    assert_eq!(
        Ledger::open(dir.path()).unwrap().entries()[0].state,
        State::Unknown,
        "saved"
    );
    let text = std::fs::read_to_string(dir.path().join(crate::safety::audit::FILE)).unwrap();
    assert!(text.contains("the write task failed"), "{text}");
    // Keep flags the entries and saves them.
    let mut desk = Desk::open(Some(dir.path()), context(), Origin::default());
    assert!(desk.keep_all());
    assert!(Ledger::open(dir.path()).unwrap().entries()[0].kept);
}

/// An entry a later release wrote, with a front end this one doesn't
/// know, doesn't stop the ledger loading.
#[test]
fn an_unknown_front_end_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut ledger = Ledger::open(dir.path()).unwrap();
        ledger.begin(&planned(12, Gate::Allowed(Tier::T1)), "r", "t");
        ledger.save().unwrap();
    }
    let path = dir.path().join(crate::safety::ledger::FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    let text = text.replace("\"seq\"", "\"front_end\": \"daemon\",\n      \"seq\"");
    std::fs::write(&path, text).unwrap();
    let ledger = Ledger::open(dir.path()).unwrap();
    assert_eq!(ledger.entries()[0].origin.front_end, FrontEnd::Unknown);
}
