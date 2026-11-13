//! Which close each hub socket gets when the transport stops using it, and
//! how long `stop` may take doing so (#2520). Tokio's paused clock and
//! in-memory sockets that record each close; `close_wire_tests` shows the
//! frames on real TLS sockets.
//!
//! The rule: a socket the transport gives up gets its close at once, 1000
//! for an orderly end and none for a peer that stopped answering. The one
//! socket it keeps is a lost hub's own when recovery will send the next
//! Connect-Request on it, which it does with reconnect on and no connector
//! for that hub; that socket gets no close.
use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::time::Instant;

use super::STOP_SEND_BUDGET;
use crate::port::TransportPort;
use crate::sc::data_attribute_tests::hub_accept;
use crate::sc::test_waits::until;
use crate::sc::{
    CLOSE_GRACE, LoopbackWebSocket, ScDisconnectCause, ScEvent, ScReconnectConfig, ScTransport,
    WebSocketPort,
};
use crate::sc_frame::{ScFunction, decode_sc_message};
use bacnet_types::error::Error;

type Closes = Arc<StdMutex<Vec<Option<u16>>>>;

/// A loopback socket that records the closes asked of it. Once `hang_send`
/// or `hang_close` is set, that operation never finishes, as against a hub
/// that stopped reading.
struct Recorder {
    inner: LoopbackWebSocket,
    closes: Closes,
    hang_send: Arc<AtomicBool>,
    hang_close: Arc<AtomicBool>,
}

/// The test's handles on a [`Recorder`].
struct Probe {
    hub: LoopbackWebSocket,
    closes: Closes,
    hang_send: Arc<AtomicBool>,
    hang_close: Arc<AtomicBool>,
}

impl Probe {
    fn closes(&self) -> Vec<Option<u16>> {
        self.closes.lock().unwrap().clone()
    }
}

fn recorder() -> (Recorder, Probe) {
    let (inner, hub) = LoopbackWebSocket::pair();
    let closes = Closes::default();
    let (hang_send, hang_close) = (Arc::default(), Arc::default());
    let probe = Probe {
        hub,
        closes: closes.clone(),
        hang_send: Arc::clone(&hang_send),
        hang_close: Arc::clone(&hang_close),
    };
    let socket = Recorder {
        inner,
        closes,
        hang_send,
        hang_close,
    };
    (socket, probe)
}

impl WebSocketPort for Recorder {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        if self.hang_send.load(Ordering::Acquire) {
            pending::<()>().await;
        }
        self.inner.send(data).await
    }

    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.inner.recv().await
    }

    async fn close(&self, status: Option<u16>) -> Result<(), Error> {
        self.closes.lock().unwrap().push(status);
        if self.hang_close.load(Ordering::Acquire) {
            pending::<()>().await;
        }
        Ok(())
    }
}

const NODE_VMAC: [u8; 6] = [0x01; 6];

fn reconnect() -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 25,
        max_delay_ms: 25,
        max_retries: 1,
    }
}

/// A started transport on `socket`, whose hub has accepted it.
async fn started(transport: &mut ScTransport<Recorder>, probe: &Probe) {
    let (started, ()) = tokio::join!(transport.start(), hub_accept(&probe.hub, [0x10; 6]));
    let _rx = started.unwrap();
}

/// `stop` spends at most its send budget against a hub that reads nothing,
/// and at most the close grace when only the close stalls. Either way it
/// asks for the close with 1000.
#[tokio::test(start_paused = true)]
async fn stop_is_bounded_against_a_hub_that_stopped_reading() {
    for (hang_send, bound) in [(true, STOP_SEND_BUDGET), (false, CLOSE_GRACE)] {
        let (socket, probe) = recorder();
        let mut transport = ScTransport::new(socket, NODE_VMAC).with_device_uuid([1; 16]);
        started(&mut transport, &probe).await;
        probe.hang_send.store(hang_send, Ordering::Release);
        probe.hang_close.store(true, Ordering::Release);
        let began = Instant::now();
        tokio::time::timeout(Duration::from_secs(60), transport.stop())
            .await
            .expect("stop must be bounded")
            .unwrap();
        let took = began.elapsed();
        assert!(
            took >= bound && took < bound + Duration::from_millis(5),
            "hang_send={hang_send}: {took:?}"
        );
        assert_eq!(probe.closes(), [Some(1000)], "hang_send={hang_send}");
    }
}

