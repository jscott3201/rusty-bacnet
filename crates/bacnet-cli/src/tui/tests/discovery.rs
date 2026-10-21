//! Who-Has search and the Networks tab (#960): the app's side from
//! synthetic worker events, with frames, and the worker against a real
//! BBMD and a real `BACnetRouter` on loopback.

use std::net::Ipv4Addr;

use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_server::server::BACnetServer;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::bbmd::ForeignDevicePolicy;
use bacnet_transport::bip::{BbmdOptions, BipMode, BipTransport};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::mstp::NoSerial;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;
use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;

use super::network::{plan, Net};
use super::*;
use crate::tui::app::network::View;
use crate::tui::app::watch::Screen;
use crate::tui::network::discovery::{DiscoveryCommand, DiscoveryEvent, FoundRow};

const NAME: &str = "Supply Air Temp";

fn found(device: u32, instance: u32, name: &str) -> FoundRow {
    FoundRow {
        device,
        object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, instance).unwrap(),
        name: name.into(),
        address: format!("10.0.0.{}:47808", device % 250),
        via: None,
        network: None,
    }
}

fn discovery(app: &mut App, event: DiscoveryEvent) {
    worker(app, WorkerEvent::Discovery(event));
}

fn router(app: &mut App, router: &str, networks: &[u16]) {
    let event = DiscoveryEvent::Router {
        router: router.into(),
        networks: networks.to_vec(),
    };
    discovery(app, event);
}

#[test]
fn find_sends_a_who_has_and_keeps_the_answers_to_it() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, &format!("find {NAME}"));
    let commands = press(&mut app, KeyCode::Enter);
    assert_eq!(
        commands,
        [Command::Discovery(DiscoveryCommand::WhoHas {
            name: NAME.into()
        })]
    );
    discovery(&mut app, DiscoveryEvent::IHave(found(40, 3, NAME)));
    // Another name, and the same device and object again, are not kept.
    discovery(
        &mut app,
        DiscoveryEvent::IHave(found(41, 1, "Return Air Temp")),
    );
    discovery(&mut app, DiscoveryEvent::IHave(found(40, 3, NAME)));
    discovery(&mut app, DiscoveryEvent::IHave(found(42, 9, NAME)));
    let search = app.who_has.as_ref().unwrap();
    let devices: Vec<u32> = search.found.iter().map(|row| row.device).collect();
    assert_eq!(devices, [40, 42]);
    assert_snapshot!("who_has_80x24", screen(&mut app, 80, 24));

    // `:find` alone closes the panel.
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "find");
    assert_eq!(press(&mut app, KeyCode::Enter), []);
    assert_eq!(app.who_has, None);
}

#[test]
fn a_refused_who_has_is_shown_and_a_reject_is_flashed() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "find Fan");
    press(&mut app, KeyCode::Enter);
    let command = DiscoveryCommand::WhoHas { name: "Fan".into() };
    let reason = "no active connection".to_string();
    discovery(&mut app, DiscoveryEvent::Refused { command, reason });
    assert_eq!(
        app.who_has.as_ref().unwrap().failed.as_deref(),
        Some("no active connection")
    );
    // The reason as the worker writes it; its kebab-case conversion is
    // tested in `worker::discovery`. The app flashes it as given.
    let reject = DiscoveryEvent::Rejected {
        router: "10.0.0.1:47808".into(),
        network: 99,
        reason: "not-directly-connected".into(),
    };
    discovery(&mut app, reject);
    let flash = &app.flash.as_ref().unwrap().text;
    assert!(flash.contains("network 99") && flash.contains("not-directly-connected"));
}

#[test]
fn passive_and_unconnected_sessions_send_nothing() {
    let mut app = new_app(None);
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "find Fan");
    assert_eq!(press(&mut app, KeyCode::Enter), []);
    assert_eq!(app.who_has, None);
    assert!(app.flash.as_ref().unwrap().text.contains("Not connected"));
}

