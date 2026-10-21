//! The device browser against the lab device on the in-memory site: the
//! object list, an object's properties and priority array, health badges,
//! the ReadProperty fallback, the element-by-element Object_List and the
//! one-request-per-device limit.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::site::{ClientLimits, Quirks, Server, Site};
use super::*;
use crate::tui::app::browser::{ListState, Pane, ViewState};
use crate::tui::point::{BrowseRead, BrowseResult, ObjectView, ReadVia};

/// The lab device's instance and address.
const LAB: u32 = 100;
const LAB_MAC: [u8; 6] = [10, 0, 0, 100, 0xBA, 0xC0];

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// The lab's objects, in the order the browser groups them.
fn lab_objects() -> Vec<ObjectIdentifier> {
    vec![
        oid(ObjectType::ANALOG_INPUT, 1),
        oid(ObjectType::ANALOG_INPUT, 2),
        oid(ObjectType::ANALOG_OUTPUT, 1),
        oid(ObjectType::ANALOG_VALUE, 1),
        oid(ObjectType::BINARY_INPUT, 1),
        oid(ObjectType::BINARY_OUTPUT, 1),
        oid(ObjectType::BINARY_VALUE, 1),
        oid(ObjectType::DEVICE, LAB),
        oid(ObjectType::NOTIFICATION_CLASS, 1),
        oid(ObjectType::TREND_LOG, 1),
        oid(ObjectType::MULTI_STATE_VALUE, 1),
    ]
}

/// Discover the one device and open it, waiting for its object list.
pub(super) async fn open_device(site: &mut Site) {
    site.who_is("", 1).await;
    site.press(KeyCode::Enter).await;
    site.pump_until(|app| {
        app.browser
            .as_ref()
            .is_some_and(|b| !matches!(b.list, ListState::Card | ListState::Reading { .. }))
    })
    .await;
}

/// Select the object whose label matches `filter`, read it, and return
/// what was read.
pub(super) async fn open_object(site: &mut Site, filter: &str) -> ObjectView {
    site.press(KeyCode::Char('/')).await;
    type_text(&mut site.app, filter);
    site.press(KeyCode::Enter).await;
    // Esc clears the filter and keeps the selection.
    site.press(KeyCode::Esc).await;
    site.press(KeyCode::Enter).await;
    site.pump_until(|app| {
        app.browser
            .as_ref()
            .is_some_and(|b| !matches!(b.view, ViewState::Reading(_)))
    })
    .await;
    match &site.app.browser.as_ref().unwrap().view {
        ViewState::Ready(view) => (**view).clone(),
        other => panic!("reading {filter} ended in {other:?}"),
    }
}

fn listed(site: &Site) -> Vec<ObjectIdentifier> {
    let browser = site.app.browser.as_ref().unwrap();
    browser
        .objects
        .rows()
        .iter()
        .filter_map(|row| match row {
            crate::tui::app::objects::ListRow::Object(object) => Some(*object),
            crate::tui::app::objects::ListRow::Group { .. } => None,
        })
        .collect()
}

