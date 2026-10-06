//! A BBMD forwards its own broadcasts to BDT peers and foreign devices (#937).

use super::*;
use crate::port_ownership::{restart, ATTEMPTS};
use tokio::time::timeout;

/// A global-broadcast Who-Is NPDU.
pub(super) const NPDU: &[u8] = &[0x01, 0x20, 0xFF, 0xFF, 0x00, 0xFF, 0x10, 0x08];
const LOCALHOST: [u8; 4] = [127, 0, 0, 1];

pub(super) async fn udp() -> UdpSocket {
    UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap()
}

pub(super) fn port_of(socket: &UdpSocket) -> u16 {
    socket.local_addr().unwrap().port()
}

pub(super) async fn recv_bvll(socket: &UdpSocket) -> BvllMessage {
    let mut recv_buf = [0u8; 2048];
    let (len, _addr) = timeout(Duration::from_secs(2), socket.recv_from(&mut recv_buf))
        .await
        .expect("timed out waiting for BVLL frame")
        .unwrap();
    decode_bvll(&recv_buf[..len]).unwrap()
}

pub(super) async fn assert_no_bvll(socket: &UdpSocket, label: &str) {
    let mut recv_buf = [0u8; 2048];
    assert!(
        timeout(Duration::from_millis(100), socket.recv_from(&mut recv_buf))
            .await
            .is_err(),
        "{label} received an unexpected extra BVLL frame"
    );
}

fn assert_local_broadcast(frame: &BvllMessage) {
    assert_eq!(frame.function, BvlcFunction::ORIGINAL_BROADCAST_NPDU);
    assert_eq!(frame.payload.as_ref(), NPDU);
}

pub(super) fn assert_own_forwarded(frame: &BvllMessage, origin: ([u8; 4], u16), label: &str) {
    assert_eq!(frame.function, BvlcFunction::FORWARDED_NPDU, "{label}");
    assert_eq!(frame.originating_ip, Some(origin.0), "{label}");
    assert_eq!(frame.originating_port, Some(origin.1), "{label}");
    assert_eq!(frame.payload.as_ref(), NPDU, "{label}");
}

fn bdt_entry(ip: [u8; 4], port: u16, broadcast_mask: [u8; 4]) -> BdtEntry {
    BdtEntry {
        ip,
        port,
        broadcast_mask,
    }
}

/// BBMD state at `127.0.0.1:bbmd_port` with `bdt` and the given registered
/// foreign devices.
fn bbmd_state(bbmd_port: u16, bdt: Vec<BdtEntry>, foreign: &[([u8; 4], u16)]) -> BbmdState {
    let mut state = BbmdState::new(LOCALHOST, bbmd_port);
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    state.set_bdt(bdt).unwrap();
    for &(ip, port) in foreign {
        assert_eq!(
            state.register_foreign_device(ip, port, 60),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );
    }
    state
}

/// The BBMD's socket and its own B/IP address.
async fn bbmd_socket() -> (Arc<BipSocket>, ([u8; 4], u16)) {
    let socket = udp().await;
    let origin = (LOCALHOST, port_of(&socket));
    (Arc::new(BipSocket::new(socket, None)), origin)
}

/// A BBMD transport wired by hand around `socket`, so its local broadcast
/// reaches `127.0.0.1:broadcast_port` rather than its own socket. The returned
/// receiver is its fanout queue; [`spawn_worker`] sends what it holds.
fn hand_wired_bbmd(
    socket: &Arc<BipSocket>,
    state: BbmdState,
    broadcast_port: u16,
    policy: FanoutPolicy,
) -> (BipTransport, mpsc::Receiver<fanout::FanoutJob>) {
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, broadcast_port, Ipv4Addr::LOCALHOST);
    transport.set_fanout_policy(policy);
    let (tx, rx) = mpsc::channel(transport.fanout_policy.queue_capacity);
    let dispatcher = fanout::FanoutDispatcher::new(
        tx,
        Arc::clone(&transport.fanout_limiter),
        Arc::clone(&transport.fanout_counters),
    );
    let bbmd = Arc::new(std::sync::Mutex::new(state));
    transport.own_broadcast = Some(OwnBroadcastForwarder::new(Arc::clone(&bbmd), dispatcher));
    transport.bbmd = Some(bbmd);
    transport.socket = Some(Arc::clone(socket));
    (transport, rx)
}