/// A heartbeat the hub leaves unanswered ends the connection. The socket
/// is given up, and closed with no status, unless recovery will send its
/// next Connect-Request on it.
#[tokio::test(start_paused = true)]
async fn a_heartbeat_timeout_closes_the_socket_unless_recovery_reuses_it() {
    // (reconnect, connector, closes on the lost socket)
    for (with_reconnect, with_connector, expected) in [
        (false, false, vec![None]),
        (true, true, vec![None]),
        (true, false, vec![]),
    ] {
        let (socket, probe) = recorder();
        let mut transport = ScTransport::new(socket, NODE_VMAC)
            .with_device_uuid([1; 16])
            .with_connect_timeout_ms(100)
            .with_test_heartbeat_timing_ms(100, 300);
        if with_reconnect {
            transport = transport.with_reconnect(ScReconnectConfig {
                max_retries: 2,
                ..reconnect()
            });
        }
        if with_connector {
            // Each redial reaches a hub that is already gone.
            transport = transport.with_connector(|| async { Ok(recorder().0) });
        }
        let mut events = transport.connection_events();
        started(&mut transport, &probe).await;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
                .await
                .expect("no heartbeat timeout")
                .unwrap();
            if let ScEvent::Disconnected { cause, .. } = event {
                assert_eq!(cause, ScDisconnectCause::HeartbeatTimeout);
                break;
            }
        }
        if expected.is_empty() {
            // Kept: each retry's Connect-Request goes out on this socket,
            // and one that goes unanswered does not close it either.
            let mut retries = 0;
            while retries < 2 {
                let data = tokio::time::timeout(Duration::from_secs(5), probe.hub.recv())
                    .await
                    .expect("no Connect-Request on the kept socket")
                    .unwrap();
                if decode_sc_message(&data).unwrap().function == ScFunction::ConnectRequest {
                    retries += 1;
                }
            }
        } else {
            until("the lost socket's close", || async {
                !probe.closes.lock().unwrap().is_empty()
            })
            .await;
        }
        assert_eq!(
            probe.closes(),
            expected,
            "reconnect={with_reconnect} connector={with_connector}"
        );
        transport.stop().await.unwrap();
    }
}

