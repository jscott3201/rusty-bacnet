//! The warnings on the B/IP forwarding side go out at most once per interval
//! (#1643): a full fan-out queue, a forwarded packet the fan-out worker
//! failed to send, and a Forwarded-NPDU the receive path failed to send. A
//! stopped fan-out worker logs at debug only. The counters that already
//! existed count as before.
//!
//! A send to the limited broadcast from a socket without SO_BROADCAST fails
//! on every OS, so these tests use it for a failing destination.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;

use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::time::{Duration, timeout};
use tracing::Level;

use super::fanout::{AtomicFanoutCounters, FanoutDispatcher, FanoutJob, FanoutRateLimiter};
use super::*;
use crate::bbmd::ForeignDevicePolicy;
use crate::log_capture::LogCapture;
use crate::warn_throttle::WARNING_INTERVAL;
use bacnet_bip_codec::{BvlcFunction, BvlcResultCode};

const NPDU: &[u8] = &[0x01, 0x20, 0xDE, 0xAD, 0xBE, 0xEF];

/// A destination every send to fails from a socket without SO_BROADCAST.
fn unreachable_target() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::BROADCAST, 0xBAC0)
}

/// A reachable destination: no socket listens there, but the send succeeds.
fn target() -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9)
}

async fn loopback_socket() -> Arc<BipSocket> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    Arc::new(BipSocket::new(socket, None))
}

/// A dispatcher whose queue holds `capacity` jobs, and the counters it
/// shares.
fn dispatcher(
    capacity: usize,
) -> (
    FanoutDispatcher,
    mpsc::Receiver<FanoutJob>,
    Arc<AtomicFanoutCounters>,
) {
    let (tx, rx) = mpsc::channel(capacity);
    let limiter = Arc::new(std::sync::Mutex::new(FanoutRateLimiter::new(
        FanoutPolicy::default(),
    )));
    let counters = Arc::new(AtomicFanoutCounters::default());
    let dispatcher = FanoutDispatcher::new(tx, limiter, Arc::clone(&counters));
    (dispatcher, rx, counters)
}

/// The level of each event whose message starts with `message`, with the
/// value of `field` for each.
fn logged(logs: &LogCapture, message: &str, field: &str) -> Vec<(Level, Option<String>)> {
    logs.matching(message)
        .into_iter()
        .map(|event| (event.level, event.fields.get(field).cloned()))
        .collect()
}

fn warned(count: &str) -> (Level, Option<String>) {
    (Level::WARN, Some(count.to_string()))
}

const DEBUG: (Level, Option<String>) = (Level::DEBUG, None);

/// A full queue still warns, since it means real local overload, but at most
/// once per interval, with the jobs dropped since the last warning.
#[tokio::test(start_paused = true)]
async fn a_full_fanout_queue_warns_at_most_once_per_interval() {
    let (logs, _guard) = LogCapture::start();
    let (dispatcher, _rx, counters) = dispatcher(1);
    let forward =
        || dispatcher.dispatch_forwarded_npdu([192, 0, 2, 9], 0xBAC0, NPDU, vec![target()], 0);
    assert!(forward(), "the first job fits");
    for _ in 0..3 {
        assert!(!forward());
    }
    tokio::time::advance(WARNING_INTERVAL).await;
    assert!(!forward());
    assert_eq!(
        logged(&logs, "BIP BBMD: fanout work queue full", "dropped"),
        [warned("1"), DEBUG, DEBUG, warned("3")]
    );
    let expected = FanoutCounters {
        queue_overflow_drops: 4,
        ..FanoutCounters::default()
    };
    assert_eq!(counters.snapshot(), expected);
}

/// The worker stops only with the transport, so a job that finds it gone
/// raced the stop: logged at debug, and not counted as an overflow.
#[tokio::test]
async fn a_stopped_fanout_worker_logs_at_debug() {
    let (logs, _guard) = LogCapture::start();
    let (dispatcher, rx, counters) = dispatcher(4);
    drop(rx);
    for _ in 0..2 {
        assert!(!dispatcher.dispatch_forwarded_npdu(
            [192, 0, 2, 9],
            0xBAC0,
            NPDU,
            vec![target()],
            0
        ));
    }
    let stopped = logs.levels("BIP BBMD: fanout worker stopped");
    assert_eq!(stopped, [Level::DEBUG, Level::DEBUG]);
    assert!(logs.warnings().is_empty(), "{:?}", logs.warnings());
    assert_eq!(counters.snapshot(), FanoutCounters::default());
}

