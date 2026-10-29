//! #2141: a connector that never answers hub probes but keeps its own Annex
//! AB.6.3 heartbeat stays registered; a silent one is still retired. Public
//! startup, real TLS handlers, paused time and the default probe policy
//! (30 s scans, 60 s idle age, 5 s ACK age, 360 s liveness bound).
use super::deadline_capacity_tests::CountedHub;
use super::deadline_test_support::*;
use super::heartbeat_test_support::frame;
use super::probe_tests::start;
use super::shutdown_blocked_tests::ControlledPeer;
use super::*;
use std::time::Duration;

const VMAC: Vmac = [0x42; 6];
/// Clock step; divides every scan and heartbeat time used here.
const STEP_S: u64 = 5;
const SCAN_S: u64 = 30;

/// How the simulated connector treats the hub's probes and its own timer.
#[derive(Clone, Copy, PartialEq)]
enum Connector {
    /// An initiator that ignores probes. It sends Heartbeat-Request once
    /// `timeout_s` passes without receiving any BVLC message; a hub probe and
    /// the ACK to its own request both restart that timer, as AB.6.3 has it.
    IgnoresProbes { timeout_s: u64 },
    /// Answers every probe and never heartbeats itself.
    AcksProbes,
    /// Sends nothing after Connect.
    Silent,
}

struct Sim {
    hub: CountedHub,
    peer: ControlledPeer,
    _tls: TestTls,
    connector: Connector,
    now_s: u64,
    sweeps: u64,
    /// When the connector last received a BVLC message.
    last_rx_s: u64,
    probes: u32,
    heartbeats: u32,
    next_id: u16,
}

impl Sim {
    async fn new(connector: Connector) -> Self {
        let tls = TestTls::new();
        let hub = start(&tls, ScHubProbePolicy::default()).await;
        until(|| hub.hub.tasks.probe_sweeps.load(Ordering::Acquire) == 1).await;
        let mut peer = poll_io(ControlledPeer::open(&tls, &hub)).await;
        poll_io(peer.connect(VMAC[0])).await;
        Self {
            hub,
            peer,
            _tls: tls,
            connector,
            now_s: 0,
            sweeps: 1,
            last_rx_s: 0, // Connect-Accept
            probes: 0,
            heartbeats: 0,
            next_id: 1,
        }
    }

    async fn registered(&self) -> bool {
        self.hub.clients.lock().await.contains_key(&VMAC)
    }

    async fn pending(&self) -> Option<super::heartbeat::PendingHeartbeat> {
        self.hub
            .clients
            .lock()
            .await
            .get(&VMAC)
            .and_then(|client| client.heartbeat.pending)
    }

    /// Advances one step; a scan due now runs to completion before the
    /// connector acts.
    async fn step(&mut self) {
        tokio::time::advance(Duration::from_secs(STEP_S)).await;
        self.now_s += STEP_S;
        if self.now_s.is_multiple_of(SCAN_S) {
            self.sweeps += 1;
            let want = self.sweeps;
            until(|| self.hub.hub.tasks.probe_sweeps.load(Ordering::Acquire) == want).await;
            self.after_sweep().await;
        }
        if let Connector::IgnoresProbes { timeout_s } = self.connector
            && self.now_s - self.last_rx_s >= timeout_s
            && self.registered().await
        {
            self.heartbeat().await;
        }
    }

    async fn after_sweep(&mut self) {
        // A probe reserved by this sweep carries this instant; the finished
        // sweep has already written it to the socket.
        let Some(probe) = self
            .pending()
            .await
            .filter(|pending| pending.published_at == self.now_s * 1000)
        else {
            return;
        };
        let request = self.recv().await;
        assert_eq!(request.function, ScFunction::HeartbeatRequest);
        assert_eq!(request.message_id, probe.message_id);
        self.probes += 1;
        self.last_rx_s = self.now_s;
        if self.connector == Connector::AcksProbes {
            self.send(frame(ScFunction::HeartbeatAck, probe.message_id))
                .await;
            poll_io(async {
                while self.pending().await.is_some() {
                    tokio::task::yield_now().await;
                }
            })
            .await;
        }
    }

    async fn heartbeat(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        self.send(frame(ScFunction::HeartbeatRequest, id)).await;
        let ack = self.recv().await;
        assert_eq!(ack.function, ScFunction::HeartbeatAck);
        assert_eq!(ack.message_id, id);
        self.heartbeats += 1;
        self.last_rx_s = self.now_s;
    }

