//! Writes to one array element on the lab site (#1801): the element's old
//! value is read before the write, kept in the ledger, and put back by
//! revert-all; an entry whose old value is unknown is left, and revert says
//! why. All on paused time.

use std::path::Path;

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Server, Site};
use super::writes::*;
use super::*;
use crate::tui::app::safety::Modal;
use crate::tui::profile::Safety;
use crate::tui::safety::ledger::{Entry, Ledger, State};
use crate::tui::safety::Stored;
use crate::tui::worker::write::UNKNOWN_PRE_IMAGE;

fn msv1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_VALUE, 1).unwrap()
}

async fn state_text(site: &Site, index: u32) -> PropertyValue {
    let db = site.server(0).database().read().await;
    let object = db.get(&msv1()).unwrap();
    object
        .read_property(PropertyIdentifier::STATE_TEXT, Some(index))
        .unwrap()
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::CharacterString(s.into())
}

/// `R`, `y` on the ledger view, and wait for the revert to end.
async fn revert_all(site: &mut Site) {
    site.press(KeyCode::Char('u')).await;
    site.press(KeyCode::Char('R')).await;
    site.press(KeyCode::Char('y')).await;
    site.pump_until(|app| app.safety.pending.is_none()).await;
}

#[tokio::test(start_paused = true)]
async fn an_indexed_write_records_the_element_and_revert_puts_it_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, "writem 100 msv:1 state-text[2] \"Low\"");
    site.press(KeyCode::Enter).await;
    previewed(&mut site).await;
    let planned = &preview(&site.app).items[0];
    assert_eq!(planned.spec.index, Some(2));
    assert_eq!(
        planned.before.as_ref().map(|b| b.text.as_str()),
        Some("\"State 2\""),
        "the element's pre-image: {:?}",
        planned.notes
    );
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    assert_eq!(state_text(&site, 2).await, text("Low"));
    let ledger = Ledger::open(dir.path()).unwrap();
    let [entry] = ledger.entries() else {
        panic!("{:?}", ledger.entries());
    };
    assert_eq!(entry.index, Some(2));
    assert_eq!(entry.undo().unwrap().text, "\"State 2\"");
    site.press(KeyCode::Esc).await;
    revert_all(&mut site).await;
    assert!(site.app.safety.ledger.is_empty());
    assert_eq!(state_text(&site, 2).await, text("State 2"));
    assert_eq!(
        state_text(&site, 1).await,
        text("State 1"),
        "only the element"
    );
    site.stop().await;
}

/// An earlier run's indexed write whose element couldn't be read.
fn unknown() -> Entry {
    Entry {
        id: 1,
        run: "earlier-run".into(),
        at: "2026-10-18T12:00:00.000Z".into(),
        device: LAB,
        object_type: ObjectType::MULTI_STATE_VALUE.to_raw(),
        instance: 1,
        property: PropertyIdentifier::STATE_TEXT.to_raw(),
        index: Some(2),
        priority: None,
        commandable: false,
        before: None,
        written: Stored {
            bytes: vec![0x73, 0x00, b'L', b'o', b'w'],
            text: "\"Low\"".into(),
        },
        state: State::Applied,
        kept: false,
        seq: 1,
    }
}

fn leave_ledger(dir: &Path, entries: &[Entry]) {
    let file = serde_json::json!({ "version": 1, "entries": entries });
    std::fs::write(dir.join("ledger.json"), file.to_string()).unwrap();
}

#[tokio::test(start_paused = true)]
async fn revert_leaves_an_entry_with_an_unknown_pre_image_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    leave_ledger(dir.path(), &[unknown()]);
    let options = options(dir.path(), Safety::default(), false);
    let mut site = Site::start_with_options(
        vec![Server::lab(100, LAB)],
        ClientLimits::default(),
        options,
    )
    .await;
    site.pump_until(|app| app.safety.policy.is_some()).await;
    assert!(matches!(modal(&site.app), Some(Modal::Ledger { .. })));
    site.press(KeyCode::Char('R')).await;
    site.press(KeyCode::Char('y')).await;
    site.pump_until(|app| app.safety.pending.is_none()).await;
    assert_eq!(writes_sent(&site), 0);
    assert_eq!(site.app.safety.ledger.len(), 1, "the entry stays");
    let records = audit(dir.path());
    let revert = records.iter().find(|r| r["event"] == "revert").unwrap();
    assert_eq!(revert["outcome"]["result"], "not-sent");
    assert_eq!(revert["outcome"]["why"], UNKNOWN_PRE_IMAGE);
    site.stop().await;
}
