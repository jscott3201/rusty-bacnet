//! One real serial/MAC owner; gates observe admission without replacing transport behavior.
use crate::mstp_wire::{FrameType, MstpFrame, decode_frame, encode_frame};
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_objects::{analog::AnalogInputObject, database::ObjectDatabase};
use bacnet_server::server::{BACnetServer, ServerConfig};
use bacnet_transport::{
    any::AnyTransport,
    mstp::{
        LoopbackSerial, MstpConfig, MstpDiagnostics, MstpExecutionMode, MstpTransport, SerialPort,
    },
    port::TransportPort,
    test_transport::{SendMode, TestTransport},
};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use std::{
    collections::VecDeque,
    future::Future,
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::Duration,
};
use tokio::sync::{Semaphore, mpsc};

pub const NODE: u8 = 3;
pub const PEER: u8 = 7;
pub const BROADCAST: u8 = 255;
/// Frames kept for a failure message.
const TRACE_FRAMES: usize = 40;

/// One fixture's MS/TP link as a failure message sees it: the node's state
/// and counters, and the last [`TRACE_FRAMES`] frames each side sent,
/// stamped with Tokio time since the fixture started (#2277): virtual
/// time in the paused variants (the dedicated ones on test time, #2284),
/// wall time in the dedicated smoke test.
///
/// `deadline_lateness` and the wall-clock stamps tell host starvation
/// (deadlines serviced seconds late, gaps between frames) from a node that
/// stopped: under background QoS on a loaded macOS host, a 20 ms sleep can
/// wake seconds late.
#[derive(Clone)]
pub struct Link {
    /// Set once the transport exists, before it moves into its owner.
    diagnostics: Arc<OnceLock<MstpDiagnostics>>,
    trace: Arc<StdMutex<VecDeque<String>>>,
    start: tokio::time::Instant,
}
impl Default for Link {
    fn default() -> Self {
        Self {
            diagnostics: Default::default(),
            trace: Default::default(),
            start: tokio::time::Instant::now(),
        }
    }
}
impl Link {
    fn note(&self, dir: &str, frame: &MstpFrame) {
        let mut trace = self.trace.lock().unwrap();
        if trace.len() == TRACE_FRAMES {
            trace.pop_front();
        }
        trace.push_back(format!(
            "{:>8.3?} {dir} {:?} ->{} {:02x?}",
            self.start.elapsed(),
            frame.frame_type,
            frame.destination,
            &frame.data[..frame.data.len().min(8)]
        ));
    }
    /// The node's state, counters and recent frames.
    pub fn report(&self) -> String {
        let trace = self.trace.lock().unwrap();
        format!(
            "{:?} since start; node {:?}; {:?}; frames:\n{}",
            self.start.elapsed(),
            self.diagnostics.get().map(MstpDiagnostics::node_state),
            self.diagnostics.get().map(MstpDiagnostics::snapshot),
            trace.iter().cloned().collect::<Vec<_>>().join("\n")
        )
    }
    /// Runs `future` with a 5 s hang backstop. On paused time (the Tokio
    /// variants, and the dedicated ones on test time) the clock moves only
    /// when every task, and the dedicated node, is idle, so the backstop
    /// fires only when nothing can run; on the wall clock (the dedicated
    /// smoke test) host starvation counts against it too.
    pub async fn bounded<T>(&self, future: impl Future<Output = T>) -> T {
        match tokio::time::timeout(Duration::from_secs(5), future).await {
            Ok(value) => value,
            Err(_) => panic!("MS/TP fixture made no progress; {}", self.report()),
        }
    }
}

