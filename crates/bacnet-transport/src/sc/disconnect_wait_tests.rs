//! A node that sends a Disconnect-Request waits for its answer before it
//! closes, as the requester does in Annex AB.6.2.2, for at most the close
//! grace (#2520): `stop`, a primary restore leaving the failover hub, and a
//! retired outbound direct link. Tokio's paused clock and sockets that
//! record each close.
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use tokio::time::Instant;

use crate::port::TransportPort;
use crate::sc::close_test_support::{Probe, eventually, recorder};
use crate::sc::data_attribute_tests::hub_accept;
use crate::sc::{
    CLOSE_GRACE, ScDisconnectCause, ScEvent, ScReconnectConfig, ScTransport, WebSocketPort,
};
use crate::sc_frame::{ScFunction, ScMessage, decode_sc_message, encode_sc_message};

const NODE_VMAC: [u8; 6] = [0x01; 6];

/// How long each test's peer takes to answer the Disconnect-Request.
const ANSWER_AFTER: Duration = Duration::from_millis(500);

/// Read until the Disconnect-Request, past any heartbeat.
async fn disconnect_request(hub: &crate::sc::LoopbackWebSocket) -> ScMessage {
    loop {
        let data = tokio::time::timeout(Duration::from_secs(5), hub.recv())
            .await
            .expect("no Disconnect-Request")
            .unwrap();
        let message = decode_sc_message(&data).unwrap();
        if message.function == ScFunction::DisconnectRequest {
            return message;
        }
    }
}

async fn send_ack(hub: &crate::sc::LoopbackWebSocket, message_id: u16) {
    let ack = ScMessage {
        function: ScFunction::DisconnectAck,
        message_id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::new(),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &ack);
    hub.send(&buf).await.unwrap();
}

/// The cause of the one `Disconnected` event.
async fn disconnected(events: &mut tokio::sync::broadcast::Receiver<ScEvent>) -> ScDisconnectCause {
    loop {
        match events.recv().await.unwrap() {
            ScEvent::Disconnected { cause, .. } => return cause,
            _ => continue,
        }
    }
}

/// A connected transport over a recording socket.
async fn connected(
    probe_vmac: [u8; 6],
) -> (
    ScTransport<crate::sc::close_test_support::Recorder>,
    Probe,
    tokio::sync::broadcast::Receiver<ScEvent>,
) {
    let (socket, probe) = recorder();
    let mut transport = ScTransport::new(socket, NODE_VMAC).with_device_uuid([1; 16]);
    let events = transport.connection_events();
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&probe.hub, probe_vmac));
    let _rx = started.unwrap();
    (transport, probe, events)
}

/// `stop` closes only once the hub has answered, and reports its own
/// close, not anything the hub then does.
#[tokio::test(start_paused = true)]
async fn stop_waits_for_the_disconnect_ack_and_reports_a_local_close() {
    let (mut transport, probe, mut events) = connected([0x10; 6]).await;
    let hub = probe.hub;
    let closes = probe.closes;
    let answer = tokio::spawn(async move {
        let request = disconnect_request(&hub).await;
        tokio::time::sleep(ANSWER_AFTER).await;
        assert!(closes.lock().unwrap().is_empty(), "closed before the ACK");
        send_ack(&hub, request.message_id).await;
        (hub, closes)
    });
    let began = Instant::now();
    transport.stop().await.unwrap();
    let took = began.elapsed();
    assert!(took >= ANSWER_AFTER && took < CLOSE_GRACE, "{took:?}");
    let (_hub, closes) = answer.await.unwrap();
    assert_eq!(*closes.lock().unwrap(), [Some(1000)]);
    assert_eq!(
        disconnected(&mut events).await,
        ScDisconnectCause::LocalClose
    );
}

/// A hub that ends the connection instead of answering the
/// Disconnect-Request still leaves a local close as the cause.
#[tokio::test(start_paused = true)]
async fn a_hub_that_hangs_up_on_the_disconnect_request_reads_as_a_local_close() {
    let (mut transport, probe, mut events) = connected([0x10; 6]).await;
    let hub = probe.hub;
    let hang_up = tokio::spawn(async move {
        disconnect_request(&hub).await;
        drop(hub);
    });
    transport.stop().await.unwrap();
    hang_up.await.unwrap();
    assert_eq!(
        disconnected(&mut events).await,
        ScDisconnectCause::LocalClose
    );
}