/// A primary that never answers the first Connect-Request, with the node
/// starting on the failover hub: closed with no status when a primary
/// restore would dial a fresh socket, kept open when it would retry this
/// one. Once a restore succeeds, the failover hub gets its
/// Disconnect-Request and a close with 1000.
#[tokio::test(start_paused = true)]
async fn the_first_connect_and_a_primary_restore_close_the_sockets_they_give_up() {
    for with_connector in [true, false] {
        let (primary, primary_probe) = recorder();
        let (failover, failover_probe) = recorder();
        let (dialled_tx, mut dialled) = tokio::sync::mpsc::unbounded_channel();
        let mut transport = ScTransport::new(primary, NODE_VMAC)
            .with_device_uuid([1; 16])
            .with_connect_timeout_ms(100)
            .with_heartbeat_interval_ms(5_000)
            .with_reconnect(reconnect())
            .with_failover(failover);
        if with_connector {
            transport = transport.with_connector(move || {
                let (socket, probe) = recorder();
                let _ = dialled_tx.send(probe);
                async { Ok(socket) }
            });
        }
        let (started, ()) = tokio::join!(
            transport.start(),
            hub_accept(&failover_probe.hub, [0x20; 6])
        );
        let _rx = started.unwrap();
        let primary_closes: &[Option<u16>] = if with_connector { &[None] } else { &[] };
        assert_eq!(
            primary_probe.closes(),
            primary_closes,
            "connector={with_connector}"
        );
        if with_connector {
            let restored = tokio::time::timeout(Duration::from_secs(5), dialled.recv())
                .await
                .expect("no primary restore dial")
                .unwrap();
            hub_accept(&restored.hub, [0x10; 6]).await;
            let leave = tokio::time::timeout(Duration::from_secs(5), failover_probe.hub.recv())
                .await
                .expect("no Disconnect-Request to the failover hub")
                .unwrap();
            assert_eq!(
                decode_sc_message(&leave).unwrap().function,
                ScFunction::DisconnectRequest
            );
            until("the failover socket's close", || async {
                !failover_probe.closes.lock().unwrap().is_empty()
            })
            .await;
            assert_eq!(failover_probe.closes(), [Some(1000)]);
        } else {
            // A restore tries the kept primary again, and leaves it open
            // when that goes unanswered too.
            for _ in 0..2 {
                let data = tokio::time::timeout(Duration::from_secs(5), primary_probe.hub.recv())
                    .await
                    .expect("no Connect-Request on the kept primary")
                    .unwrap();
                let request = decode_sc_message(&data).unwrap();
                assert_eq!(request.function, ScFunction::ConnectRequest);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            assert_eq!(primary_probe.closes(), []);
        }
        transport.stop().await.unwrap();
    }
}

/// How an outbound direct link ended.
#[derive(Debug, Clone, Copy)]
enum LinkEnd {
    /// This node retired it, as `stop` does.
    Retired,
    /// The peer sent a Disconnect-Request.
    Answered,
    /// The peer left a heartbeat unanswered.
    Unanswered,
}

/// An outbound direct link's worker closes its socket by how the link
/// ended: 1000 after its own Disconnect-Request or after answering the
/// peer's, no status when the peer stopped answering heartbeats.
#[tokio::test(start_paused = true)]
async fn an_outbound_direct_link_closes_by_how_it_ended() {
    use crate::sc::direct_membership::{DirectMembership, DirectRole, disconnect_request};
    use crate::sc::direct_pool::PooledDirect;
    use crate::sc::direct_socket::DirectSocket;
    for (end, expected) in [
        (LinkEnd::Retired, Some(1000)),
        (LinkEnd::Answered, Some(1000)),
        (LinkEnd::Unanswered, None),
    ] {
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
            (3_000, 6_000),
        );
        match end {
            LinkEnd::Retired => pooled.member.retire(),
            LinkEnd::Answered => {
                let mut wire = bytes::BytesMut::new();
                crate::sc_frame::encode_sc_message(&mut wire, &disconnect_request());
                probe.hub.send(&wire).await.unwrap();
            }
            // The worker probes at 3 s and gives up at 6 s.
            LinkEnd::Unanswered => {}
        }
        tokio::time::timeout(Duration::from_secs(30), worker)
            .await
            .expect("the link did not end")
            .unwrap();
        assert_eq!(probe.closes(), [expected], "{end:?}");
    }
}

/// A start where neither hub answers its Connect-Request gives up both
/// sockets, and closes each with no status.
#[tokio::test(start_paused = true)]
async fn a_start_that_neither_hub_answers_closes_both_sockets() {
    let (primary, primary_probe) = recorder();
    let (failover, failover_probe) = recorder();
    let mut transport = ScTransport::new(primary, NODE_VMAC)
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_failover(failover);
    let error = match transport.start().await {
        Err(error) => error,
        Ok(_) => panic!("no hub answered"),
    };
    assert!(matches!(error, Error::Timeout(_)), "{error}");
    assert_eq!(primary_probe.closes(), [None]);
    assert_eq!(failover_probe.closes(), [None]);
}

/// Once the primary's reconnect attempts are used up, a failover hub that
/// never answers the Connect-Request is given up and closed with no status.
#[tokio::test(start_paused = true)]
async fn an_unanswered_failover_after_reconnect_exhaustion_is_closed() {
    let (socket, probe) = recorder();
    let (dialled_tx, mut dialled) = tokio::sync::mpsc::unbounded_channel();
    let mut transport = ScTransport::new(socket, NODE_VMAC)
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(100)
        .with_test_heartbeat_timing_ms(100, 300)
        .with_reconnect(reconnect())
        // The primary's redials reach a hub that is already gone.
        .with_connector(|| async { Ok(recorder().0) })
        .with_failover_connector(move || {
            let (socket, probe) = recorder();
            let _ = dialled_tx.send(probe);
            async { Ok(socket) }
        });
    started(&mut transport, &probe).await;
    let failover = tokio::time::timeout(Duration::from_secs(5), dialled.recv())
        .await
        .expect("no failover dial")
        .unwrap();
    // The close comes after the connect wait, so step the paused clock.
    let deadline = Instant::now() + Duration::from_secs(5);
    while failover.closes().is_empty() {
        assert!(
            Instant::now() < deadline,
            "the failover socket was not closed"
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(failover.closes(), [None]);
    transport.stop().await.unwrap();
}
