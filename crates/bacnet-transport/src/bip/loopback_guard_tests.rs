//! A loopback interface on the wildcard socket refuses the local leg to
//! 255.255.255.255, counted, in every bind mode that has that shape; a
//! BBMD's unicast legs still go (#2691). No test here puts a broadcast on
//! the host's network: the BBMD tests' sockets record instead of sending.

use super::super::own_broadcast_tests::{NPDU, assert_own_forwarded};
use super::super::send_sink::SendSink;
use super::super::socket::BindPlan;
use super::super::socket_tests::plan;
use super::super::*;
use super::{LoopbackBroadcastRefused, warn_at_start};
use crate::bvll::forwarded_message;
use bytes::Bytes;
use std::net::SocketAddr;

const OWN: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 47808);
const LIMITED: SocketAddrV4 = SocketAddrV4::new(Ipv4Addr::BROADCAST, 47808);

async fn recording() -> (Arc<BipSocket>, SendSink) {
    let sink = SendSink::default();
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    (Arc::new(BipSocket::recording(socket, sink.clone())), sink)
}

#[test]
fn the_guard_follows_the_bind_mode() {
    let address = Ipv4Addr::new(192, 0, 2, 10);
    for (interface, port, by_address, guarded) in [
        // An explicit loopback interface on the wildcard socket, with an
        // ephemeral or a requested port.
        (Ipv4Addr::LOCALHOST, 0, false, true),
        (Ipv4Addr::LOCALHOST, 47808, false, true),
        (Ipv4Addr::new(127, 0, 0, 2), 47808, false, true),
        // Per-address binding puts the socket on the loopback address.
        (Ipv4Addr::LOCALHOST, 47808, true, false),
        // A wildcard interface or another explicit interface.
        (Ipv4Addr::UNSPECIFIED, 47808, false, false),
        (address, 47808, false, false),
        (address, 47808, true, false),
    ] {
        let plan = plan(interface, port, by_address);
        assert_eq!(
            plan.loopback_on_wildcard(),
            guarded,
            "{interface}:{port} by address {by_address}"
        );
    }
}

#[test]
fn the_start_warning_names_only_a_refusing_configuration() {
    let limited = |interface, by_address| BindPlan {
        broadcast: Ipv4Addr::BROADCAST,
        ..plan(interface, 0, by_address)
    };
    // A normal node or a BBMD.
    assert!(warn_at_start(
        &limited(Ipv4Addr::LOCALHOST, false),
        false,
        false
    ));
    // Each of these never sends a local broadcast to 255.255.255.255.
    assert!(!warn_at_start(
        &limited(Ipv4Addr::LOCALHOST, false),
        false,
        true
    ));
    assert!(!warn_at_start(
        &limited(Ipv4Addr::LOCALHOST, false),
        true,
        false
    ));
    assert!(!warn_at_start(
        &limited(Ipv4Addr::LOCALHOST, true),
        false,
        false
    ));
    assert!(!warn_at_start(
        &limited(Ipv4Addr::UNSPECIFIED, false),
        false,
        false
    ));
    let loopback_subnet = plan(Ipv4Addr::LOCALHOST, 0, false);
    assert!(!warn_at_start(&loopback_subnet, false, false));
}

/// A start computes the guard from its bind plan: a loopback interface on
/// an ephemeral port is guarded, refuses a broadcast, counts it and still
/// sends unicast; a wildcard interface is not guarded.
#[tokio::test]
async fn a_started_transport_guards_by_its_bind_plan() {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let _rx = transport.start().await.unwrap();
    assert!(transport.node.as_ref().unwrap().loopback_on_wildcard);
    let error = transport.send_broadcast(NPDU).await.unwrap_err();
    assert!(
        LoopbackBroadcastRefused::from_error(&error).is_some(),
        "{error}"
    );
    assert!(error.to_string().contains("127.255.255.255"), "{error}");
    assert!(!error.to_string().contains("broadcast_address"), "{error}");
    assert_eq!(transport.send_counters().loopback_limited_broadcast, 1);
    let peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let SocketAddr::V4(peer_addr) = peer.local_addr().unwrap() else {
        unreachable!()
    };
    let mac = encode_bip_mac(peer_addr.ip().octets(), peer_addr.port());
    transport.send_unicast(NPDU, &mac).await.unwrap();
    transport.stop().await.unwrap();

    let mut wildcard = BipTransport::new(Ipv4Addr::UNSPECIFIED, 0, Ipv4Addr::BROADCAST);
    let _rx = wildcard.start().await.unwrap();
    assert!(!wildcard.node.as_ref().unwrap().loopback_on_wildcard);
    wildcard.stop().await.unwrap();
}

