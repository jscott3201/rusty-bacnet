//! The accepting side's idle bound against long initiator heartbeat periods
//! (#2133), on tokio's paused clock with the default configuration.
//!
//! Annex AB.6.3 caps a fixed heartbeat timeout at 300 seconds and makes a
//! configurable one support up to at least that, so an initiator that only
//! sends Heartbeat-Request at that pace must keep its link, as must one that
//! never probes because we keep sending to it. A silent link still closes at
//! the bound, and a socket that never sends Connect-Request is held only for
//! the connect wait.
use super::super::direct_accept_tests::TestCa;
use super::*;
use crate::sc::direct_membership::{DirectMembership, DirectRole, Membership};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

/// Frames fed by the test; `None` from the channel ends the stream.
struct Feed(mpsc::UnboundedReceiver<DirectFrame>);
impl DirectWsRead for Feed {
    async fn next_frame(&mut self) -> Option<Result<DirectFrame, String>> {
        self.0.recv().await.map(Ok)
    }
}

/// Records the function of every data frame the loop writes.
struct Sink(Arc<std::sync::Mutex<Vec<ScFunction>>>);
impl DirectWs for Sink {
    type Read = Feed;
    async fn send_data(&mut self, data: &[u8]) -> Result<(), ()> {
        self.0
            .lock()
            .unwrap()
            .push(decode_sc_message(data).unwrap().function);
        Ok(())
    }
    async fn send_close(&mut self) -> Result<(), ()> {
        Ok(())
    }
}

fn default_config() -> DirectAcceptConfig {
    let ca = TestCa::generate();
    DirectAcceptConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        [2; 6],
        [2; 16],
        ca.node_config(vec!["localhost".into()]),
    )
}

struct Link {
    member: Arc<Membership>,
    frames: UnboundedSender<DirectFrame>,
    sent: Arc<std::sync::Mutex<Vec<ScFunction>>>,
    /// Our side's writer onto the link (the loop's `Event::Write` arm).
    response: crate::port::DirectResponse,
    task: tokio::task::JoinHandle<()>,
}

impl Link {
    /// An established accepted link served by the production loop.
    fn start() -> Self {
        let registry = Arc::new(DirectMembership::default());
        let member = registry
            .reserve([1; 16], [1; 6], [2; 16], [2; 6], DirectRole::Accepted, 1)
            .unwrap()
            .commit_with_limits((1476, 1476), Duration::from_secs(10));
        let (frames, feed) = unbounded_channel();
        let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
        let config = default_config();
        let sink = Sink(sent.clone());
        let owner = member.clone();
        let identity = crate::port::DirectScIdentity::verified([3; 32], owner.generation);
        let response = crate::port::DirectResponse::new(&owner, identity);
        let writer = response.clone();
        let task = tokio::spawn(async move {
            let mut responses = owner.take_writes();
            let _retire = RetireMember(&owner);
            let (tx, _rx) = mpsc::channel(4);
            let admission = Arc::new(ScNpduAdmission::new(ScNpduAdmissionPolicy::default()));
            let warnings = DirectWarnings::default();
            let (mut sink, mut feed) = (sink, Feed(feed));
            serve_npdu_loop(
                &mut sink,
                &mut feed,
                &config,
                AdmittedDirectPeer {
                    address: "127.0.0.1:1".parse().unwrap(),
                    member: &owner,
                    provenance: crate::port::TransportProvenance::verified_direct_peer(
                        identity,
                        Default::default(),
                    ),
                    response,
                    warnings: &warnings,
                },
                &tx,
                &admission,
                &mut responses,
            )
            .await;
        });
        Self {
            member,
            frames,
            sent,
            response: writer,
            task,
        }
    }

    /// Sends one Heartbeat-Request and waits for the loop's ACK.
    async fn heartbeat(&self, message_id: u16) {
        let before = self.sent.lock().unwrap().len();
        let wire =
            crate::sc::direct_membership::bare_control(ScFunction::HeartbeatRequest, message_id);
        self.frames
            .send(DirectFrame::Binary(wire.to_vec()))
            .unwrap();
        while self.sent.lock().unwrap().len() == before {
            assert!(
                !self.task.is_finished(),
                "link closed before heartbeat {message_id}"
            );
            tokio::task::yield_now().await;
        }
        assert_eq!(
            self.sent.lock().unwrap().last(),
            Some(&ScFunction::HeartbeatAck)
        );
    }

    /// Lets the loop run at the current paused instant.
    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    fn open(&self) -> bool {
        !self.task.is_finished() && self.member.is_current()
    }
}

/// Keeps a link open across `periods` heartbeats sent `period` apart.
async fn survives(period: Duration, periods: u16) {
    let link = Link::start();
    Link::settle().await;
    for id in 1..=periods {
        tokio::time::advance(period).await;
        Link::settle().await;
        assert!(link.open(), "dropped before heartbeat {id} at {period:?}");
        link.heartbeat(id).await;
    }
    assert!(link.open());
}

#[tokio::test(start_paused = true)]
async fn initiator_heartbeating_every_120_seconds_keeps_the_link() {
    survives(Duration::from_secs(120), 5).await;
}