/// Every lab object is listed, AO:1 opens with its properties, and its
/// priority pane marks slot 10 and shows Relinquish_Default.
#[tokio::test(start_paused = true)]
async fn the_lab_device_lists_every_object_and_opens_ao1() {
    let mut site = Site::start_with(vec![Server::lab(100, LAB)], ClientLimits::default()).await;
    open_device(&mut site).await;
    let browser = site.app.browser.as_ref().unwrap();
    assert_eq!(browser.list, ListState::Ready { indexed: false });
    let mut expected = lab_objects();
    expected.sort_by_key(|o| (o.object_type().to_raw(), o.instance_number()));
    assert_eq!(listed(&site), expected);
    let card = browser.card.as_ref().unwrap();
    assert!(
        card.rows.contains(&("Vendor", "Rusty Lab".into())),
        "{card:?}"
    );

    let view = open_object(&mut site, "analog-output").await;
    assert_eq!(view.object, oid(ObjectType::ANALOG_OUTPUT, 1));
    assert_eq!(view.via, ReadVia::Rpm);
    let priority = view.priority.as_ref().expect("AO:1 is commandable");
    assert_eq!(priority.winner(), Some(10));
    assert_eq!(priority.slots[9].as_deref(), Some("55"));
    assert_eq!(priority.relinquish_default.as_deref(), Some("0"));
    assert_eq!(view.value(PropertyIdentifier::PRESENT_VALUE), Some("55 %"));
    for (w, h) in [(200, 50), (120, 40)] {
        let text = screen(&mut site.app, w, h);
        assert!(text.contains(">10 site defined"), "{text}");
        assert!(text.contains("Relinquish_Default  0"), "{text}");
        assert_snapshot!(format!("browser_ao1_{w}x{h}"), text);
    }
    // At 80 columns the priority array is a pane of its own (#1894), so
    // the properties keep the width; Tab reaches it.
    let text = screen(&mut site.app, 80, 24);
    assert!(!text.contains(">10 site defined"), "{text}");
    assert!(text.contains("[Properties]   Priority"), "{text}");
    assert_snapshot!("browser_ao1_80x24", text);
    press(&mut site.app, KeyCode::Tab);
    let text = screen(&mut site.app, 80, 24);
    assert!(text.contains(">10 site defined"), "{text}");
    assert!(text.contains("Relinquish_Default  0"), "{text}");
    assert_snapshot!("browser_ao1_priority_80x24", text);
    press(&mut site.app, KeyCode::Tab);
    assert_eq!(site.app.browser.as_ref().unwrap().focus, Pane::Card);
    site.stop().await;
}

/// AI:1 is out of service and in fault: the badges say so in text, not
/// only in colour.
#[tokio::test(start_paused = true)]
async fn ai1_out_of_service_and_in_fault_shows_both_badges() {
    let mut site = Site::start_with(vec![Server::lab(100, LAB)], ClientLimits::default()).await;
    open_device(&mut site).await;
    let view = open_object(&mut site, "analog-input 1").await;
    assert_eq!(view.object, oid(ObjectType::ANALOG_INPUT, 1));
    // In fault, its Event_State is FAULT too, so the device also reports
    // it in alarm.
    assert_eq!(view.health.badges(), ["[ALM]", "[FLT]", "[OOS]"]);
    assert!(view.priority.is_none(), "an input has no priority array");
    let stamps = view.value(PropertyIdentifier::EVENT_TIME_STAMPS).unwrap();
    assert!(
        stamps.starts_with("[seq 0, 20") && stamps.ends_with(", seq 0]"),
        "the to-fault stamp is a date and time: {stamps}"
    );
    // The to-fault stamp is the server's wall clock.
    assert_eq!(super::values::mask_open_object(&mut site.app), 1);
    for (w, h) in [(200, 50), (120, 40), (80, 24)] {
        let text = screen(&mut site.app, w, h);
        assert!(text.contains("Status: [ALM] [FLT] [OOS]"), "{text}");
        assert_snapshot!(format!("browser_ai1_{w}x{h}"), text);
    }
    // A 30-character name keeps its gap before the value.
    let text = screen(&mut site.app, 120, 40);
    assert!(
        text.contains("reliability-evaluation-inhibit  false"),
        "{text}"
    );
    // Colour adds nothing the text lacks.
    let plain = screen(&mut site.app, 120, 40);
    site.app.color = true;
    assert_eq!(screen(&mut site.app, 120, 40), plain);
    site.stop().await;
}

/// At 200 columns the priority array sits beside the properties and the
/// object list is wider than at 120.
#[tokio::test(start_paused = true)]
async fn a_wide_terminal_puts_the_priority_array_beside_the_properties() {
    let mut site = Site::start_with(vec![Server::lab(100, LAB)], ClientLimits::default()).await;
    open_device(&mut site).await;
    open_object(&mut site, "analog-output").await;
    let wide = screen(&mut site.app, 200, 50);
    let full = screen(&mut site.app, 120, 40);
    // Both panes start on the same row.
    let row = wide
        .lines()
        .find(|line| line.contains("┌ analog-output 1"))
        .expect("properties pane");
    assert!(row.contains("┌ Priority array: 10 in control"), "{wide}");
    let objects = |text: &str| {
        text.lines()
            .find(|line| line.contains("┌ Objects"))
            .and_then(|line| line.find('┐'))
            .expect("objects pane")
    };
    assert!(objects(&wide) > objects(&full), "{wide}\n{full}");
    site.stop().await;
}