/// Two routers claiming network 3 are flagged on the Networks tab.
#[test]
fn the_networks_tab_flags_a_network_two_routers_claim() {
    let mut app = connected_app();
    press(&mut app, KeyCode::Char('4'));
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab);
    }
    assert_eq!(app.network.view, View::Networks);
    assert_eq!(
        press(&mut app, KeyCode::Char('r')),
        [Command::Discovery(DiscoveryCommand::WhoIsRouter)]
    );
    router(&mut app, "10.0.0.1:47808", &[2, 3]);
    router(&mut app, "10.0.0.9:47808", &[4]);
    assert!(app.network.routers.duplicates().is_empty());
    router(&mut app, "10.0.0.7:47808", &[3]);
    let event = DiscoveryEvent::NetworkNumber {
        network: 1,
        configured: true,
    };
    discovery(&mut app, event);
    assert_eq!(
        app.network
            .routers
            .duplicates()
            .into_iter()
            .collect::<Vec<_>>(),
        [3]
    );
    let rows = app.network.routers.rows();
    assert!(rows.iter().all(|row| row.duplicate() == (row.network == 3)));
    let frame = screen(&mut app, 120, 40);
    assert!(frame.contains("DUPLICATE"), "{frame}");
    assert_snapshot!("networks_tab_120x40", frame);
    assert_snapshot!("networks_tab_80x24", screen(&mut app, 80, 24));

    // The route to 3 moved to 10.0.0.7: a new search starts from nothing,
    // so the old claim no longer flags it.
    assert_eq!(
        press(&mut app, KeyCode::Char('r')),
        [Command::Discovery(DiscoveryCommand::WhoIsRouter)]
    );
    assert!(app.network.routers.is_empty());
    router(&mut app, "10.0.0.1:47808", &[2]);
    router(&mut app, "10.0.0.7:47808", &[3]);
    assert!(app.network.routers.duplicates().is_empty());
    assert!(!screen(&mut app, 120, 40).contains("DUPLICATE"));
}

async fn bbmd_with_supply_air(instance: u32) -> BACnetServer<BipTransport> {
    let mut db = super::lab::lab_database(instance);
    db.add(Box::new(
        AnalogInputObject::new(7, NAME, EngineeringUnits::DEGREES_FAHRENHEIT).unwrap(),
    ))
    .unwrap();
    BACnetServer::bip_builder()
        .device(super::lab::lab_identity(instance))
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .mode(BipMode::Bbmd(BbmdOptions {
            foreign_device_policy: Some(ForeignDevicePolicy::default()),
            ..BbmdOptions::default()
        }))
        .build()
        .await
        .unwrap()
}

/// `:find` through the BBMD reaches its device, whose I-Have comes back
/// forwarded and fills the panel.
#[tokio::test]
async fn find_through_the_bbmd_lists_its_device() {
    let mut server = bbmd_with_supply_air(3310).await;
    let mac = server.local_mac().to_vec();
    let bbmd =
        std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, u16::from_be_bytes([mac[4], mac[5]]));
    let mut net = Net::start(plan(Some(bbmd))).await;
    net.pump_until("registered", |app| {
        app.network
            .registration
            .is_some_and(|(snapshot, _)| snapshot.registered)
    })
    .await;
    net.command_line(&format!("find {NAME}")).await;
    net.pump_until("the I-Have", |app| {
        app.who_has.as_ref().is_some_and(|s| !s.found.is_empty())
    })
    .await;
    let row = &net.app.who_has.as_ref().unwrap().found[0];
    assert_eq!((row.device, row.address.clone()), (3310, bbmd.to_string()));
    net.stop().await;
    server.stop().await.unwrap();
}

/// `r` on the Networks tab asks the routers; the `BACnetRouter` whose B/IP
/// port is the BBMD answers with network 2 behind it.
#[tokio::test]
async fn the_networks_tab_maps_a_real_router() {
    let port = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut bip = BipTransport::new(Ipv4Addr::LOCALHOST, port, Ipv4Addr::LOCALHOST);
    bip.set_mode(BipMode::Bbmd(BbmdOptions {
        foreign_device_policy: Some(ForeignDevicePolicy::default()),
        ..BbmdOptions::default()
    }));
    let (far, _station) = LoopbackTransport::pair(vec![0x01], vec![0x05]);
    let ports = vec![
        RouterPort {
            transport: AnyTransport::<NoSerial>::Bip(Box::new(bip)),
            network_number: 1,
        },
        RouterPort {
            transport: AnyTransport::Loopback(far),
            network_number: 2,
        },
    ];
    let StartedRouter { mut router, .. } = BACnetRouter::start(ports, RouterOptions::new())
        .await
        .expect("the router binds its port");
    let bbmd = std::net::SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let mut net = Net::start(plan(Some(bbmd))).await;
    net.pump_until("registered", |app| {
        app.network
            .registration
            .is_some_and(|(snapshot, _)| snapshot.registered)
    })
    .await;
    net.press(KeyCode::Char('4')).await;
    while net.app.network.view != View::Networks {
        net.press(KeyCode::Tab).await;
    }
    let asked = press(&mut net.app, KeyCode::Char('r'));
    assert_eq!(asked, [Command::Discovery(DiscoveryCommand::WhoIsRouter)]);
    net.send(asked).await;
    net.pump_until("the router", |app| !app.network.routers.is_empty())
        .await;
    let rows = net.app.network.routers.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].network, 2);
    assert_eq!(rows[0].routers, [bbmd.to_string()]);
    assert_eq!(net.app.screen, Screen::Network);
    net.stop().await;
    router.stop().await;
}
