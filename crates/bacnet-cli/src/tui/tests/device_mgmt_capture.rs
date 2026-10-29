//! The capture keeps DCC and ReinitializeDevice passwords masked (#1799):
//! a B/IP client on loopback sends both, with the capture running, to a
//! lab server that takes the password. The ring holds the requests with
//! `*` for each password character and marked redacted, the detail pane
//! says so, and neither the frames drawn nor a pcap export to the
//! workspace holds the password.

use std::net::Ipv4Addr;

use bacnet_server::server::{BACnetServer, DccPolicy};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{EnableDisable, ReinitializedState};
use ratatui::crossterm::event::KeyCode;
use tokio::sync::mpsc;

use super::capture::capturing;
use super::device_mgmt::PASSWORD;
use super::network::plan;
use super::site::bounded;
use super::*;
use crate::tui::capture::{CaptureCommand, CaptureEvent, Direction, worker as capture_worker};
use crate::tui::worker::{EVENT_CHANNEL_CAPACITY, EventSink};

/// The lab device on loopback, taking `PASSWORD` for both services.
async fn lab_server(instance: u32) -> BACnetServer<BipTransport> {
    BACnetServer::bip_builder()
        .device(super::lab::lab_identity(instance))
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(super::lab::lab_database(instance))
        .dcc_password(PASSWORD)
        .dcc_policy(DccPolicy::RequirePassword)
        .reinit_password(PASSWORD)
        .on_reinitialize(|_, _| Ok(()))
        .build()
        .await
        .unwrap()
}

/// The frames of either service, requests and replies, in ring order.
fn ours(ring: &crate::tui::capture::ring::Ring) -> Vec<&crate::tui::capture::CaptureFrame> {
    let services = ["device-communication-control", "reinitialize-device"];
    ring.kept()
        .iter()
        .filter_map(|n| ring.get(*n))
        .filter(|f| {
            f.summary
                .service
                .as_deref()
                .is_some_and(|s| services.contains(&s))
        })
        .collect()
}

#[tokio::test]
async fn the_capture_masks_the_password_on_screen_and_in_the_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut server = lab_server(4322).await;
    let target = server.local_mac().to_vec();
    let mut client = crate::transport::build_bip_client(&plan(None).args)
        .await
        .unwrap();
    let (sink, mut events) = EventSink::channel(EVENT_CHANNEL_CAPACITY);
    let (start, commands) = mpsc::channel(4);
    let mut app = capturing();
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
        let password = Some(PASSWORD.to_string());
        let dcc = client.device_communication_control(
            &target,
            EnableDisable::DISABLE_INITIATION,
            Some(15),
            password.clone(),
        );
        assert!(dcc.await.is_ok());
        let reinit = client.reinitialize_device(&target, ReinitializedState::WARMSTART, password);
        assert!(reinit.await.is_ok());
        // Both requests and both acknowledgements, whatever else the
        // session sent.
        while ours(&app.capture.ring).len() < 4 {
            worker(&mut app, events.recv().await.unwrap());
        }
        drop(start);
    };
    let pump = capture_worker::run(&client, &sink, commands, AddressStyle::Bip);
    bounded(async { tokio::join!(pump, exchange) }).await;

    let ring = &app.capture.ring;
    let frames: Vec<_> = ring.kept().iter().map(|n| ring.get(*n).unwrap()).collect();
    let ours = ours(ring);
    let requests: Vec<_> = ours
        .iter()
        .filter(|f| f.direction == Direction::Out)
        .collect();
    assert_eq!(requests.len(), 2, "{frames:?}");
    for frame in &requests {
        assert!(frame.redacted, "{frame:?}");
        let text = String::from_utf8_lossy(&frame.bytes);
        assert!(!text.contains(PASSWORD), "{frame:?}");
    }
    // Same length, so the request still decodes, its password masked.
    let apdu = &requests[0].bytes[6..];
    let sent = DeviceCommunicationControlRequest::decode(&apdu[4..]).unwrap();
    assert_eq!(
        sent.password.as_deref(),
        Some("*".repeat(PASSWORD.len()).as_str())
    );
    assert_eq!(sent.time_duration, Some(15));
    let replies: Vec<_> = ours
        .iter()
        .filter(|f| f.direction == Direction::In)
        .collect();
    assert_eq!(replies.len(), 2, "{frames:?}");
    assert!(replies.iter().all(|f| !f.redacted));

    // The export, to the workspace.
    let commands = press(&mut app, KeyCode::Char('e'));
    let [Command::CaptureExport(job)] = commands.as_slice() else {
        panic!("{commands:?}");
    };
    let exported = crate::tui::capture::pcap::export(job, dir.path());
    assert_eq!(exported.error, None);
    assert_eq!(exported.written.len(), 1, "{exported:?}");
    let mut drawn = Vec::new();
    for filter in [
        "svc:DeviceCommunicationControl pdu:confirmed",
        "svc:ReinitializeDevice pdu:confirmed",
    ] {
        press(&mut app, KeyCode::Char('/'));
        type_text(&mut app, filter);
        press(&mut app, KeyCode::Enter);
        let wide = screen(&mut app, 120, 40);
        assert!(wide.contains("(password redacted)"), "{wide}");
        drawn.push(wide);
        // At 80 columns the hex view is a pane of its own.
        press(&mut app, KeyCode::Tab);
        press(&mut app, KeyCode::Tab);
        drawn.push(screen(&mut app, 80, 24));
        press(&mut app, KeyCode::Char('/'));
        press(&mut app, KeyCode::Esc);
    }
    for frame in &drawn {
        assert!(!frame.contains(PASSWORD), "{frame}");
    }
    let mut files = 0;
    for entry in std::fs::read_dir(dir.path()).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains(PASSWORD));
        let masked = "*".repeat(PASSWORD.len());
        assert!(
            bytes.windows(masked.len()).any(|w| w == masked.as_bytes()),
            "the export holds the masked requests"
        );
        files += 1;
    }
    assert_eq!(files, 1, "the pcap");
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