/// A loopback BBMD's own broadcast still goes to its BDT peers and foreign
/// devices: only the local leg is refused, and the send reports that.
#[tokio::test]
async fn a_loopback_bbmd_forwards_its_own_broadcast_and_refuses_the_local_leg() {
    let (socket, sink) = recording().await;
    let peer = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 47808);
    let foreign = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 3), 47809);
    let mut state = BbmdState::new(OWN.ip().octets(), OWN.port());
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state
        .set_bdt(vec![BdtEntry {
            ip: peer.ip().octets(),
            port: peer.port(),
            broadcast_mask: [255; 4],
        }])
        .unwrap();
    let code = state.register_foreign_device(foreign.ip().octets(), foreign.port(), 60);
    assert_eq!(code, BvlcResultCode::SUCCESSFUL_COMPLETION);

    // Wired as start would wire 127.0.0.1:47808 with 255.255.255.255.
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, OWN.port(), Ipv4Addr::BROADCAST);
    let (tx, mut rx) = mpsc::channel(bbmd.fanout_policy.queue_capacity);
    let dispatcher = fanout::FanoutDispatcher::new(
        tx,
        Arc::clone(&bbmd.fanout_limiter),
        Arc::clone(&bbmd.fanout_counters),
    );
    let state = Arc::new(std::sync::Mutex::new(state));
    bbmd.own_broadcast = Some(OwnBroadcastForwarder::new(Arc::clone(&state), dispatcher));
    bbmd.bbmd = Some(state);
    bbmd.wire_for_test(Arc::clone(&socket));

    let error = bbmd.send_broadcast(NPDU).await.unwrap_err();
    assert!(
        LoopbackBroadcastRefused::from_error(&error).is_some(),
        "{error}"
    );
    let job = rx.try_recv().expect("the fanout is queued");
    let frame = decode_bvll(&job.frame).unwrap();
    // A Forwarded-NPDU from the BBMD's own address.
    assert_own_forwarded(&frame, (OWN.ip().octets(), OWN.port()), "fanout");
    assert_eq!(job.targets, vec![peer, foreign]);
    assert!(sink.take().is_empty(), "no local broadcast went out");
    assert_eq!(bbmd.send_counters().loopback_limited_broadcast, 1);
}

/// A loopback BBMD broadcasting to 255.255.255.255, as a receive context.
fn guarded_bbmd(socket: Arc<BipSocket>, state: BbmdState) -> RecvContext {
    let (npdu_tx, _) = mpsc::channel(4);
    let mut ctx = RecvContext::for_test(
        socket,
        npdu_tx,
        OWN,
        LIMITED,
        Some(Arc::new(std::sync::Mutex::new(state))),
    );
    Arc::get_mut(&mut ctx.node).unwrap().loopback_on_wildcard = true;
    ctx
}

fn destinations(sink: &SendSink) -> Vec<SocketAddrV4> {
    sink.take().into_iter().map(|sent| sent.to).collect()
}