/// Leaving the failover hub after a primary restore waits for the
/// Disconnect-ACK too, and closes as soon as it comes. A `stop` in that wait
/// lets it finish within its own budget rather than cutting it off.
#[tokio::test(start_paused = true)]
async fn leaving_the_failover_hub_waits_for_the_disconnect_ack() {
    for stop_early in [false, true] {
        let (primary, _silent_primary) = recorder();
        let (failover, failover_probe) = recorder();
        let (dialled_tx, mut dialled) = tokio::sync::mpsc::unbounded_channel::<Probe>();
        let mut transport = ScTransport::new(primary, NODE_VMAC)
            .with_device_uuid([1; 16])
            .with_connect_timeout_ms(100)
            .with_heartbeat_interval_ms(5_000)
            .with_reconnect(ScReconnectConfig {
                initial_delay_ms: 25,
                max_delay_ms: 25,
                max_retries: 1,
            })
            .with_connector(move || {
                let (socket, probe) = recorder();
                let _ = dialled_tx.send(probe);
                async { Ok(socket) }
            })
            .with_failover(failover);
        let (started, ()) = tokio::join!(
            transport.start(),
            hub_accept(&failover_probe.hub, [0x20; 6])
        );
        let _rx = started.unwrap();
        let restored = tokio::time::timeout(Duration::from_secs(5), dialled.recv())
            .await
            .unwrap()
            .unwrap();
        hub_accept(&restored.hub, [0x10; 6]).await;
        let request = disconnect_request(&failover_probe.hub).await;
        if stop_early {
            transport.stop().await.unwrap();
            assert_eq!(failover_probe.closes(), [Some(1000)], "stopped in the wait");
            continue;
        }
        let asked = Instant::now();
        tokio::time::sleep(ANSWER_AFTER).await;
        assert_eq!(failover_probe.closes(), [], "closed before the ACK");
        send_ack(&failover_probe.hub, request.message_id).await;
        eventually("the failover close", || !failover_probe.closes().is_empty()).await;
        assert!(asked.elapsed() < CLOSE_GRACE, "{:?}", asked.elapsed());
        assert_eq!(failover_probe.closes(), [Some(1000)]);
        transport.stop().await.unwrap();
    }
}

/// A retired outbound direct link waits for its Disconnect-ACK too.
#[tokio::test(start_paused = true)]
async fn a_retired_direct_link_waits_for_the_disconnect_ack() {
    use crate::sc::direct_membership::{DirectMembership, DirectRole};
    use crate::sc::direct_pool::PooledDirect;
    use crate::sc::direct_socket::DirectSocket;
    let (socket, probe) = recorder();
    let owner = Arc::new(DirectMembership::default());
    let member = owner
        .reserve([2; 16], [2; 6], [1; 16], [1; 6], DirectRole::Outbound, 16)
        .unwrap()
        .commit();
    let permits = Arc::new(tokio::sync::Semaphore::new(1));
    let (pooled, worker) = PooledDirect::start(
        DirectSocket::Custom(socket),
        member,
        permits.try_acquire_owned().unwrap(),
        #[cfg(feature = "sc-tls")]
        None,
        (1476, 1476),
        (30_000, 60_000),
    );
    pooled.member.retire();
    let request = disconnect_request(&probe.hub).await;
    let asked = Instant::now();
    tokio::time::sleep(ANSWER_AFTER).await;
    assert_eq!(probe.closes(), [], "closed before the ACK");
    send_ack(&probe.hub, request.message_id).await;
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert!(asked.elapsed() < CLOSE_GRACE, "{:?}", asked.elapsed());
    assert_eq!(probe.closes(), [Some(1000)]);
}