pub struct Gates {
    pub hold_send: bool,
    pub hold_write: bool,
    pub hold_stop: bool,
    pub queued: Semaphore,
    pub send_dropped: Semaphore,
    pub stop_started: Semaphore,
    pub stop_release: Semaphore,
    pub serial_dropped: Semaphore,
}
impl Gates {
    pub fn new(hold_send: bool, hold_write: bool, hold_stop: bool) -> Arc<Self> {
        Arc::new(Self {
            hold_send,
            hold_write,
            hold_stop,
            queued: Semaphore::new(0),
            send_dropped: Semaphore::new(0),
            stop_started: Semaphore::new(0),
            stop_release: Semaphore::new(0),
            serial_dropped: Semaphore::new(0),
        })
    }
}
struct SignalOnDrop<'a>(&'a Semaphore);
impl Drop for SignalOnDrop<'_> {
    fn drop(&mut self) {
        self.0.add_permits(1);
    }
}
pub struct ObservedSerial {
    /// Live until drop, which takes it before signalling `serial_dropped`.
    inner: Option<LoopbackSerial>,
    /// Live until drop, as for `inner`.
    writes: Option<mpsc::UnboundedSender<MstpFrame>>,
    gates: Arc<Gates>,
    link: Link,
}
impl ObservedSerial {
    fn wire(&self) -> &LoopbackSerial {
        self.inner.as_ref().expect("serial wire is live until drop")
    }
}
impl Drop for ObservedSerial {
    fn drop(&mut self) {
        // A Drop body runs before the struct's fields drop. Close the simulated
        // wire and the frame observer explicitly first, so a waiter that sees the
        // permit also sees peer writes fail; in DedicatedThread mode that waiter
        // runs on another thread.
        drop(self.inner.take());
        drop(self.writes.take());
        self.gates.serial_dropped.add_permits(1);
    }
}
impl SerialPort for ObservedSerial {
    async fn write(&self, data: &[u8]) -> Result<(), Error> {
        let (frame, consumed) = decode_frame(data)?;
        assert_eq!(
            consumed,
            data.len(),
            "one complete standard frame per write"
        );
        if self.gates.hold_write && frame.data.starts_with(&[1, 0x80, 0x13]) {
            // Cancellation may stop a write before any simulated bytes complete.
            std::future::pending::<()>().await;
        }
        self.wire().write(data).await?;
        self.link.note("node", &frame);
        self.writes
            .as_ref()
            .expect("observer is live until drop")
            .send(frame)
            .expect("observer stays attached");
        Ok(())
    }
    async fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        self.wire().read(buf).await
    }
}

/// `inner` with the gates' holds: a stop that waits for `stop_release`, and a
/// local Number control broadcast that, once it has reached the link, stays
/// pending. Everything else is the real transport's.
pub fn gated_transport(inner: AnyTransport, gates: Arc<Gates>) -> TestTransport {
    let send_gates = gates.clone();
    TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::ForwardFirst)
        .before_stop(move || {
            let gates = gates.clone();
            async move {
                if gates.hold_stop {
                    gates.stop_started.add_permits(1);
                    gates.stop_release.acquire().await.unwrap().forget();
                }
                Ok(())
            }
        })
        .on_send(move |frame| {
            let gates = send_gates.clone();
            async move {
                if frame.broadcast && gates.hold_send && frame.npdu.starts_with(&[1, 0x80, 0x13]) {
                    let _signal = SignalOnDrop(&gates.send_dropped);
                    gates.queued.add_permits(1);
                    std::future::pending::<()>().await;
                }
                Ok(())
            }
        })
        .build()
}

/// The three application owners of one MS/TP link's local Number controls.
#[derive(Clone, Copy, Debug)]
pub enum Role {
    Server,
    Endpoint,
    /// Standalone client: no Device object, database or configured authority.
    Client,
}

