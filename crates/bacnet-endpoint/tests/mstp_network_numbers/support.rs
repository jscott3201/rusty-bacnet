//! One real serial/MAC owner; gates observe admission without replacing transport behavior.
use crate::mstp_wire::{decode_frame, encode_frame, FrameType, MstpFrame};
use bacnet_client::client::{BACnetClient, ClientConfig};
use bacnet_endpoint::session::{EndpointSession, SessionRole};
use bacnet_objects::{analog::AnalogInputObject, database::ObjectDatabase};
use bacnet_server::server::{BACnetServer, ServerConfig};
use bacnet_transport::{
    any::AnyTransport,
    mstp::{LoopbackSerial, MstpConfig, MstpExecutionMode, MstpTransport, SerialPort},
    port::{ReceivedNpdu, TransportPort},
};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::Error;
use bytes::{Bytes, BytesMut};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, Semaphore};

pub const NODE: u8 = 3;
pub const PEER: u8 = 7;
pub const BROADCAST: u8 = 255;
pub async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), future)
        .await
        .expect("MS/TP fixture made no progress")
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

pub struct GatedTransport {
    inner: AnyTransport<ObservedSerial>,
    gates: Arc<Gates>,
}
impl TransportPort for GatedTransport {
    fn supports_local_nonrouter_number_controls(&self) -> bool {
        self.inner.supports_local_nonrouter_number_controls()
    }
    fn bip_port(&self) -> Option<bacnet_transport::port::BipPort> {
        self.inner.bip_port()
    }
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        if self.gates.hold_stop {
            self.gates.stop_started.add_permits(1);
            self.gates.stop_release.acquire().await.unwrap().forget();
        }
        self.inner.stop().await
    }
    fn abort(&mut self) {
        self.inner.abort();
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.inner.send_broadcast(npdu).await?;
        if self.gates.hold_send && npdu.starts_with(&[1, 0x80, 0x13]) {
            let _signal = SignalOnDrop(&self.gates.send_dropped);
            self.gates.queued.add_permits(1);
            std::future::pending::<()>().await;
        }
        Ok(())
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
    fn local_receive_apdu_capacity(&self) -> u16 {
        self.inner.local_receive_apdu_capacity()
    }
    fn egress_apdu_limit(&self) -> u16 {
        self.inner.egress_apdu_limit()
    }
    fn is_broadcast_mac(&self, mac: &[u8]) -> bool {
        self.inner.is_broadcast_mac(mac)
    }
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
    Server(Box<BACnetServer<GatedTransport>>),
    Endpoint(Box<EndpointSession<GatedTransport>>),
    Client(Box<BACnetClient<GatedTransport>>),
}
impl Owner {
    async fn start(role: Role, transport: GatedTransport) -> Self {
        let mut db = ObjectDatabase::new();
        let mut analog =
            AnalogInputObject::new(1, "number-progress", EngineeringUnits::SQUARE_METERS).unwrap();
        analog.set_present_value(42.0);
        db.add(Box::new(analog)).unwrap();
        match role {
            Role::Server => Self::Server(Box::new(
                bounded(BACnetServer::start(ServerConfig::default(), db, transport))
                    .await
                    .unwrap(),
            )),
            Role::Endpoint => {
                let mut session =
                    EndpointSession::new(transport, SessionRole::Both, Default::default())
                        .unwrap()
                        .with_database(db);
                bounded(session.start()).await.unwrap();
                Self::Endpoint(Box::new(session))
            }
            Role::Client => Self::Client(Box::new(
                bounded(BACnetClient::start(ClientConfig::default(), transport))
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
        encode_frame(
            &mut wire,
            &MstpFrame {
                frame_type: kind,
                destination,
                source: PEER,
                data: Bytes::copy_from_slice(data),
            },
        )
        .unwrap();
        self.serial.write(&wire).await
    }
    pub async fn control(&self, destination: u8, data: &[u8]) {
        bounded(self.frame(
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
        bounded(async {
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
        bounded(async {
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
        bounded(async {
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
        bounded(gates.serial_dropped.acquire())
            .await
            .unwrap()
            .forget();
        assert!(
            bounded(self.frame(FrameType::TOKEN, NODE, &[]))
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
pub async fn transport(mode: MstpExecutionMode, gates: Arc<Gates>) -> (GatedTransport, Peer) {
    let (serial, peer) = LoopbackSerial::pair();
    let (tx, writes) = mpsc::unbounded_channel();
    let transport = MstpTransport::new(
        ObservedSerial {
            inner: Some(serial),
            writes: Some(tx),
            gates: gates.clone(),
        },
        MstpConfig {
            this_station: NODE,
            max_master: PEER,
            ..Default::default()
        },
    )
    .with_execution_mode(mode);
    let inner = AnyTransport::Mstp(transport);
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
        GatedTransport { inner, gates },
        Peer {
            serial,
            writes,
            drain,
        },
    )
}
pub async fn fixture(role: Role, mode: MstpExecutionMode, gates: Arc<Gates>) -> (Owner, Peer) {
    let (transport, peer) = transport(mode, gates).await;
    (Owner::start(role, transport).await, peer)
}
