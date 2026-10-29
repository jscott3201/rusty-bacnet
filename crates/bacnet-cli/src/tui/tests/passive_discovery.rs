//! Receive-only discovery and local searches.
use super::*;
use crate::tui::network::discovery::{DiscoveryEvent, FoundRow};
use bacnet_types::{enums::ObjectType, primitives::ObjectIdentifier};

#[test]
fn passive_find_filters_retained_observations_without_commands() {
    let mut app = new_app(None);
    app.safety.passive = true;
    worker(
        &mut app,
        WorkerEvent::PassiveListening {
            local: "127.0.0.1:47808".into(),
            foreign_ignored: false,
        },
    );
    let row = FoundRow {
        device: 12,
        object: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        name: "Supply".into(),
        address: "127.0.0.1:47808".into(),
        via: None,
        network: None,
    };
    worker(
        &mut app,
        WorkerEvent::Discovery(DiscoveryEvent::IHave(row.clone())),
    );
    press(&mut app, KeyCode::Char(':'));
    type_text(&mut app, "find Supply");
    assert!(press(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        app.who_has.as_ref().expect("local passive search").found,
        [row]
    );
}

use crate::tui::worker::{
    EventSink,
    session::{self, ConnectPlan, SessionOptions},
};
use bacnet_encoding::{
    apdu::{Apdu, UnconfirmedRequest, encode_apdu},
    npdu::{Npdu, NpduAddress, encode_npdu},
};
use bacnet_services::who_has::IHaveRequest;
use bacnet_transport::bvll::Bvll;
use bytes::BytesMut;
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::{net::UdpSocket, sync::mpsc, task::JoinHandle};

pub(super) fn options() -> SessionOptions {
    let mut options = SessionOptions::default();
    options.safety.passive = true;
    options
}

pub(super) fn wire(message: Bvll<'_>) -> Vec<u8> {
    let mut wire = vec![0; 65_535];
    let n = message.encode_into(&mut wire).unwrap();
    wire.truncate(n);
    wire
}

pub(super) fn i_have(name: &str, routed: bool) -> Vec<u8> {
    let mut service = BytesMut::new();
    IHaveRequest {
        device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 12).unwrap(),
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        object_name: name.into(),
    }
    .encode(&mut service)
    .unwrap();
    let apdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: bacnet_types::enums::UnconfirmedServiceChoice::I_HAVE,
        service_request: service.freeze(),
    });
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, &apdu).unwrap();
    let mut npdu = Npdu {
        payload: payload.freeze(),
        ..Npdu::default()
    };
    if routed {
        npdu.source = Some(NpduAddress {
            network: 7,
            mac_address: bacnet_types::MacAddr::from_slice(&[5]),
        });
    }
    let mut encoded = BytesMut::new();
    encode_npdu(&mut encoded, &npdu).unwrap();
    encoded.to_vec()
}

pub(super) struct Passive {
    pub(super) app: App,
    pub(super) commands: mpsc::Sender<Command>,
    pub(super) events: mpsc::Receiver<WorkerEvent>,
    pub(super) task: JoinHandle<Result<(), String>>,
    pub(super) address: SocketAddrV4,
}
impl Passive {
    pub(super) async fn start(plan: ConnectPlan) -> Self {
        let (sink, mut events) = EventSink::channel(32);
        let (commands, command_rx) = mpsc::channel(16);
        let task = session::spawn(plan, sink, command_rx, options());
        let mut app = new_app(None);
        app.safety.passive = true;
        let event = events.recv().await.unwrap();
        let WorkerEvent::PassiveListening { ref local, .. } = event else {
            panic!("{event:?}")
        };
        let address = local.parse().unwrap();
        worker(&mut app, event);
        let event = events.recv().await.unwrap();
        assert!(matches!(event, WorkerEvent::Safety(_)));
        worker(&mut app, event);
        Self {
            app,
            commands,
            events,
            task,
            address,
        }
    }
    pub(super) async fn observation(&mut self) -> DiscoveryEvent {
        loop {
            let event = self.events.recv().await.expect("listener still running");
            let discovery = match &event {
                WorkerEvent::Discovery(event) => Some(event.clone()),
                _ => None,
            };
            worker(&mut self.app, event);
            if let Some(discovery) = discovery {
                return discovery;
            }
        }
    }
    pub(super) fn find(&mut self, name: &str) {
        press(&mut self.app, KeyCode::Char(':'));
        type_text(&mut self.app, &format!("find {name}"));
        assert!(
            press(&mut self.app, KeyCode::Enter).is_empty(),
            "local find sends no commands"
        );
    }
    pub(super) async fn stop(self) {
        self.commands.send(Command::Disconnect).await.unwrap();
        self.task.await.unwrap().unwrap();
        let rebound = std::net::UdpSocket::bind(self.address).expect("one-shot immediate rebind");
        drop(rebound);
    }
}