pub enum Owner {
    Server(Box<BACnetServer<TestTransport>>),
    Endpoint(Box<EndpointSession<TestTransport>>),
    Client(Box<BACnetClient<TestTransport>>),
}
impl Owner {
    async fn start(role: Role, transport: TestTransport, link: &Link) -> Self {
        let mut db = ObjectDatabase::new();
        let mut analog =
            AnalogInputObject::new(1, "number-progress", EngineeringUnits::SQUARE_METERS).unwrap();
        analog.set_present_value(42.0);
        db.add(Box::new(analog)).unwrap();
        match role {
            Role::Server => Self::Server(Box::new(
                link.bounded(BACnetServer::start(ServerConfig::default(), db, transport))
                    .await
                    .unwrap(),
            )),
            Role::Endpoint => {
                let mut session =
                    EndpointSession::new(transport, SessionRole::Both, Default::default())
                        .unwrap()
                        .with_database(db);
                link.bounded(session.start()).await.unwrap();
                Self::Endpoint(Box::new(session))
            }
            Role::Client => Self::Client(Box::new(
                link.bounded(BACnetClient::start(ClientConfig::default(), transport))
                    .await
                    .unwrap(),
            )),
        }
    }
    pub async fn stop(&mut self) {
        match self {
            Self::Server(owner) => {
                owner.stop().await.unwrap();
            }
            Self::Endpoint(owner) => {
                owner.stop().await.unwrap();
            }
            Self::Client(owner) => {
                owner.stop().await.unwrap();
            }
        }
    }
}
pub struct Peer {
    pub serial: Arc<LoopbackSerial>,
    pub writes: mpsc::UnboundedReceiver<MstpFrame>,
    pub link: Link,
    drain: tokio::task::JoinHandle<()>,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.drain.abort();
    }
}
impl Peer {
    pub async fn frame(&self, kind: FrameType, destination: u8, data: &[u8]) -> Result<(), Error> {
        let mut wire = BytesMut::new();
        let frame = MstpFrame {
            frame_type: kind,
            destination,
            source: PEER,
            data: Bytes::copy_from_slice(data),
        };
        encode_frame(&mut wire, &frame).unwrap();
        self.link.note("peer", &frame);
        self.serial.write(&wire).await
    }
    pub async fn control(&self, destination: u8, data: &[u8]) {
        self.link
            .bounded(self.frame(
                FrameType::BACNET_DATA_NOT_EXPECTING_REPLY,
                destination,
                data,
            ))
            .await
            .unwrap();
    }
    pub async fn number(&mut self, number: u16) {
        let frame = self.data().await;
        assert_eq!(frame.frame_type, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
        assert_eq!(frame.destination, BROADCAST);
        assert_eq!(
            frame.data.as_ref(),
            [1, 0x80, 0x13, (number >> 8) as u8, number as u8, 0]
        );
    }
    /// The node's next data frame. The peer plays the node's only other
    /// master: it hands over the token, answers a Poll For Master addressed
    /// to it, and passes back every token it is given. A poll for another
    /// address is left to time out; a token sent while the node polls would
    /// be an unexpected frame there (Clause 9.5.6.8) and cost the token.
    pub async fn data(&mut self) -> MstpFrame {
        let link = self.link.clone();
        link.bounded(async {
            self.frame(FrameType::TOKEN, NODE, &[]).await.unwrap();
            loop {
                let frame = self.writes.recv().await.expect("serial owner disappeared");
                assert_eq!(frame.source, NODE);
                match frame.frame_type {
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY
                    | FrameType::BACNET_DATA_EXPECTING_REPLY => return frame,
                    FrameType::POLL_FOR_MASTER if frame.destination == PEER => {
                        self.frame(FrameType::REPLY_TO_POLL_FOR_MASTER, NODE, &[])
                            .await
                            .unwrap();
                    }
                    FrameType::TOKEN if frame.destination == PEER => {
                        self.frame(FrameType::TOKEN, NODE, &[]).await.unwrap();
                    }
                    FrameType::POLL_FOR_MASTER | FrameType::REPLY_POSTPONED => {}
                    other => panic!("unexpected frame {other:?}"),
                }
            }
        })
        .await
    }
    /// Wait until the node passes the token to the peer, answering its
    /// polls on the way: a master sends a request only while it holds the
    /// token.
    pub async fn await_token(&mut self) {
        let link = self.link.clone();
        link.bounded(async {
            loop {
                let frame = self.writes.recv().await.expect("serial owner disappeared");
                assert_eq!(frame.source, NODE);
                match frame.frame_type {
                    FrameType::POLL_FOR_MASTER if frame.destination == PEER => {
                        self.frame(FrameType::REPLY_TO_POLL_FOR_MASTER, NODE, &[])
                            .await
                            .unwrap();
                    }
                    FrameType::TOKEN if frame.destination == PEER => return,
                    _ => {}
                }
            }
        })
        .await
    }

    /// The node's answer to a request the peer sent while holding the
    /// token. After Reply Postponed the peer passes the token on, so the
    /// node can send the answer with it.
    pub async fn answer(&mut self) -> MstpFrame {
        let link = self.link.clone();
        link.bounded(async {
            loop {
                let frame = self.writes.recv().await.expect("serial owner disappeared");
                assert_eq!(frame.source, NODE);
                match frame.frame_type {
                    FrameType::BACNET_DATA_NOT_EXPECTING_REPLY => return frame,
                    FrameType::REPLY_POSTPONED => {
                        self.frame(FrameType::TOKEN, NODE, &[]).await.unwrap();
                    }
                    FrameType::POLL_FOR_MASTER if frame.destination == PEER => {
                        self.frame(FrameType::REPLY_TO_POLL_FOR_MASTER, NODE, &[])
                            .await
                            .unwrap();
                    }
                    FrameType::TOKEN if frame.destination == PEER => {
                        self.frame(FrameType::TOKEN, NODE, &[]).await.unwrap();
                    }
                    _ => {}
                }
            }
        })
        .await
    }

    pub async fn assert_released(&mut self, gates: &Gates) {
        self.link
            .bounded(gates.serial_dropped.acquire())
            .await
            .unwrap()
            .forget();
        assert!(
            self.link
                .bounded(self.frame(FrameType::TOKEN, NODE, &[]))
                .await
                .is_err(),
            "no receive owner after stop/drop"
        );
        while let Ok(frame) = self.writes.try_recv() {
            assert!(
                !frame.data.starts_with(&[1, 0x80, 0x13]),
                "no completed number write after queued stop"
            );
        }
    }
}
pub async fn transport(mode: MstpExecutionMode, gates: Arc<Gates>) -> (TestTransport, Peer) {
    let (serial, peer) = LoopbackSerial::pair();
    let (tx, writes) = mpsc::unbounded_channel();
    let link = Link::default();
    let transport = MstpTransport::new(
        ObservedSerial {
            inner: Some(serial),
            writes: Some(tx),
            gates: gates.clone(),
            link: link.clone(),
        },
        MstpConfig {
            this_station: NODE,
            max_master: PEER,
            ..Default::default()
        },
    )
    .with_execution_mode(mode);
    // Taken before the transport moves into its owner.
    link.diagnostics
        .set(transport.diagnostics())
        .expect("one transport per link");
    let inner = AnyTransport::from(transport);
    assert_eq!(inner.local_receive_apdu_capacity(), 480);
    assert_eq!(inner.egress_apdu_limit(), 480);
    assert!(inner.bip_port().is_none(), "no configured B/IP authority");
    let serial = Arc::new(peer);
    let drain_serial = serial.clone();
    let drain = tokio::spawn(async move {
        let mut buf = [0; 2048];
        while drain_serial.read(&mut buf).await.is_ok() {}
    });
    (
        gated_transport(inner, gates),
        Peer {
            serial,
            writes,
            link,
            drain,
        },
    )
}
pub async fn fixture(role: Role, mode: MstpExecutionMode, gates: Arc<Gates>) -> (Owner, Peer) {
    let (transport, peer) = transport(mode, gates).await;
    (Owner::start(role, transport, &peer.link).await, peer)
}
