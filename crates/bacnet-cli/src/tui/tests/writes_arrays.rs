//! Array writes on the lab site, after review of #1801: an element changed
//! since its preview is not sent, two elements of one array in a batch are
//! both recorded and reverted, a resize (index 0) is never reverted, and an
//! array written by element and whole goes back to its original. All on
//! paused time.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Server, Site};
use super::writes::*;
use super::*;
use crate::tui::app::safety::Modal;
use crate::tui::profile::Safety;
use crate::tui::safety::ledger::Ledger;
use crate::tui::safety::plan::RESIZE;
use crate::tui::safety::{Effect, Gate, Occupancy, Outcome, Planned, Stored, Tier, WriteSpec};
use crate::tui::worker::write::{STALE, UNKNOWN_PRE_IMAGE};

fn msv1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap()
}

async fn state_texts(site: &Site) -> Vec<PropertyValue> {
    let db = site.server(0).database().read().await;
    let object = db.get(&msv1()).unwrap();
    (1..=3)
        .map(|i| {
            object
                .read_property(PropertyIdentifier::STATE_TEXT, Some(i))
                .unwrap()
        })
        .collect()
}

fn texts(values: &[&str]) -> Vec<PropertyValue> {
    values
        .iter()
        .map(|v| PropertyValue::CharacterString((*v).into()))
        .collect()
}

const ORIGINAL: [&str; 3] = ["State 1", "State 2", "State 3"];

/// Set the server's State_Text elements, as another writer would.
async fn set_texts(site: &Site, values: &[&str]) {
    let mut db = site.server(0).database().write().await;
    let object = db.get_mut(&msv1()).unwrap();
    for (i, value) in texts(values).into_iter().enumerate() {
        object
            .write_property(
                PropertyIdentifier::STATE_TEXT,
                Some(i as u32 + 1),
                value,
                None,
            )
            .unwrap();
    }
}

/// `:` and a `writem` line, then wait for the preview.
async fn writem(site: &mut Site, line: &str) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, line);
    site.press(KeyCode::Enter).await;
    previewed(site).await;
}

/// `u`, `R`, `y`, and wait for the revert to end.
async fn revert_all(site: &mut Site) {
    site.press(KeyCode::Char('u')).await;
    site.press(KeyCode::Char('R')).await;
    site.press(KeyCode::Char('y')).await;
    site.pump_until(|app| app.safety.pending.is_none()).await;
}

#[tokio::test(start_paused = true)]
async fn an_element_changed_since_its_preview_is_not_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    writem(&mut site, "writem 100 msv:1 state-text[2] \"Low\"").await;
    set_texts(&site, &["State 1", "Someone else", "State 3"]).await;
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::NotSent(STALE.into())]);
    assert_eq!(writes_sent(&site), 0);
    assert_eq!(
        state_texts(&site).await,
        texts(&["State 1", "Someone else", "State 3"])
    );
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn two_elements_of_one_array_in_a_batch_are_recorded_and_reverted() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    writem(
        &mut site,
        "writem 100 msv:1 state-text[1] \"One\" msv:1 state-text[3] \"Three\"",
    )
    .await;
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(
        state_texts(&site).await,
        texts(&["One", "State 2", "Three"])
    );
    let ledger = Ledger::open(dir.path()).unwrap();
    let indexes: Vec<_> = ledger.entries().iter().map(|e| e.index).collect();
    assert_eq!(indexes, [Some(1), Some(3)]);
    site.press(KeyCode::Esc).await;
    revert_all(&mut site).await;
    assert!(site.app.safety.ledger.is_empty());
    assert_eq!(state_texts(&site).await, texts(&ORIGINAL));
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_resize_is_recorded_without_a_pre_image_and_never_reverted() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    writem(&mut site, "writem 100 msv:1 state-text[0] 4").await;
    let planned = &preview(&site.app).items[0];
    assert_eq!(planned.before, None);
    assert!(
        planned.notes.iter().any(|n| n == RESIZE),
        "{:?}",
        planned.notes
    );
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(outcomes(&site.app), [Outcome::Ack]);
    site.press(KeyCode::Esc).await;
    let sent = writes_sent(&site);
    revert_all(&mut site).await;
    assert_eq!(writes_sent(&site), sent, "nothing to put back");
    assert_eq!(site.app.safety.ledger.len(), 1, "the entry stays");
    let records = audit(dir.path());
    let revert = records.iter().find(|r| r["event"] == "revert").unwrap();
    assert_eq!(revert["outcome"]["why"], UNKNOWN_PRE_IMAGE);
    site.stop().await;
}

fn encoded(values: &[&str]) -> Stored {
    let mut buf = BytesMut::new();
    for value in texts(values) {
        bacnet_encoding::primitives::encode_property_value(&mut buf, &value).unwrap();
    }
    Stored {
        bytes: buf.to_vec(),
        text: values.join(", "),
    }
}

/// A write to MSV:1's State_Text, `[index]` or whole, as the plan makes it.
fn state_text(index: Option<u32>, value: &[&str], before: &[&str]) -> Planned {
    Planned {
        spec: WriteSpec {
            device: LAB,
            object: msv1(),
            property: PropertyIdentifier::STATE_TEXT,
            index,
            value: None,
            priority: None,
        },
        priority: None,
        value: encoded(value),
        commandable: false,
        before: Some(encoded(before)),
        occupancy: Occupancy::NotCommandable,
        effect: Effect::NotPresentValue,
        out_of_service: None,
        notes: Vec::new(),
        gate: Gate::Allowed(Tier::T1),
    }
}

/// The reviewer's sequence: `[2]` = A, the whole array, `[2]` = B. The
/// command line can't type a whole array, so an earlier run's ledger holds
/// it, built by the ledger itself, and the device holds what it wrote.
/// Revert-all puts back the original array and empties the ledger.
#[tokio::test(start_paused = true)]
async fn an_array_written_by_element_and_whole_reverts_to_its_original() {
    let dir = tempfile::tempdir().unwrap();
    let mut ledger = Ledger::open(dir.path()).unwrap();
    for planned in [
        state_text(Some(2), &["A"], &["State 2"]),
        state_text(None, &["X", "Y", "Z"], &["State 1", "A", "State 3"]),
        state_text(Some(2), &["B"], &["Y"]),
    ] {
        let id = ledger.begin(&planned, "earlier-run", "2026-10-20T12:00:00.000Z");
        ledger.finish(id, Some(true), &[]);
    }
    ledger.save().unwrap();
    assert_eq!(ledger.entries().len(), 1);
    let options = options(dir.path(), Safety::default(), false);
    let mut site = Site::start_with_options(
        vec![Server::lab(100, LAB)],
        ClientLimits::default(),
        options,
    )
    .await;
    set_texts(&site, &["X", "B", "Z"]).await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    assert!(matches!(modal(&site.app), Some(Modal::Ledger { .. })));
    site.press(KeyCode::Char('R')).await;
    site.press(KeyCode::Char('y')).await;
    site.pump_until(|app| app.safety.ledger.is_empty() && app.safety.pending.is_none())
        .await;
    assert_eq!(state_texts(&site).await, texts(&ORIGINAL));
    assert!(Ledger::open(dir.path()).unwrap().entries().is_empty());
    site.stop().await;
}