/// The worker counts each failed send, as before, and warns at most once
/// per interval with the failures since the last warning.
#[tokio::test(start_paused = true)]
async fn failed_fanout_sends_warn_at_most_once_per_interval() {
    let (logs, _guard) = LogCapture::start();
    let socket = loopback_socket().await;
    let counters = Arc::new(AtomicFanoutCounters::default());
    let frame = Bytes::from_static(NPDU);
    for targets in [vec![unreachable_target(); 3], vec![unreachable_target()]] {
        let (tx, rx) = mpsc::channel(1);
        tx.send(FanoutJob {
            frame: frame.clone(),
            targets,
        })
        .await
        .unwrap();
        drop(tx);
        fanout::run_fanout_worker(Arc::clone(&socket), rx, Arc::clone(&counters)).await;
        tokio::time::advance(WARNING_INTERVAL).await;
    }
    assert_eq!(
        logged(&logs, "BIP BBMD: failed to send forwarded NPDU", "failed"),
        [warned("1"), DEBUG, DEBUG, warned("3")]
    );
    let expected = FanoutCounters {
        send_errors: 4,
        ..FanoutCounters::default()
    };
    assert_eq!(counters.snapshot(), expected);
}

/// A BBMD that can't send the local copy of a registered foreign device's
/// Distribute-Broadcast-To-Network, nor forward it to another foreign device
/// (without a fan-out worker), warns at most once per interval. It still
/// NAKs the request, as before.
#[tokio::test(start_paused = true)]
async fn failed_forwarded_npdu_sends_warn_at_most_once_per_interval() {
    let (logs, _guard) = LogCapture::start();
    let socket = loopback_socket().await;
    let local = socket.local_addr().unwrap();
    let foreign = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let foreign_port = foreign.local_addr().unwrap().port();
    let mut state = BbmdState::new(Ipv4Addr::LOCALHOST.octets(), local.port());
    state.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let unreachable = unreachable_target();
    for (ip, port) in [
        (Ipv4Addr::LOCALHOST, foreign_port),
        (*unreachable.ip(), unreachable.port()),
    ] {
        let registered = state.register_foreign_device(ip.octets(), port, 60);
        assert_eq!(registered, BvlcResultCode::SUCCESSFUL_COMPLETION);
    }
    let (npdu_tx, _npdu_rx) = mpsc::channel(8);
    let ctx = RecvContext {
        ..RecvContext::for_test(
            socket,
            npdu_tx,
            SocketAddrV4::new(Ipv4Addr::from(Ipv4Addr::LOCALHOST.octets()), local.port()),
            SocketAddrV4::new(Ipv4Addr::BROADCAST, 0xBAC0),
            Some(Arc::new(std::sync::Mutex::new(state))),
        )
    };
    let distribute = BvllMessage {
        function: BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK,
        payload: Bytes::from_static(NPDU),
    };
    let sender = (Ipv4Addr::LOCALHOST.octets(), foreign_port);
    for _ in 0..2 {
        handle_bvll_message(&distribute, sender, Delivery::Unicast, &ctx).await;
    }
    tokio::time::advance(WARNING_INTERVAL).await;
    handle_bvll_message(&distribute, sender, Delivery::Unicast, &ctx).await;
    // Each request fails twice: the local copy, then the other foreign
    // device.
    assert_eq!(
        logged(&logs, "B/IP: failed to send a Forwarded-NPDU", "failed"),
        [warned("1"), DEBUG, DEBUG, DEBUG, warned("4"), DEBUG]
    );

    // Real time from here, so a missing NAK fails fast instead of waiting
    // for the test runner's timeout.
    tokio::time::resume();
    let mut buf = [0u8; 64];
    for _ in 0..3 {
        let (len, _) = timeout(Duration::from_secs(2), foreign.recv_from(&mut buf))
            .await
            .expect("each request is NAKed")
            .unwrap();
        let reply = decode_bvll(&buf[..len]).unwrap();
        assert_eq!(
            decode_bvlc_result_code(&reply).unwrap(),
            BvlcResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK
        );
    }
    assert_eq!(ctx.drops.snapshot(), ReceiveCounters::default());
}