fn spawn_worker(transport: &mut BipTransport, rx: mpsc::Receiver<fanout::FanoutJob>) {
    let socket = Arc::clone(transport.socket.as_ref().unwrap());
    let counters = Arc::clone(&transport.fanout_counters);
    transport.fanout_task = Some(tokio::spawn(fanout::run_fanout_worker(
        socket, rx, counters,
    )));
}

#[tokio::test]
async fn bbmd_own_broadcast_reaches_local_subnet_bdt_peer_and_foreign_device() {
    let (socket, origin) = bbmd_socket().await;
    let local_subnet = udp().await;
    let bdt_peer = udp().await;
    let foreign = udp().await;
    let state = bbmd_state(
        origin.1,
        vec![bdt_entry(LOCALHOST, port_of(&bdt_peer), [255; 4])],
        &[(LOCALHOST, port_of(&foreign))],
    );
    let (mut bbmd, rx) = hand_wired_bbmd(
        &socket,
        state,
        port_of(&local_subnet),
        FanoutPolicy::default(),
    );
    spawn_worker(&mut bbmd, rx);

    bbmd.send_broadcast(NPDU).await.unwrap();

    assert_local_broadcast(&recv_bvll(&local_subnet).await);
    assert_own_forwarded(&recv_bvll(&bdt_peer).await, origin, "two-hop BDT peer");
    assert_own_forwarded(&recv_bvll(&foreign).await, origin, "foreign device");
    // One frame each, no Forwarded-NPDU copy on the local subnet, and nothing
    // to the BBMD's own BDT entry.
    assert_no_bvll(&local_subnet, "local subnet").await;
    assert_no_bvll(&bdt_peer, "two-hop BDT peer").await;
    assert_no_bvll(&foreign, "foreign device").await;
    assert_no_bvll(&socket, "the BBMD's own BDT entry").await;

    let frame_len = (10 + NPDU.len()) as u64;
    assert_eq!(
        bbmd.fanout_counters(),
        FanoutCounters {
            packets_forwarded: 2,
            bytes_forwarded: 2 * frame_len,
            ..FanoutCounters::default()
        }
    );
}

#[tokio::test]
async fn bbmd_own_broadcast_targets_follow_bdt_masks_and_skip_own_entry() {
    let (socket, origin) = bbmd_socket().await;
    let local_subnet = udp().await;
    let state = bbmd_state(
        origin.1,
        vec![
            // Its own row, listed explicitly.
            bdt_entry(origin.0, origin.1, [255; 4]),
            // One-hop peer: the directed broadcast of its subnet.
            bdt_entry([198, 51, 100, 20], 47808, [255, 255, 255, 0]),
            // Two-hop peer: unicast to the peer BBMD.
            bdt_entry([203, 0, 113, 30], 47809, [255; 4]),
        ],
        &[([192, 0, 2, 50], 47810)],
    );
    // No worker: the queued job is inspected instead of sent.
    let (bbmd, mut rx) = hand_wired_bbmd(
        &socket,
        state,
        port_of(&local_subnet),
        FanoutPolicy::default(),
    );

    bbmd.send_broadcast(NPDU).await.unwrap();

    assert_local_broadcast(&recv_bvll(&local_subnet).await);
    let job = rx.try_recv().expect("own broadcast queued a fanout job");
    assert_own_forwarded(&decode_bvll(&job.frame).unwrap(), origin, "fanout frame");
    assert_eq!(
        job.targets,
        vec![
            SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 255), 47808),
            SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 30), 47809),
            SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 50), 47810),
        ]
    );
    assert!(rx.try_recv().is_err(), "exactly one fanout job");
}