/// A long value wraps under the value column at 80 columns, and the pane
/// scrolls through the wrapped lines to the last one.
#[tokio::test(start_paused = true)]
async fn a_long_value_wraps_under_its_column_at_80_columns() {
    let mut site = Site::start_with(vec![Server::lab(100, LAB)], ClientLimits::default()).await;
    open_device(&mut site).await;
    let mut view = open_object(&mut site, "analog-input 1").await;
    let long = "\"Zone 3 space temperature, north wall, beside the return grille \
                (sensor out for calibration until the next service visit)\"";
    for row in &mut view.properties {
        if row.property == PropertyIdentifier::DESCRIPTION {
            row.value = Ok(long.into());
        }
    }
    let browser = site.app.browser.as_mut().unwrap();
    browser.view = ViewState::Ready(Box::new(view));
    let text = screen(&mut site.app, 80, 24);
    // Continuation lines start under the value, not under the name.
    let under_value = format!("│{}beside the return grille", " ".repeat(33));
    assert!(text.contains(&under_value), "{text}");
    assert!(text.contains("next service visit)\""), "{text}");
    assert_snapshot!("browser_ai1_long_value_80x24", text);
    // End shows the last property, past the wrapped lines.
    site.press(KeyCode::End).await;
    let text = screen(&mut site.app, 80, 24);
    assert!(text.contains("event-algorithm-inhibit "), "{text}");
    // A long read error wraps too.
    let browser = site.app.browser.as_mut().unwrap();
    browser.view = ViewState::Failed(
        oid(ObjectType::ANALOG_INPUT, 1),
        "no reply from 10.0.0.100:47808 after 3 attempts; the device may be \
         offline or the router to its network may be down"
            .into(),
    );
    let text = screen(&mut site.app, 80, 24);
    assert!(text.contains("│ Read failed: no reply"), "{text}");
    assert!(text.contains("network may be down"), "{text}");
    site.stop().await;
}