#[tokio::test(start_paused = true)]
async fn initiator_at_the_longest_fixed_heartbeat_keeps_the_link() {
    survives(Duration::from_secs(300), 3).await;
}

/// An initiator restarts its heartbeat timer on every BVLC message it
/// receives, so one we keep sending to never probes; our writes must hold
/// the link open by themselves.
#[tokio::test(start_paused = true)]
async fn our_writes_alone_keep_a_quiet_initiators_link() {
    let link = Link::start();
    Link::settle().await;
    let scope = crate::port::DirectResponseScope::default();
    for n in 1..=3 {
        tokio::time::advance(Duration::from_secs(200)).await;
        Link::settle().await;
        assert!(link.open(), "dropped before write {n}");
        link.response.send(&[1, 0], &scope).await.unwrap();
    }
    tokio::time::advance(Duration::from_secs(200)).await;
    Link::settle().await;
    assert!(link.open(), "dropped 200 s after the last write");
    assert_eq!(
        link.sent.lock().unwrap().as_slice(),
        [ScFunction::EncapsulatedNpdu; 3],
        "the peer sent nothing, so nothing but our writes went out"
    );
}

#[tokio::test(start_paused = true)]
async fn a_silent_link_still_closes_at_the_idle_bound() {
    let link = Link::start();
    Link::settle().await;
    link.heartbeat(1).await;
    // The bound restarts at the last inbound frame.
    tokio::time::advance(DIRECT_ACCEPT_IDLE_TIMEOUT - Duration::from_millis(1)).await;
    Link::settle().await;
    assert!(link.open(), "closed before the idle bound");
    tokio::time::advance(Duration::from_millis(1)).await;
    Link::settle().await;
    assert!(!link.open(), "a silent link outlived the idle bound");
    link.task.await.unwrap();
    assert!(!link.member.is_current());
}

/// The handshake keeps its own connect wait; the long idle bound never
/// applies before Connect-Request arrives.
#[tokio::test(start_paused = true)]
async fn a_socket_that_never_sends_connect_request_is_closed_at_the_connect_wait() {
    let config = default_config();
    let (_frames, feed) = unbounded_channel();
    let sent = Arc::new(std::sync::Mutex::new(Vec::new()));
    let membership = Arc::new(DirectMembership::default());
    let warnings = DirectWarnings::default();
    let started = tokio::time::Instant::now();
    let (mut sink, mut feed) = (Sink(sent.clone()), Feed(feed));
    let outcome = super::super::serve_handshake(
        &mut sink,
        &mut feed,
        &config,
        "127.0.0.1:1".parse().unwrap(),
        &membership,
        &warnings,
    )
    .await;
    assert!(outcome.is_none());
    assert_eq!(
        started.elapsed(),
        Duration::from_millis(super::super::DIRECT_ACCEPT_DEFAULT_CONNECT_TIMEOUT_MS)
    );
    assert!(sent.lock().unwrap().is_empty());
}

/// Over real sockets: a pending peer frees its slot after the connect wait
/// even though the idle bound is the long default.
#[tokio::test]
async fn a_pending_socket_frees_its_slot_without_waiting_for_the_idle_bound() {
    let ca = TestCa::generate();
    let config = DirectAcceptConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        [0xAA; 6],
        [9; 16],
        ca.node_config(vec!["localhost".into(), "127.0.0.1".into()]),
    )
    .with_connect_timeout(Duration::from_millis(200));
    let (mut listener, _rx) = DirectListener::start(config).await.unwrap();
    let url = format!(
        "wss://localhost:{}/.bacnet/sc",
        listener.local_addr().port()
    );
    let _ws = tokio::time::timeout(
        Duration::from_secs(5),
        crate::sc_tls::TlsWebSocket::connect_direct(&url, ca.node_config(vec!["node".into()])),
    )
    .await
    .expect("direct dial timed out")
    .expect("direct dial must succeed");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while listener.active_connections() != 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a pending socket outlived the connect wait"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    listener.stop().await;
}

#[tokio::test]
async fn start_rejects_zero_or_oversized_timeouts_before_binding() {
    let day = Duration::from_secs(24 * 60 * 60);
    for timeout in [Duration::ZERO, day + Duration::from_nanos(1), Duration::MAX] {
        let config = default_config().with_idle_timeout(timeout);
        assert!(
            DirectListener::start(config).await.is_err(),
            "idle {timeout:?} must be rejected"
        );
    }
    let five_minutes = Duration::from_secs(300);
    for timeout in [
        Duration::ZERO,
        five_minutes + Duration::from_nanos(1),
        Duration::MAX,
    ] {
        let config = default_config().with_connect_timeout(timeout);
        assert!(
            DirectListener::start(config).await.is_err(),
            "connect {timeout:?} must be rejected"
        );
    }
    // The caps themselves are accepted.
    let config = default_config()
        .with_idle_timeout(day)
        .with_connect_timeout(five_minutes);
    let (mut listener, _rx) = DirectListener::start(config).await.unwrap();
    listener.stop().await;
    // Short values stay a deliberate local choice.
    let config = default_config().with_idle_timeout(Duration::from_millis(1));
    let (mut listener, _rx) = DirectListener::start(config).await.unwrap();
    listener.stop().await;
}