    async fn send(&mut self, message: ScMessage) {
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &message);
        poll_io(self.peer.ws.send(Message::Binary(buf.freeze())))
            .await
            .unwrap();
    }

    async fn recv(&mut self) -> ScMessage {
        match poll_io(self.peer.ws.next()).await {
            Some(Ok(Message::Binary(bytes))) => decode_sc_message(&bytes).unwrap(),
            other => panic!("at {} s: expected a BVLC frame, got {other:?}", self.now_s),
        }
    }

    /// Steps to `until_s`, asserting the connector stays registered.
    async fn run_registered(&mut self, until_s: u64) {
        while self.now_s < until_s {
            self.step().await;
            assert!(self.registered().await, "retired at {} s", self.now_s);
        }
    }

    async fn stop(mut self) {
        poll_io(self.hub.hub.stop()).await;
        assert_eq!(self.hub.hub.tasks.len(), 0);
    }
}

/// The connector's own heartbeats keep it registered across many periods
/// although it never ACKs a probe. Each probe restarts its timer, so its
/// heartbeat lands one full timeout after the probe.
async fn ignores_probes_stays_registered(
    timeout_s: u64,
    until_s: u64,
    heartbeats: u32,
    probes: u32,
) {
    let mut sim = Sim::new(Connector::IgnoresProbes { timeout_s }).await;
    sim.run_registered(until_s).await;
    assert_eq!(sim.heartbeats, heartbeats, "connector heartbeats");
    // Every idle period got exactly one probe, never answered.
    assert_eq!(sim.probes, probes, "hub probes");
    assert_eq!(sim.hub.clients.outcomes.snapshot().heartbeat_retirements, 0);
    sim.stop().await;
}

#[tokio::test(start_paused = true)]
async fn connector_heartbeating_every_120s_without_probe_acks_stays_registered() {
    // Probes at 90 s, then 90 s after each heartbeat; heartbeats 120 s after
    // each probe: 210, 420, ... 1680 (8 by 1800 s), probes 90, 300, ... 1770 (9).
    ignores_probes_stays_registered(120, 1800, 8, 9).await;
}

#[tokio::test(start_paused = true)]
async fn connector_heartbeating_every_300s_without_probe_acks_stays_registered() {
    // The longest fixed AB.6.3 timeout: probes at 90 s, 480 s, ...;
    // heartbeats 300 s after each: 390, 780, ... 2340 (6 by 2400 s), probes 90, 480, ... 2040 (6).
    ignores_probes_stays_registered(300, 2400, 6, 6).await;
}

#[tokio::test(start_paused = true)]
async fn silent_connector_is_probed_once_and_retired_after_the_liveness_bound() {
    let mut sim = Sim::new(Connector::Silent).await;
    // Last frame (Connect) at 0; the 90 s scan is the first past the 60 s
    // idle age and probes. Retirement needs more than 360 s of silence after
    // that probe: the 450 s scan keeps it, the 480 s scan retires it. So a
    // silent connector goes at probe time + bound, rounded up to a scan.
    sim.run_registered(450).await;
    assert_eq!(sim.probes, 1, "an unanswered probe is not repeated");
    assert_eq!(sim.pending().await.unwrap().published_at, 90_000);
    sim.run_until_retired(480).await;
    assert_eq!(sim.hub.clients.outcomes.snapshot().heartbeat_retirements, 1);
    assert!(matches!(
        poll_io(sim.peer.ws.next()).await,
        None | Some(Err(_)) | Some(Ok(Message::Close(_)))
    ));
    sim.stop().await;
}

#[tokio::test(start_paused = true)]
async fn connector_that_acks_probes_is_probed_every_idle_period_and_stays() {
    let mut sim = Sim::new(Connector::AcksProbes).await;
    sim.run_registered(1800).await;
    // Each ACK refreshes activity, so the next probe waits for the next scan
    // past the idle age: one every 90 s, unchanged by #2141.
    assert_eq!(sim.probes, 20);
    assert_eq!(sim.heartbeats, 0);
    sim.stop().await;
}

impl Sim {
    /// Steps to `at_s`, which must be the first instant the connector is gone.
    async fn run_until_retired(&mut self, at_s: u64) {
        while self.now_s < at_s {
            assert!(self.registered().await, "retired early at {} s", self.now_s);
            self.step().await;
        }
        assert!(!self.registered().await, "still registered at {at_s} s");
    }
}