/// A peer's Forwarded-NPDU that came by unicast is rebroadcast locally
/// (Annex J.4.5); on a loopback BBMD that leg is refused and counted, and
/// the foreign devices still get theirs.
#[tokio::test]
async fn a_loopback_bbmd_refuses_to_rebroadcast_a_peers_forward() {
    let (socket, sink) = recording().await;
    let peer = ([127, 0, 0, 2], 47808);
    let foreign = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 3), 47809);
    let mut state = BbmdState::new(OWN.ip().octets(), OWN.port());
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let row = |(ip, port)| BdtEntry {
        ip,
        port,
        broadcast_mask: [255; 4],
    };
    let own = (OWN.ip().octets(), OWN.port());
    state.set_bdt(vec![row(peer), row(own)]).unwrap();
    let code = state.register_foreign_device(foreign.ip().octets(), foreign.port(), 60);
    assert_eq!(code, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let ctx = guarded_bbmd(socket, state);
    let msg = forwarded_message(([192, 0, 2, 20], 47808), &[0x01, 0x00, 0xAA]);

    handle_bvll_message(&msg, peer, Delivery::Unicast, &ctx).await;

    assert_eq!(destinations(&sink), vec![foreign]);
    assert_eq!(ctx.node.send_counters().loopback_limited_broadcast, 1);
}

/// A registered foreign device's Distribute-Broadcast-To-Network reaches
/// the BDT peers; the refused local leg counts as a forwarding that failed,
/// so the foreign device gets a NAK (Annex J.4.5).
#[tokio::test]
async fn a_loopback_bbmd_naks_a_distribute_whose_local_leg_it_refuses() {
    let (socket, sink) = recording().await;
    let peer = SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 2), 47808);
    let foreign = ([127, 0, 0, 3], 47809);
    let mut state = BbmdState::new(OWN.ip().octets(), OWN.port());
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state
        .set_bdt(vec![BdtEntry {
            ip: peer.ip().octets(),
            port: peer.port(),
            broadcast_mask: [255; 4],
        }])
        .unwrap();
    let code = state.register_foreign_device(foreign.0, foreign.1, 60);
    assert_eq!(code, BvlcResultCode::SUCCESSFUL_COMPLETION);
    let ctx = guarded_bbmd(socket, state);
    let msg = BvllMessage {
        function: BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        payload: Bytes::from_static(&[0x01, 0x20, 0xBA, 0xC0, 0xDB, 0x10]),
    };

    handle_bvll_message(&msg, foreign, Delivery::Unicast, &ctx).await;

    let sent = sink.take();
    let to: Vec<_> = sent.iter().map(|sent| sent.to).collect();
    let foreign = SocketAddrV4::new(Ipv4Addr::from(foreign.0), foreign.1);
    assert_eq!(to, vec![peer, foreign], "the forward, then the NAK");
    let nak = decode_bvll(&sent[1].bytes).unwrap();
    assert_eq!(nak.function, BvlcFunction::BVLC_RESULT);
    assert_eq!(
        decode_bvlc_result_code(&nak).unwrap(),
        BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
    );
    assert_eq!(ctx.node.send_counters().loopback_limited_broadcast, 1);
}

/// A caller that wraps the refusal in its own error, keeping it as the
/// source (bacnet-ops names the broadcast's destination), still has it
/// found, so a front end can name its own option.
#[test]
fn the_refusal_is_found_inside_a_wrapping_error() {
    use std::io;
    #[derive(Debug)]
    struct Wrapped(io::Error);
    impl std::fmt::Display for Wrapped {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}, sending to 255.255.255.255:47808", self.0)
        }
    }
    impl std::error::Error for Wrapped {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    let refused = io::Error::new(io::ErrorKind::InvalidInput, LoopbackBroadcastRefused);
    let wrapped = io::Error::new(io::ErrorKind::InvalidInput, Wrapped(refused));
    assert!(LoopbackBroadcastRefused::from_io_error(&wrapped).is_some());
    let error = Error::Transport(wrapped);
    assert!(LoopbackBroadcastRefused::from_error(&error).is_some());
    let other = Wrapped(io::Error::other("no route"));
    let other = io::Error::new(io::ErrorKind::InvalidInput, other);
    assert!(LoopbackBroadcastRefused::from_io_error(&other).is_none());
}
