//! Capture from real sources (#971): the frame tap of a B/IP client on
//! loopback, through the capture worker and through the coordinator, and
//! the MS/TP monitor's stream on the scripted ring.

use std::net::Ipv4Addr;

use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;

use super::capture::capturing;
use super::network::{Net, plan};
use super::site::bounded;
use super::*;
use crate::tui::capture::{CaptureCommand, CaptureEvent, Direction, worker as capture_worker};
use crate::tui::worker::{EVENT_CHANNEL_CAPACITY, EventSink};

/// The lab device on loopback.
async fn lab_server(instance: u32) -> BACnetServer<BipTransport> {
    BACnetServer::bip_builder()
        .device(super::lab::lab_identity(instance))
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(super::lab::lab_database(instance))
        .build()
        .await
        .unwrap()
}

fn filter(app: &mut App, text: &str) {
    press(app, KeyCode::Char('/'));
    type_text(app, text);
    press(app, KeyCode::Enter);
}

/// The issue's first acceptance check: an in-process ReadProperty over B/IP
/// shows the tapped request and response, and the detail pane the matching
/// invoke ID and service; an unknown property's Error is the only frame
/// "service ReadProperty, errors only" keeps.
#[tokio::test]
async fn a_read_property_over_bip_shows_the_tapped_request_and_response() {
    let mut server = lab_server(4321).await;
    let target = server.local_mac().to_vec();
    let mut client = crate::transport::build_bip_client(&plan(None).args)
        .await
        .unwrap();
    let (sink, mut events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let (start, commands) = mpsc::channel(4);
    let mut app = capturing();
    let ai1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let exchange = async {
        start.send(CaptureCommand::Start).await.unwrap();
        loop {
            let event = events.recv().await.unwrap();
            let started = matches!(event, WorkerEvent::Capture(CaptureEvent::Started { .. }));
            worker(&mut app, event);
            if started {
                break;
            }
        }
        let value = client.read_property(&target, ai1, PropertyIdentifier::PRESENT_VALUE, None);
        assert!(value.await.is_ok());
        let refused = client.read_property(&target, ai1, PropertyIdentifier::FILE_SIZE, None);
        assert!(refused.await.is_err());
        while app.capture.ring.len() < 4 {
            worker(&mut app, events.recv().await.unwrap());
        }
        drop(start);
    };
    let pump = capture_worker::run(&client, &sink, commands, AddressStyle::Bip);
    bounded(async { tokio::join!(pump, exchange) }).await;

    let ring = &app.capture.ring;
    let frames: Vec<_> = (1..=4).map(|n| ring.get(n).unwrap().clone()).collect();
    let directions: Vec<Direction> = frames.iter().map(|f| f.direction).collect();
    assert_eq!(
        directions,
        [Direction::Out, Direction::In, Direction::Out, Direction::In]
    );
    let server_address = bacnet_ops::target::format_mac(&target);
    assert_eq!(frames[0].destination, server_address);
    assert_eq!(frames[1].source, server_address);
    let pdus: Vec<_> = frames
        .iter()
        .map(|f| f.summary.pdu.clone().unwrap())
        .collect();
    assert_eq!(
        pdus,
        [
            "Confirmed-Request",
            "Complex-ACK",
            "Confirmed-Request",
            "Error"
        ]
    );
    let invoke = frames[0].summary.invoke_id.unwrap();
    assert_eq!(frames[1].summary.invoke_id, Some(invoke));
    assert!(
        frames.iter().all(|f| f.udp.is_some()),
        "endpoints for export"
    );

    // The response, selected, decodes to its invoke ID and service.
    press(&mut app, KeyCode::Home);
    press(&mut app, KeyCode::Down);
    app.settle();
    let detail = app.capture.detail.as_ref().unwrap();
    assert_eq!(detail.number, 2);
    let labels = flat_labels(&detail.fields);
    assert!(
        labels.contains(&format!("Invoke ID: {invoke}")),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"Service: read-property".to_string()),
        "{labels:?}"
    );
    assert!(labels.iter().any(|l| l == "Value: 72.5"), "{labels:?}");

    filter(&mut app, "svc:ReadProperty errors");
    assert_eq!(app.capture.visible().collect::<Vec<_>>(), [4]);
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

fn flat_labels(fields: &[bacnet_ops::decode::tree::Field]) -> Vec<String> {
    let mut out = Vec::new();
    for field in fields {
        out.push(field.label.clone());
        out.extend(flat_labels(&field.children));
    }
    out
}

/// Through the coordinator: opening the screen subscribes the session's
/// tap, so the Who-Is the form sends appears; a new session subscribes
/// again by itself.
#[tokio::test]
async fn the_session_tap_reaches_the_screen_through_the_coordinator() {
    let mut net = Net::start(plan(None)).await;
    net.press(KeyCode::Char('7')).await;
    net.pump_until("the tap", |app| !app.capture.sources.is_empty())
        .await;
    net.press(KeyCode::Char('1')).await;
    net.press(KeyCode::Char('d')).await;
    // An unbounded Who-Is asks for a second Enter.
    net.press(KeyCode::Enter).await;
    net.press(KeyCode::Enter).await;
    net.pump_until("the Who-Is", |app| app.capture.ring.len() > 0)
        .await;
    let first = net.app.capture.ring.get(1).unwrap();
    assert_eq!(first.summary.service.as_deref(), Some("who-is"));
    assert_eq!(first.direction, Direction::Out);
    assert!(first.summary.link.contains("broadcast"), "{first:?}");

    net.press(KeyCode::Char('x')).await;
    net.pump_until("down", |app| app.link == crate::tui::app::Link::Down)
        .await;
    assert!(net.app.capture.sources.is_empty(), "the tap went with it");
    assert_eq!(
        net.app.capture.ring.len() as u64,
        net.app.capture.ring.next() - 1
    );
    net.press(KeyCode::Char('c')).await;
    net.pump_until("the tap again", |app| !app.capture.sources.is_empty())
        .await;
    net.stop().await;
}

/// The monitor's own stream feeds the screen: tokens and data frames from
/// the scripted ring, and the monitor still writes nothing.
#[tokio::test(start_paused = true)]
async fn the_monitor_feeds_the_capture_and_still_writes_nothing() {
    use super::mstp_monitor::{Run, acceptance_line};
    let mut run = Run::start(acceptance_line(), None, false).await;
    run.listen_for(Duration::ZERO).await;
    run.press(KeyCode::Char('7')).await;
    run.listen_for(Duration::from_secs(6)).await;
    let capture = &run.app.capture;
    assert_eq!(
        capture.sources,
        [("MS/TP monitor, every frame on the line".to_string(), true)]
    );
    let links: std::collections::BTreeSet<String> = (1..capture.ring.next())
        .filter_map(|n| capture.ring.get(n))
        .map(|f| f.summary.link.clone())
        .collect();
    for link in [
        "token",
        "bacnet-data-expecting-reply",
        "bacnet-data-not-expecting-reply",
    ] {
        assert!(links.contains(link), "{link}: {links:?}");
    }
    assert_eq!(capture.lost, 0);
    let token = (1..capture.ring.next())
        .filter_map(|n| capture.ring.get(n))
        .find(|f| f.summary.link == "token")
        .unwrap();
    assert_eq!(&token.bytes[..2], &[0x55, 0xFF], "from the preamble");
    assert_eq!(token.direction, Direction::Wire);
    assert_eq!(run.quit().await, b"", "the monitor wrote to the line");
}
