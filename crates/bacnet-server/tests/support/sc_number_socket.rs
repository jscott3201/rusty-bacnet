//! Deterministic single-writer SC boundary; this is not TLS/backpressure proof.
use bacnet_transport::{
    sc::{LoopbackWebSocket, ScTransport, WebSocketPort},
    sc_frame::{BROADCAST_VMAC, ScFunction, ScMessage, decode_sc_message, encode_sc_message},
};
use bacnet_types::{error::Error, primitives::ObjectIdentifier};
use bytes::{Bytes, BytesMut};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Mutex, Semaphore};

pub const PEER: [u8; 6] = [0x30; 6];
pub async fn bounded<T>(f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(3), f)
        .await
        .expect("gated SC owner made no progress")
}
pub async fn count(value: &AtomicUsize, expected: usize) {
    bounded(async {
        while value.load(Ordering::SeqCst) != expected {
            tokio::task::yield_now().await;
        }
    })
    .await;
}
pub struct Observed {
    pub hold_number: AtomicBool,
    pub numbers_started: AtomicUsize,
    pub numbers_dropped: AtomicUsize,
    pub numbers_completed: AtomicUsize,
    pub number_release: Semaphore,
    pub hold_disconnect: AtomicBool,
    pub disconnect_started: AtomicUsize,
    pub disconnect_release: Semaphore,
    pub socket_dropped: AtomicUsize,
}
struct Pending<'a>(&'a AtomicUsize);
impl Drop for Pending<'_> {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
pub struct GateSocket {
    inner: LoopbackWebSocket,
    observed: Arc<Observed>,
    writer: Mutex<()>,
}
impl Drop for GateSocket {
    fn drop(&mut self) {
        self.observed.socket_dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl WebSocketPort for GateSocket {
    async fn send(&self, data: &[u8]) -> Result<(), Error> {
        let _writer = self.writer.lock().await;
        let frame = decode_sc_message(data).unwrap();
        if frame.function == ScFunction::EncapsulatedNpdu
            && frame.payload.starts_with(&[1, 0x80, 0x13])
        {
            self.observed.numbers_started.fetch_add(1, Ordering::SeqCst);
            let _pending = Pending(&self.observed.numbers_dropped);
            if self.observed.hold_number.load(Ordering::SeqCst) {
                self.observed
                    .number_release
                    .acquire()
                    .await
                    .unwrap()
                    .forget();
            }
            self.inner.send(data).await?;
            self.observed
                .numbers_completed
                .fetch_add(1, Ordering::SeqCst);
            return Ok(());
        }
        if frame.function == ScFunction::DisconnectRequest
            && self.observed.hold_disconnect.load(Ordering::SeqCst)
        {
            self.observed
                .disconnect_started
                .fetch_add(1, Ordering::SeqCst);
            self.observed
                .disconnect_release
                .acquire()
                .await
                .unwrap()
                .forget();
        }
        self.inner.send(data).await
    }
    async fn recv(&self) -> Result<Vec<u8>, Error> {
        self.inner.recv().await
    }
}
pub struct Peer(LoopbackWebSocket);
impl Peer {
    pub async fn accept(&self) {
        let wire = bounded(self.0.recv()).await.unwrap();
        let request = decode_sc_message(&wire).unwrap();
        assert_eq!(request.function, ScFunction::ConnectRequest);
        let mut payload = vec![0x10; 6];
        payload.extend_from_slice(&[0x11; 16]);
        payload.extend_from_slice(&6000u16.to_be_bytes());
        payload.extend_from_slice(&1497u16.to_be_bytes());
        let frame = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: request.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: vec![],
            data_options: vec![],
            payload: Bytes::from(payload),
        };
        self.frame(frame).await;
    }
    async fn frame(&self, frame: ScMessage) {
        let mut wire = BytesMut::new();
        encode_sc_message(&mut wire, &frame);
        bounded(self.0.send(&wire)).await.unwrap();
    }
    pub async fn send(&self, group: bool, npdu: &[u8]) {
        self.frame(ScMessage {
            function: ScFunction::EncapsulatedNpdu,
            message_id: 42,
            originating_vmac: Some(PEER),
            destination_vmac: group.then_some(BROADCAST_VMAC),
            dest_options: vec![],
            data_options: vec![],
            payload: Bytes::copy_from_slice(npdu),
        })
        .await;
    }
    pub async fn learn_and_query(&self) {
        self.send(true, &[1, 0x80, 0x13, 0, 17, 1]).await;
        self.send(false, &[1, 0x80, 0x12]).await;
    }
    pub async fn number(&self) {
        let wire = bounded(self.0.recv()).await.unwrap();
        let frame = decode_sc_message(&wire).unwrap();
        assert_eq!(frame.destination_vmac, Some(BROADCAST_VMAC));
        assert_eq!(frame.payload.as_ref(), &[1, 0x80, 0x13, 0, 17, 0]);
    }
}
pub fn fixture() -> (ScTransport<GateSocket>, Peer, Arc<Observed>) {
    let (inner, peer) = LoopbackWebSocket::pair();
    let observed = Arc::new(Observed {
        hold_number: true.into(),
        numbers_started: 0.into(),
        numbers_dropped: 0.into(),
        numbers_completed: 0.into(),
        number_release: Semaphore::new(0),
        hold_disconnect: false.into(),
        disconnect_started: 0.into(),
        disconnect_release: Semaphore::new(0),
        socket_dropped: 0.into(),
    });
    let socket = GateSocket {
        inner,
        observed: observed.clone(),
        writer: Mutex::new(()),
    };
    (
        // These tests fill the network-control queue from one peer, so the
        // per-source control-message rate limit (#518) mustn't bind first.
        ScTransport::new(socket, [0x20; 6])
            .with_device_uuid([0x21; 16])
            .with_control_rate_limit(bacnet_transport::sc::ScControlRateLimit::new(4096, 4096)),
        Peer(peer),
        observed,
    )
}
pub fn target() -> ObjectIdentifier {
    ObjectIdentifier::new(bacnet_types::enums::ObjectType::DEVICE, 7).unwrap()
}
pub fn database() -> bacnet_objects::database::ObjectDatabase {
    let mut db = bacnet_objects::database::ObjectDatabase::new();
    db.add(Box::new(
        bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(7, 0)
                .expect("valid Device identity")
                .with_name("progress"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db
}
pub fn write_npdu() -> Vec<u8> {
    use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, encode_apdu};
    use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value, "handled while held")
        .unwrap();
    let mut service = BytesMut::new();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: target(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut service)
    .unwrap();
    let mut npdu = BytesMut::from(&[1, 4][..]);
    encode_apdu(
        &mut npdu,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 480,
            invoke_id: 94,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    npdu.to_vec()
}
pub async fn handled(db: &tokio::sync::RwLock<bacnet_objects::database::ObjectDatabase>) {
    use bacnet_types::{enums::PropertyIdentifier, primitives::PropertyValue};
    bounded(async {
        loop {
            let value = db
                .read()
                .await
                .get(&target())
                .unwrap()
                .read_property(PropertyIdentifier::DESCRIPTION, None)
                .unwrap();
            if value == PropertyValue::CharacterString("handled while held".into()) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}