#[tokio::test]
async fn bbmd_own_broadcast_skips_an_expired_foreign_device() {
    let (socket, origin) = bbmd_socket().await;
    let local_subnet = udp().await;
    let live = ([192, 0, 2, 50], 47810);
    let expired = ([192, 0, 2, 51], 47811);
    let mut state = bbmd_state(origin.1, Vec::new(), &[live, expired]);
    // Registered with a 60 s TTL: past it and the 30 s grace period.
    state.backdate_foreign_device_for_test(expired.0, expired.1, Duration::from_secs(120));
    // No worker: the queued job is inspected instead of sent.
    let (bbmd, mut rx) = hand_wired_bbmd(
        &socket,
        state,
        port_of(&local_subnet),
        FanoutPolicy::default(),
    );

    bbmd.send_broadcast(NPDU).await.unwrap();

    assert_local_broadcast(&recv_bvll(&local_subnet).await);
    let job = rx.try_recv().expect("own broadcast queued a fanout job");
    assert_own_forwarded(&decode_bvll(&job.frame).unwrap(), origin, "fanout frame");
    assert_eq!(
        job.targets,
        vec![SocketAddrV4::new(Ipv4Addr::from(live.0), live.1)],
        "only the live foreign device is a target"
    );
    assert!(rx.try_recv().is_err(), "exactly one fanout job");
    let state = bbmd.bbmd_state().unwrap().lock().unwrap();
    assert_eq!(state.fdt_len_for_test(), 1, "the expired entry is purged");
}

#[tokio::test]
async fn bbmd_own_broadcast_fanout_budgets_never_block_the_local_broadcast() {
    let (socket, origin) = bbmd_socket().await;
    let local_subnet = udp().await;
    let bdt_peer = udp().await;
    let foreign = udp().await;
    let state = bbmd_state(
        origin.1,
        vec![bdt_entry(LOCALHOST, port_of(&bdt_peer), [255; 4])],
        &[(LOCALHOST, port_of(&foreign))],
    );
    // One target per broadcast, and one packet per second from this origin.
    let policy = FanoutPolicy {
        max_fanout_per_input: 1,
        max_packets_per_sec_per_origin: 1,
        ..FanoutPolicy::default()
    };
    let (mut bbmd, rx) = hand_wired_bbmd(&socket, state, port_of(&local_subnet), policy);
    spawn_worker(&mut bbmd, rx);

    // The per-input cap forwards to the BDT peer (resolved first) only.
    bbmd.send_broadcast(NPDU).await.unwrap();
    assert_local_broadcast(&recv_bvll(&local_subnet).await);
    assert_own_forwarded(&recv_bvll(&bdt_peer).await, origin, "BDT peer");

    // The origin budget is spent: nothing is forwarded, the local broadcast
    // still goes out and the send still succeeds.
    bbmd.send_broadcast(NPDU).await.unwrap();
    assert_local_broadcast(&recv_bvll(&local_subnet).await);
    assert_no_bvll(&bdt_peer, "throttled BDT peer").await;
    assert_no_bvll(&foreign, "throttled foreign device").await;

    let counters = bbmd.fanout_counters();
    assert_eq!(counters.packets_forwarded, 1);
    assert_eq!(counters.packets_throttled, 3);
}

#[tokio::test]
async fn bbmd_own_broadcast_queue_overflow_never_fails_the_local_broadcast() {
    let (socket, origin) = bbmd_socket().await;
    let local_subnet = udp().await;
    let state = bbmd_state(
        origin.1,
        vec![bdt_entry([203, 0, 113, 30], 47809, [255; 4])],
        &[],
    );
    let policy = FanoutPolicy {
        queue_capacity: 1,
        ..FanoutPolicy::default()
    };
    // Nothing drains the queue, so the second fanout job overflows it.
    let (bbmd, _queue) = hand_wired_bbmd(&socket, state, port_of(&local_subnet), policy);

    for _ in 0..2 {
        bbmd.send_broadcast(NPDU).await.unwrap();
        assert_local_broadcast(&recv_bvll(&local_subnet).await);
    }

    let counters = bbmd.fanout_counters();
    assert_eq!(counters.queue_overflow_drops, 1);
    assert_eq!(counters.packets_forwarded, 0);
}

