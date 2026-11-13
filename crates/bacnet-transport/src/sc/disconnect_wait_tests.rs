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
/// Disconnect-ACK too, and closes as soon as it comes.
#[tokio::test(start_paused = true)]
async fn leaving_the_failover_hub_waits_for_the_disconnect_ack() {
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
    let asked = Instant::now();
    tokio::time::sleep(ANSWER_AFTER).await;
    assert_eq!(failover_probe.closes(), [], "closed before the ACK");
    send_ack(&failover_probe.hub, request.message_id).await;
    eventually("the failover close", || !failover_probe.closes().is_empty()).await;
    assert!(asked.elapsed() < CLOSE_GRACE, "{:?}", asked.elapsed());
    assert_eq!(failover_probe.closes(), [Some(1000)]);
    transport.stop().await.unwrap();
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
