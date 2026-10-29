//! Writes typed by name on the lab site (#2075): `:writem` takes an
//! enumeration's names and a bit string's named bits for the property it
//! writes, previews them as the browser names them, and refuses a
//! mistyped name with the choices. All on paused time.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use ratatui::crossterm::event::KeyCode;

use super::site::{Server, Site};
use super::writes::*;
use super::*;
use crate::tui::profile::Safety;

async fn value(
    site: &Site,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let db = site.server(0).database().read().await;
    db.get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// Type `line` on the command bar and run it.
async fn command(site: &mut Site, line: &str) {
    site.press(KeyCode::Char(':')).await;
    type_text(&mut site.app, line);
    site.press(KeyCode::Enter).await;
}

#[tokio::test(start_paused = true)]
async fn writem_takes_names_and_named_bits() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    command(
        &mut site,
        "writem 100 bv:1 pv ACTIVE@8 ai:1 event-enable to-fault|to-normal",
    )
    .await;
    previewed(&mut site).await;
    let items = &preview(&site.app).items;
    assert_eq!(items[0].value.text, "active");
    assert_eq!(items[1].value.text, "to-fault | to-normal");
    site.press(KeyCode::Char('y')).await;
    finished(&mut site).await;
    let bv1 = ObjectIdentifier::new(ObjectType::BINARY_VALUE, 1).unwrap();
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    assert_eq!(
        value(&site, bv1, PropertyIdentifier::PRESENT_VALUE).await,
        PropertyValue::Enumerated(1)
    );
    assert_eq!(
        value(&site, ai1, PropertyIdentifier::EVENT_ENABLE).await,
        PropertyValue::BitString {
            unused_bits: 5,
            data: vec![0b0110_0000],
        }
    );
    site.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_mistyped_name_is_refused_before_anything_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut site = lab(dir.path(), Safety::default(), Server::lab(100, LAB)).await;
    arm(&mut site).await;
    command(&mut site, "writem 100 bv:1 pv actve").await;
    let flash = site
        .app
        .flash
        .as_ref()
        .map(|f| f.text.clone())
        .unwrap_or_default();
    assert!(
        flash.contains("unknown BinaryPV 'actve'; expected one of: inactive, active"),
        "{flash}"
    );
    assert_eq!(writes_sent(&site), 0);
    site.stop().await;
}