/// The object list and values a device gives by RPM, as `(objects, AO:1)`.
async fn lab_read(refuse_rpm: bool) -> (Vec<ObjectIdentifier>, ObjectView, Option<ReadVia>) {
    let server = Server {
        quirks: Quirks {
            refuse_rpm,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    open_device(&mut site).await;
    let objects = listed(&site);
    let card_via = site
        .app
        .browser
        .as_ref()
        .unwrap()
        .card
        .as_ref()
        .unwrap()
        .via;
    let view = open_object(&mut site, "analog-output").await;
    site.stop().await;
    (objects, view, card_via)
}

/// A device that rejects RPM gives the same list and values by ReadProperty.
#[tokio::test(start_paused = true)]
async fn a_device_refusing_rpm_reads_the_same_by_read_property() {
    let (objects, by_rpm, card_via) = lab_read(false).await;
    assert_eq!(card_via, Some(ReadVia::Rpm));
    let (fallback_objects, by_rp, card_via) = lab_read(true).await;
    assert_eq!(card_via, Some(ReadVia::Rp));
    assert_eq!(by_rp.via, ReadVia::Rp);
    assert_eq!(fallback_objects, objects);
    assert_eq!(by_rp.priority, by_rpm.priority);
    assert_eq!(by_rp.health, by_rpm.health);
    // Every property RPM gave (bar Property_List, which ALL may include)
    // comes back by ReadProperty with the same value.
    for row in &by_rpm.properties {
        if row.property == PropertyIdentifier::PROPERTY_LIST {
            continue;
        }
        let fallback = by_rp.properties.iter().find(|r| r.property == row.property);
        assert_eq!(fallback, Some(row), "{}", row.property);
    }
}

/// A client that takes no segmented replies and small APDUs can't get a
/// 500-object Object_List in one answer; it reads it element by element.
#[tokio::test(start_paused = true)]
async fn a_large_object_list_is_read_element_by_element() {
    let server = Server {
        database: Some(super::lab::large_database(LAB, 500)),
        ..Server::bare(100, LAB)
    };
    let small = ClientLimits {
        max_apdu: 206,
        segmented: false,
    };
    let mut site = Site::start_with(vec![server], small).await;
    open_device(&mut site).await;
    let browser = site.app.browser.as_ref().unwrap();
    let objects = listed(&site);
    assert_eq!(
        browser.list,
        ListState::Ready { indexed: true },
        "{}",
        objects.len()
    );
    assert_eq!(objects.len(), 501, "500 analog values and the Device");
    assert!(objects.contains(&oid(ObjectType::ANALOG_VALUE, 500)));
    let text = screen(&mut site.app, 120, 40);
    assert!(text.contains("Objects (501)"), "{text}");
    assert!(
        text.contains("Object_List read element by element"),
        "{text}"
    );
    site.stop().await;
}

/// Reads queued behind the device read wait their turn: the device never
/// has two of our confirmed requests outstanding, and of two object reads
/// waiting, only the newer runs.
#[tokio::test(start_paused = true)]
async fn a_device_gets_one_request_at_a_time() {
    let mut site = Site::start_with(vec![Server::lab(100, LAB)], ClientLimits::default()).await;
    site.who_is("", 1).await;
    // Open the device and, before anything comes back, ask for two
    // objects as a fast typist would.
    let mut commands = press(&mut site.app, KeyCode::Enter);
    for (req, object) in [
        (1_001, oid(ObjectType::ANALOG_INPUT, 1)),
        (1_002, oid(ObjectType::ANALOG_OUTPUT, 1)),
    ] {
        commands.push(Command::Browse {
            req,
            device: LAB,
            read: BrowseRead::Object(object),
        });
    }
    site.send(commands).await;
    let mut answered = Vec::new();
    site.pump_events_until(|event| {
        if let WorkerEvent::Browse { req, result } = event {
            answered.push((*req, matches!(result, BrowseResult::Object(_))));
        }
        answered.contains(&(1_002, true))
    })
    .await;
    assert_eq!(site.hub.most_outstanding(&LAB_MAC), 1);
    assert!(
        !answered.iter().any(|(req, _)| *req == 1_001),
        "{answered:?}"
    );
    site.stop().await;
}

/// The services of ReadProperty and ReadPropertyMultiple.
const RP: u8 = 12;
const RPM: u8 = 14;

/// A device that stops answering fails the read once, by timeout, and is
/// not taken for one that refuses RPM: reading it again tries RPM again,
/// and no ReadProperty is ever sent to it.
#[tokio::test(start_paused = true)]
async fn a_silent_device_fails_once_and_keeps_rpm() {
    let server = Server {
        quirks: Quirks {
            silent: true,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    open_device(&mut site).await;
    let failed = |app: &App| {
        app.browser
            .as_ref()
            .is_some_and(|b| matches!(b.list, ListState::Failed(_)))
    };
    assert!(
        failed(&site.app),
        "{:?}",
        site.app.browser.as_ref().unwrap().list
    );
    let first = site.hub.services_to(&LAB_MAC);
    assert!(
        !first.is_empty() && first.iter().all(|s| *s == RPM),
        "{first:?}"
    );
    // Read it again: RPM first, as before.
    site.press(KeyCode::Char('r')).await;
    site.pump_until(failed).await;
    let services = site.hub.services_to(&LAB_MAC);
    assert!(services.len() > first.len());
    assert!(!services.contains(&RP), "{services:?}");
    site.stop().await;
}

/// An RPM answer too large to send falls back to ReadProperty for that
/// read only; the next object is read with RPM again.
#[tokio::test(start_paused = true)]
async fn a_size_abort_falls_back_for_that_read_only() {
    let server = Server {
        quirks: Quirks {
            abort_first_rpm: true,
            ..Quirks::default()
        },
        ..Server::lab(100, LAB)
    };
    let mut site = Site::start_with(vec![server], ClientLimits::default()).await;
    open_device(&mut site).await;
    let card_via = site
        .app
        .browser
        .as_ref()
        .unwrap()
        .card
        .as_ref()
        .unwrap()
        .via;
    assert_eq!(card_via, Some(ReadVia::Rp), "the aborted card read");
    let view = open_object(&mut site, "analog-input 2").await;
    assert_eq!(view.via, ReadVia::Rpm);
    site.stop().await;
}
