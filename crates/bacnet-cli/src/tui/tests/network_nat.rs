//! Our FDT entry behind a NAT router (#1943): with `--nat-global-address`,
//! the entry at the global address is ours, marked as translated. From
//! synthetic tables, and in process against a BBMD that sees the client
//! through a translating relay.

use std::net::{Ipv4Addr, SocketAddrV4};

use insta::assert_snapshot;
use ratatui::crossterm::event::KeyCode;
use tokio::net::UdpSocket;

use super::network::{Net, bbmd_server, local, server_address};
use super::network_frames::{LOCAL, at_ip, bdt_row, fdt_row, registered};
use super::*;
use crate::tui::app::network::View;
use crate::tui::network::{NetworkEvent, OwnEntry};
use crate::tui::profile::Profile;
use crate::tui::worker::session::{ConnectPlan, TransportKind};

/// Where the NAT router of these frames puts us.
const GLOBAL: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 61000);

/// `registered`, with the BBMD's FDT holding `fdt` and the connection's
/// global address `global`.
fn behind_nat(global: Option<SocketAddrV4>, fdt: Vec<SocketAddrV4>) -> App {
    let mut app = registered();
    app.network.foreign.as_mut().unwrap().nat_global = global;
    let tables = NetworkEvent::Tables {
        bbmd: at_ip(1),
        local: LOCAL,
        bdt: Ok(vec![bdt_row(1, [255; 4])]),
        fdt: Ok(fdt.into_iter().map(|at| fdt_row(at, 88)).collect()),
    };
    worker(&mut app, WorkerEvent::Network(tables));
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.network.view, View::Fdt);
    app
}

#[test]
fn the_entry_at_the_global_address_is_ours_and_translated_120x40() {
    let other = SocketAddrV4::new(Ipv4Addr::new(10, 0, 7, 9), 47808);
    let mut app = behind_nat(Some(GLOBAL), vec![other, GLOBAL]);
    assert_eq!(
        app.network.own,
        Some(OwnEntry {
            index: 1,
            translated: true
        })
    );
    let text = screen(&mut app, 120, 40);
    assert!(
        text.contains("As the BBMD sees us 203.0.113.7:61000  NAT-translated from 10.0.9.5:47808"),
        "{text}"
    );
    assert!(
        text.contains("203.0.113.7:61000      60 s      88 s      us, via NAT"),
        "{text}"
    );
    assert!(!text.contains("this client"), "{text}");
    assert_snapshot!("network_fdt_nat_120x40", text);
}

/// An entry at our own address still wins over the global address.
#[test]
fn an_entry_at_our_address_wins_over_the_global_address() {
    let mut app = behind_nat(Some(GLOBAL), vec![GLOBAL, LOCAL]);
    assert_eq!(
        app.network.own,
        Some(OwnEntry {
            index: 1,
            translated: false
        })
    );
    let text = screen(&mut app, 120, 40);
    assert!(!text.contains("NAT-translated"), "{text}");
    assert!(text.contains("this client"), "{text}");
}

/// No entry at either address: the hint names what was looked for, and
/// without a global address it points at the option.
#[test]
fn the_hint_names_the_option_or_the_global_address() {
    let other = SocketAddrV4::new(Ipv4Addr::new(10, 0, 7, 9), 47808);
    let mut app = behind_nat(None, vec![other, GLOBAL]);
    assert_eq!(app.network.own, None);
    let text = screen(&mut app, 200, 40);
    assert!(
        text.contains(
            "no FDT entry has our address 10.0.9.5:47808; a NAT router may have rewritten it \
             (name it with --nat-global-address)"
        ),
        "{text}"
    );
    let elsewhere = SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 8), 61000);
    let mut app = behind_nat(Some(elsewhere), vec![other, GLOBAL]);
    assert_eq!(app.network.own, None);
    let text = screen(&mut app, 200, 40);
    assert!(
        text.contains(
            "no FDT entry has our address 10.0.9.5:47808 or the NAT global address 203.0.113.8:61000"
        ),
        "{text}"
    );
}

fn v4(socket: &UdpSocket) -> SocketAddrV4 {
    match socket.local_addr().unwrap() {
        std::net::SocketAddr::V4(at) => at,
        std::net::SocketAddr::V6(_) => unreachable!(),
    }
}

/// A one-client NAT on loopback: the client talks to `front`, and the
/// BBMD sees every datagram come from `back`, the global address it
/// records in its FDT. Returns `(front, back)` and the relay task.
async fn nat(bbmd: SocketAddrV4) -> (SocketAddrV4, SocketAddrV4, tokio::task::JoinHandle<()>) {
    let front = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let back = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let (front_at, back_at) = (v4(&front), v4(&back));
    let relay = tokio::spawn(async move {
        let (mut out, mut inbound) = ([0u8; 1500], [0u8; 1500]);
        let mut client = None;
        loop {
            // Both arms are cancel safe: a datagram is taken whole or not.
            tokio::select! {
                Ok((n, from)) = front.recv_from(&mut out) => {
                    client = Some(from);
                    let _ = back.send_to(&out[..n], bbmd).await;
                }
                Ok((n, _)) = back.recv_from(&mut inbound) => {
                    if let Some(client) = client {
                        let _ = front.send_to(&inbound[..n], client).await;
                    }
                }
            }
        }
    });
    (front_at, back_at, relay)
}

/// The BBMD records the relay's address, not ours: the global address
/// finds the entry, and the screen says it was translated.
#[tokio::test]
async fn a_translated_registration_is_found_at_the_global_address() {
    let mut server = bbmd_server(3304, true).await;
    let (front, global, relay) = nat(server_address(&server)).await;
    let mut profile = Profile::default();
    let bip = profile.bip_mut();
    bip.interface = Ipv4Addr::LOCALHOST;
    bip.port = 0;
    bip.broadcast = Ipv4Addr::LOCALHOST;
    bip.bbmd = Some(front);
    bip.ttl = 60;
    bip.nat_global_address = Some(global);
    let plan = ConnectPlan {
        kind: TransportKind::Bip,
        args: profile.transport_args().unwrap(),
        await_interface: false,
    };
    let mut net = Net::start(plan).await;
    net.press(KeyCode::Char('4')).await;
    assert_eq!(net.app.network.foreign.unwrap().nat_global, Some(global));
    net.pump_until("our FDT entry", |app| app.network.own.is_some())
        .await;
    let own = net.app.network.own.unwrap();
    assert!(own.translated);
    let fdt = net
        .app
        .network
        .fdt
        .as_ref()
        .unwrap()
        .result
        .clone()
        .unwrap();
    assert_eq!(fdt[own.index].address, global);
    assert_ne!(global, local(&net.app));
    assert!(fdt.iter().all(|entry| entry.address != local(&net.app)));
    net.press(KeyCode::Tab).await;
    let text = screen(&mut net.app, 160, 40);
    let line = format!(
        "As the BBMD sees us {global}  NAT-translated from {}",
        local(&net.app)
    );
    assert!(text.contains(&line), "{text}");
    assert!(text.contains("us, via NAT"), "{text}");
    net.stop().await;
    relay.abort();
    server.stop().await.unwrap();
}