#[tokio::test]
async fn started_bbmd_forwards_its_own_broadcasts_across_restart() {
    // Each run starts on a fresh port; see `restart` for why it can lose it.
    'run: for attempt in 1..=ATTEMPTS {
        let bdt_peer = udp().await;
        let foreign = udp().await;
        let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
        bbmd.enable_bbmd(vec![bdt_entry(LOCALHOST, port_of(&bdt_peer), [255; 4])]);
        bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
        let mut rx = bbmd.start().await.unwrap();
        let origin = decode_bip_mac(bbmd.local_mac()).unwrap();

        let mut register = BytesMut::new();
        encode_bvll(
            &mut register,
            BvlcFunction::REGISTER_FOREIGN_DEVICE,
            &60u16.to_be_bytes(),
        )
        .unwrap();
        foreign
            .send_to(
                &register,
                SocketAddrV4::new(Ipv4Addr::from(origin.0), origin.1),
            )
            .await
            .unwrap();
        assert_eq!(
            decode_bvlc_result_code(&recv_bvll(&foreign).await).unwrap(),
            BvlcResultCode::SUCCESSFUL_COMPLETION
        );

        for round in ["first start", "restart"] {
            if round == "restart" {
                bbmd.stop().await.unwrap();
                assert!(bbmd.own_broadcast.is_none(), "stop drops the forwarder");
                let Some(started) = restart(&mut bbmd, attempt).await else {
                    continue 'run;
                };
                rx = started.unwrap();
            }
            bbmd.send_broadcast(NPDU).await.unwrap();
            assert_own_forwarded(&recv_bvll(&bdt_peer).await, origin, round);
            assert_own_forwarded(&recv_bvll(&foreign).await, origin, round);
            // Its local copy reaches its own socket here and is dropped as an
            // echo.
            assert!(
                timeout(Duration::from_millis(100), rx.recv())
                    .await
                    .is_err(),
                "{round}: own broadcast echo must not be delivered"
            );
            // Nor forwarded again as if another device had sent it.
            assert_no_bvll(&bdt_peer, round).await;
            assert_no_bvll(&foreign, round).await;
        }
        bbmd.stop().await.unwrap();
        return;
    }
}

#[tokio::test]
async fn plain_and_foreign_device_modes_do_not_forward_own_broadcasts() {
    let mut plain = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    let _plain_rx = plain.start().await.unwrap();
    assert!(plain.own_broadcast.is_none());
    plain.stop().await.unwrap();

    let bbmd = udp().await;
    let mut foreign = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    foreign.register_as_foreign_device(ForeignDeviceConfig {
        bbmd_ip: Ipv4Addr::LOCALHOST,
        bbmd_port: port_of(&bbmd),
        ttl: 60,
    });
    let _foreign_rx = foreign.start().await.unwrap();
    assert!(foreign.own_broadcast.is_none());
    assert_eq!(
        recv_bvll(&bbmd).await.function,
        BvlcFunction::REGISTER_FOREIGN_DEVICE
    );
    // A foreign device still hands its broadcast to its BBMD only.
    foreign.send_broadcast(NPDU).await.unwrap();
    let frame = recv_bvll(&bbmd).await;
    assert_eq!(
        frame.function,
        BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK
    );
    assert_eq!(frame.payload.as_ref(), NPDU);
    assert_eq!(foreign.fanout_counters(), FanoutCounters::default());
    foreign.stop().await.unwrap();
}