/// While a silent outbound direct peer holds `stop` up for the close grace,
/// a hub that answers the Disconnect-Request and then hangs up starts no
/// recovery (#2520): with a one-millisecond reconnect delay, recovery would
/// otherwise dial the hub again many times before `stop` ends.
#[tokio::test(start_paused = true)]
async fn a_stop_held_up_by_a_silent_direct_peer_never_redials_the_hub() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    const TARGET: [u8; 6] = [0x22; 6];
    let (socket, probe) = recorder();
    let dials = Arc::new(AtomicUsize::new(0));
    let redials = dials.clone();
    let peers: Arc<std::sync::Mutex<Vec<Probe>>> = Arc::default();
    let kept = peers.clone();
    let mut transport = ScTransport::new(socket, NODE_VMAC)
        .with_device_uuid([1; 16])
        .with_reconnect(ScReconnectConfig {
            initial_delay_ms: 1,
            max_delay_ms: 1,
            max_retries: 100,
        })
        .with_connector(move || {
            redials.fetch_add(1, Ordering::SeqCst);
            async { Ok(recorder().0) }
        })
        .with_direct_discovery(true)
        .with_custom_direct_dialer(move |_uri| {
            let (socket, peer) = recorder();
            let kept = kept.clone();
            // The peer accepts the Connect-Request, then never says a word.
            tokio::spawn(async move {
                let request = decode_sc_message(&peer.hub.recv().await.unwrap()).unwrap();
                let mut accept = TARGET.to_vec();
                accept.extend_from_slice(&[0x44; 16]);
                accept.extend_from_slice(&[0x05, 0xc4, 0x05, 0xc4]);
                send(
                    &peer.hub,
                    ScFunction::ConnectAccept,
                    request.message_id,
                    accept,
                )
                .await;
                kept.lock().unwrap().push(peer);
            });
            async move { Ok(socket) }
        });
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&probe.hub, [0x10; 6]));
    let _rx = started.unwrap();
    // The first send resolves TARGET through the hub and dials it.
    let (sent, ()) = tokio::join!(transport.send_unicast(&[1, 0, 0x30], &TARGET), async {
        let request = decode_sc_message(&probe.hub.recv().await.unwrap()).unwrap();
        assert_eq!(request.function, ScFunction::AddressResolution);
        let ack = ScMessage {
            function: ScFunction::AddressResolutionAck,
            message_id: request.message_id,
            originating_vmac: Some(TARGET),
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from_static(b"wss://peer.example/sc"),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &ack);
        probe.hub.send(&buf).await.unwrap();
    });
    sent.unwrap();
    assert_eq!(peers.lock().unwrap().len(), 1, "the direct link is up");
    let hub = probe.hub;
    let hang_up = tokio::spawn(async move {
        let request = disconnect_request(&hub).await;
        send_ack(&hub, request.message_id).await;
        drop(hub);
    });
    let began = Instant::now();
    transport.stop().await.unwrap();
    assert!(began.elapsed() >= CLOSE_GRACE, "{:?}", began.elapsed());
    hang_up.await.unwrap();
    assert_eq!(dials.load(Ordering::SeqCst), 0);
}

async fn send(hub: &crate::sc::LoopbackWebSocket, function: ScFunction, id: u16, payload: Vec<u8>) {
    let message = ScMessage {
        function,
        message_id: id,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from(payload),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &message);
    hub.send(&buf).await.unwrap();
}

/// While `stop` waits for its Disconnect-ACK, no Heartbeat-Request goes
/// out and no heartbeat timeout fires, even with an interval shorter than
/// the wait; the cause stays this node's own close.
#[tokio::test(start_paused = true)]
async fn no_heartbeat_runs_while_stop_waits_for_the_ack() {
    let (socket, probe) = recorder();
    let mut transport = ScTransport::new(socket, NODE_VMAC)
        .with_device_uuid([1; 16])
        .with_test_heartbeat_timing_ms(100, 300);
    let mut events = transport.connection_events();
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&probe.hub, [0x10; 6]));
    let _rx = started.unwrap();
    let (hub, closes) = (probe.hub, probe.closes);
    let answer = tokio::spawn(async move {
        let request = disconnect_request(&hub).await;
        let quiet = tokio::time::timeout(ANSWER_AFTER, hub.recv()).await;
        assert!(quiet.is_err(), "sent while waiting for the ACK: {quiet:?}");
        send_ack(&hub, request.message_id).await;
        hub
    });
    transport.stop().await.unwrap();
    let _hub = answer.await.unwrap();
    assert_eq!(
        disconnected(&mut events).await,
        ScDisconnectCause::LocalClose
    );
    assert_eq!(*closes.lock().unwrap(), [Some(1000)]);
}

/// A hub that refuses this node's Disconnect-Request ends the connection
/// all the same: a local close, logged at debug, not as a fatal Result.
#[tokio::test(start_paused = true)]
async fn a_refused_disconnect_request_is_not_a_fatal_result() {
    let (logs, _guard) = crate::log_capture::LogCapture::start();
    let (mut transport, probe, mut events) = connected([0x10; 6]).await;
    let hub = probe.hub;
    let refuse = tokio::spawn(async move {
        let request = disconnect_request(&hub).await;
        // A NAK to the Disconnect-Request: class RESOURCES, code OTHER.
        let nak = vec![0x08, 0x01, 0x00, 0x00, 0x03, 0x00, 0x00];
        send(&hub, ScFunction::Result, request.message_id, nak).await;
        hub
    });
    transport.stop().await.unwrap();
    let _hub = refuse.await.unwrap();
    assert_eq!(
        disconnected(&mut events).await,
        ScDisconnectCause::LocalClose
    );
    assert!(logs.matching("BACnet/SC fatal BVLC-Result").is_empty());
}