#[tokio::test]
async fn passive_receives_real_router_startup_after_listener_readiness() {
    use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
    use bacnet_transport::{
        any::AnyTransport, bbmd::BdtEntry, bip::BipTransport, loopback::LoopbackTransport,
    };
    let mut net = Passive::start(super::network::plan(None)).await;
    let mut bip = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    // A real BBMD forwards the router's startup announcement to the already
    // bound listener; no discovery request triggers it (Clauses 6.6.2/6.6.3.3).
    bip.enable_bbmd(vec![BdtEntry {
        ip: net.address.ip().octets(),
        port: net.address.port(),
        broadcast_mask: [255; 4],
    }]);
    let (far, _station) = LoopbackTransport::pair(vec![1], vec![5]);
    let StartedRouter { mut router, .. } = BACnetRouter::start(
        vec![
            RouterPort {
                transport: AnyTransport::Bip(Box::new(bip)),
                network_number: 1,
            },
            RouterPort {
                transport: AnyTransport::Loopback(far),
                network_number: 2,
            },
        ],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    let (origin, networks) = loop {
        match net.observation().await {
            DiscoveryEvent::Router { router, networks } => break (router, networks),
            DiscoveryEvent::NetworkNumber { network, .. } => assert_eq!(network, 1),
            other => panic!("unexpected startup observation: {other:?}"),
        }
    };
    assert_eq!(networks, [2]);
    let rows = net.app.network.routers.rows();
    assert_eq!(rows[0].network, 2);
    assert_eq!(rows[0].routers.as_slice(), std::slice::from_ref(&origin));
    assert!(origin.starts_with("127.0.0.1:"));
    net.app.screen = crate::tui::app::watch::Screen::Network;
    net.app.network.view = crate::tui::app::network::View::Networks;
    let frame = screen(&mut net.app, 160, 40);
    assert!(
        frame.contains(&origin) && frame.contains("listening for announcements"),
        "{frame}"
    );
    router.stop().await.unwrap();
    net.stop().await;
}

#[tokio::test]
async fn passive_native_ihaves_keep_local_forwarded_and_routed_stations() {
    let mut net = Passive::start(super::network::plan(None)).await;
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let local = i_have("Supply", false);
    peer.send_to(&wire(Bvll::OriginalUnicastNpdu(&local)), net.address)
        .await
        .unwrap();
    let DiscoveryEvent::IHave(first) = net.observation().await else {
        panic!("I-Have")
    };
    assert_eq!(first.address, peer.local_addr().unwrap().to_string());
    assert_eq!(
        net.app.names.known(12, &first.address, first.object),
        Some("Supply")
    );
    net.find("supply");
    assert!(
        net.app.who_has.as_ref().unwrap().found.is_empty(),
        "exact name match"
    );
    net.find("Supply");
    assert_eq!(
        net.app.who_has.as_ref().unwrap().found.as_slice(),
        std::slice::from_ref(&first)
    );
    let origin = SocketAddrV4::new(Ipv4Addr::new(10, 22, 0, 3), 47808);
    peer.send_to(
        &wire(Bvll::ForwardedNpdu {
            origin,
            npdu: &local,
        }),
        net.address,
    )
    .await
    .unwrap();
    let DiscoveryEvent::IHave(forwarded) = net.observation().await else {
        panic!("I-Have")
    };
    assert_eq!(forwarded.address, origin.to_string());
    let routed = i_have("Supply", true);
    peer.send_to(
        &wire(Bvll::ForwardedNpdu {
            origin,
            npdu: &routed,
        }),
        net.address,
    )
    .await
    .unwrap();
    let DiscoveryEvent::IHave(routed) = net.observation().await else {
        panic!("I-Have")
    };
    assert_eq!(routed.address, "7:0x05");
    assert_eq!(routed.via, Some(origin.to_string()));
    assert_eq!(routed.network, Some(7));
    assert_eq!(
        net.app.who_has.as_ref().unwrap().found,
        [first, forwarded, routed]
    );
    let frame = screen(&mut net.app, 160, 40);
    assert!(
        frame.contains("Listening for")
            && frame.contains("overheard")
            && !frame.contains("sent 0s"),
        "{frame}"
    );
    net.stop().await;
}

#[tokio::test]
async fn passive_zero_send_attempts_with_active_positive_control_and_hostile_traffic() {
    use crate::tui::network::discovery::DiscoveryCommand;
    use bacnet_transport::{
        bip::{BipTransport, SendAttemptObserver},
        port::TransportPort,
    };
    let reserved = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = match reserved.local_addr().unwrap() {
        std::net::SocketAddr::V4(a) => a,
        _ => unreachable!(),
    };
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    drop(reserved);
    let positive = SendAttemptObserver::new(address.port());
    let mut active = BipTransport::new(Ipv4Addr::LOCALHOST, address.port(), Ipv4Addr::LOCALHOST);
    let _received = active.start().await.unwrap();
    peer.send_to(&wire(Bvll::ReadBroadcastDistributionTable), address)
        .await
        .unwrap();
    let mut reply = [0; 128];
    let (n, from) = peer.recv_from(&mut reply).await.unwrap();
    assert_eq!(from.port(), address.port());
    assert_eq!(
        reply[1], 0,
        "normal node responds with a BVLC result, {n} bytes"
    );
    assert!(
        positive.attempts() > 0,
        "oracle observes the actual automatic send boundary"
    );
    active.stop().await.unwrap();
    drop(positive);
    let observer = SendAttemptObserver::new(address.port());
    let bbmd = match peer.local_addr().unwrap() {
        std::net::SocketAddr::V4(a) => a,
        _ => unreachable!(),
    };
    let mut plan = super::network::plan(Some(bbmd));
    plan.args.port = address.port();
    let mut net = Passive::start(plan).await;
    assert_eq!(net.address, address);
    assert!(
        net.app
            .flash
            .as_ref()
            .unwrap()
            .text
            .contains("no registration")
    );
    net.find("marker");
    assert!(
        press(&mut net.app, KeyCode::Char('d')).is_empty(),
        "Who-Is key stays silent"
    );
    net.app.screen = crate::tui::app::watch::Screen::Network;
    net.app.network.view = crate::tui::app::network::View::Networks;
    assert!(
        press(&mut net.app, KeyCode::Char('r')).is_empty(),
        "router discovery key stays silent"
    );
    for command in [
        DiscoveryCommand::WhoIsRouter,
        DiscoveryCommand::WhoHas {
            name: "marker".into(),
        },
    ] {
        net.commands
            .send(Command::Discovery(command.clone()))
            .await
            .unwrap();
        assert!(
            matches!(net.observation().await, DiscoveryEvent::Refused { command: refused, .. } if refused == command)
        );
    }
    // Native normal-node NAK, foreign registration and table management stimuli.
    for message in [
        Bvll::ReadBroadcastDistributionTable,
        Bvll::ReadForeignDeviceTable,
        Bvll::RegisterForeignDevice { ttl: 60 },
        Bvll::DistributeBroadcastToNetwork(&[1, 0, 0x10, 8]),
    ] {
        peer.send_to(&wire(message), address).await.unwrap();
    }
    // Who-Is and a confirmed ReadProperty request; neither may produce an answer.
    for npdu in [
        &[1, 0, 0x10, 8][..],
        &[1, 4, 0, 5, 1, 12, 0x0c, 2, 0, 0, 12, 0x19, 77][..],
    ] {
        peer.send_to(&wire(Bvll::OriginalUnicastNpdu(npdu)), address)
            .await
            .unwrap();
    }
    let marker = i_have("marker", false);
    peer.send_to(&wire(Bvll::OriginalUnicastNpdu(&marker)), address)
        .await
        .unwrap();
    assert!(
        matches!(net.observation().await, DiscoveryEvent::IHave(_)),
        "causal receive fence"
    );
    net.stop().await;
    assert_eq!(
        observer.attempts(),
        0,
        "startup, commands, requests, foreign settings and shutdown"
    );
    assert_eq!(
        peer.try_recv_from(&mut reply).unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn passive_runtime_discards_group_origins_before_valid_observation_marker() {
    let mut net = Passive::start(super::network::plan(None)).await;
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    for npdu in [vec![1, 0x80, 1, 0, 2], i_have("invalid origin", false)] {
        for origin in ["239.1.2.3:47808", "255.255.255.255:47809"] {
            peer.send_to(
                &wire(Bvll::ForwardedNpdu {
                    origin: origin.parse().unwrap(),
                    npdu: &npdu,
                }),
                net.address,
            )
            .await
            .unwrap();
        }
    }
    let marker = i_have("valid marker", false);
    peer.send_to(&wire(Bvll::OriginalUnicastNpdu(&marker)), net.address)
        .await
        .unwrap();
    assert!(
        matches!(net.observation().await, DiscoveryEvent::IHave(row) if row.name == "valid marker")
    );
    assert_eq!(net.app.overheard.0.len(), 1);
    assert_eq!(net.app.names.len(), 1);
    assert!(net.app.network.routers.is_empty());
    net.stop().await;
}
