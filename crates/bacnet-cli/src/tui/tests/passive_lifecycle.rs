//! Listener ownership across cancellation, a full UI queue and replacement.
use super::passive_discovery::{Passive, i_have, options, wire};
use super::*;
use crate::tui::app::Link;
use crate::tui::message::WorkerEvent;
use crate::tui::worker::{EventSink, coordinator, session};
use bacnet_transport::{bip::BipMulticast, bvll::Bvll};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::atomic::Ordering;
use tokio::{
    net::UdpSocket,
    sync::{mpsc, watch},
};

fn listening(event: WorkerEvent) -> SocketAddrV4 {
    match event {
        WorkerEvent::Session { event, .. } => listening(*event),
        WorkerEvent::PassiveListening { local, .. } => local.parse().unwrap(),
        event => panic!("expected ready listener, got {event:?}"),
    }
}
#[tokio::test]
async fn passive_full_queue_cancellation_closes_socket_and_counts_drops() {
    let (sink, mut events) = EventSink::channel(1);
    let dropped = sink.dropped_counter();
    let (cancel, cancelled) = watch::channel(false);
    let (commands, rx) = mpsc::channel(2);
    let task = session::spawn(
        super::network::plan(None),
        sink.for_session(7, cancelled),
        rx,
        options(),
    );
    let address = listening(events.recv().await.unwrap());
    // Safety::Ready now fills the UI queue; observations must drop, never block.
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    peer.send_to(
        &wire(Bvll::OriginalUnicastNpdu(&i_have("full", false))),
        address,
    )
    .await
    .unwrap();
    while dropped.load(Ordering::Relaxed) == 0 {
        tokio::task::yield_now().await;
    }
    cancel.send_replace(true);
    task.await.unwrap().unwrap();
    assert!(matches!(
        events.recv().await,
        Some(WorkerEvent::Session { generation: 7, .. })
    ));
    assert!(
        std::net::UdpSocket::bind(address).is_ok(),
        "immediate one-shot rebind"
    );
    drop(commands);
}
#[tokio::test]
async fn passive_ui_event_receiver_close_releases_unscoped_listener() {
    let net = Passive::start(super::network::plan(None)).await;
    let address = net.address;
    drop(net.events);
    net.task.await.unwrap().unwrap();
    assert!(std::net::UdpSocket::bind(address).is_ok());
    drop(net.commands);
}
/// Feed a session's three startup events to `app`. A failed start on the
/// port the test chose is judged by the listener's own bind error: a port
/// another socket held goes back to `rerun_on_lost_port`, and any other
/// failure panics (`lost_port`).
async fn started(app: &mut App, events: &mut mpsc::Receiver<WorkerEvent>) -> std::io::Result<()> {
    for _ in 0..3 {
        let event = events.recv().await.unwrap();
        if let Some(failure) = super::connect_failure(&event) {
            return Err(super::lost_port(failure));
        }
        worker(app, event);
    }
    Ok(())
}
/// The listener must come back on the same address, so the test picks the
/// port and frees it before the first bind; the session also frees it on
/// disconnect and the UI close. Another process can take it in any of those
/// gaps, so a lost port reruns the test on a fresh one (#2216): for the
/// listener's binds, as its own bind error says (#2776).
#[tokio::test]
async fn passive_coordinator_disconnect_reconnect_and_ui_close_retire_generations() {
    super::port_retry::rerun_on_lost_port(retire_generations).await;
}
async fn retire_generations() -> std::io::Result<()> {
    let reserved = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = reserved.local_addr().unwrap();
    let mut plan = super::network::plan(None);
    plan.args.port = address.port();
    drop(reserved);
    let (sink, mut events) = EventSink::channel(2);
    let filler = sink.clone();
    let dropped = sink.dropped_counter();
    let (commands, rx) = mpsc::channel(8);
    let task = coordinator::spawn(Some(plan), None, sink, rx, options());
    let mut app = new_app(None);
    app.safety.passive = true;
    started(&mut app, &mut events).await?;
    assert_eq!(
        app.passive_listener.as_deref(),
        Some(address.to_string().as_str())
    );
    let generation = app.generation;
    // Fill the UI queue without draining it, then prove ingress proceeds.
    assert_eq!(
        filler.offer(WorkerEvent::Passive),
        crate::tui::worker::Offer::Sent
    );
    assert_eq!(
        filler.offer(WorkerEvent::Passive),
        crate::tui::worker::Offer::Sent
    );
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    peer.send_to(
        &wire(Bvll::OriginalUnicastNpdu(&i_have("old", false))),
        address,
    )
    .await
    .unwrap();
    while dropped.load(Ordering::Relaxed) == 0 {
        tokio::task::yield_now().await;
    }
    commands.send(Command::Disconnect).await.unwrap();
    loop {
        let event = events.recv().await.unwrap();
        let done = matches!(event, WorkerEvent::Disconnected { .. });
        worker(&mut app, event);
        if done {
            break;
        }
    }
    assert_eq!(app.link, Link::Down);
    // Disconnect completed socket cleanup.
    drop(std::net::UdpSocket::bind(address)?);
    commands.send(Command::Reconnect).await.unwrap();
    started(&mut app, &mut events).await?;
    assert_eq!(app.generation, generation + 1);
    assert_eq!(
        app.passive_listener.as_deref(),
        Some(address.to_string().as_str())
    );
    worker(
        &mut app,
        WorkerEvent::Session {
            generation,
            event: Box::new(WorkerEvent::Discovery(
                crate::tui::network::discovery::DiscoveryEvent::Router {
                    router: "stale".into(),
                    networks: vec![99],
                },
            )),
        },
    );
    assert!(
        app.network.routers.is_empty(),
        "old generation cannot update replacement"
    );
    drop(events);
    drop(commands);
    task.await.unwrap();
    // UI closure joins the listener.
    std::net::UdpSocket::bind(address).map(drop)
}
/// A socket held the port when the listener bound it and let it go before
/// the test looked. The port was still lost, and the test reruns, as the
/// listener's own bind error says; a bind of the port afterwards would find
/// it free (#2776).
#[tokio::test]
async fn a_port_held_at_the_bind_and_freed_before_the_check_is_lost() {
    let holder = bacnet_transport::bip::PassiveBip::bind(Ipv4Addr::UNSPECIFIED, 0)
        .await
        .unwrap();
    let mut plan = super::network::plan(None);
    plan.args.port = holder.local_addr().unwrap().port();
    let (sink, mut events) = EventSink::channel(4);
    let (_commands, rx) = mpsc::channel(2);
    // The session reports its failed bind and ends before the holder lets go.
    session::spawn(plan, sink, rx, options())
        .await
        .unwrap()
        .unwrap();
    drop(holder);
    let mut app = new_app(None);
    app.safety.passive = true;
    let lost = started(&mut app, &mut events)
        .await
        .expect_err("a lost port");
    assert!(super::port_retry::lost_to_another_socket(&lost), "{lost}");
}
#[tokio::test]
async fn passive_bind_failure_and_unsupported_modes_never_claim_readiness() {
    let reserved = bacnet_transport::bip::PassiveBip::bind(Ipv4Addr::UNSPECIFIED, 0)
        .await
        .unwrap();
    let mut busy = super::network::plan(None);
    busy.args.port = reserved.local_addr().unwrap().port();
    let mut multicast = super::network::plan(None);
    multicast.args.multicast = Some(BipMulticast::new(Ipv4Addr::new(239, 1, 2, 3)).unwrap());
    let mut unsupported = Vec::new();
    for kind in [
        session::TransportKind::Sc,
        session::TransportKind::Bip6,
        session::TransportKind::Mstp,
    ] {
        let mut plan = super::network::plan(None);
        plan.kind = kind;
        unsupported.push(plan);
    }
    for plan in [busy, multicast].into_iter().chain(unsupported) {
        let (sink, mut events) = EventSink::channel(4);
        let (_commands, rx) = mpsc::channel(2);
        let task = session::spawn(plan, sink, rx, options());
        assert!(matches!(
            events.recv().await,
            Some(WorkerEvent::ConnectFailed { .. })
        ));
        task.await.unwrap().unwrap();
        assert!(events.recv().await.is_none());
    }
}
