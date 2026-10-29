//! Live server tests for the local encoded-response issuance boundary.
//! A transport exposes an encoded NPDU, then holds its send Result unresolved.
//! This is local operation observation, not physical delivery or peer receipt.
use super::*;
use crate::server::test_transport::{SendMode, SentFrame, StartMode, TestTransport};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::value_types::CharacterStringValueObject;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;

struct CountedValue {
    inner: CharacterStringValueObject,
    writes: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    panic_on_read: Arc<AtomicBool>,
    description: String,
}
impl BACnetObject for CountedValue {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        if p == PropertyIdentifier::DESCRIPTION {
            self.reads.fetch_add(1, Ordering::AcqRel);
            assert!(
                !self.panic_on_read.swap(false, Ordering::AcqRel),
                "injected object panic before response"
            );
            return Ok(PropertyValue::CharacterString(self.description.clone()));
        }
        self.inner.read_property(p, i)
    }
    fn write_property(
        &mut self,
        p: PropertyIdentifier,
        i: Option<u32>,
        v: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property(p, i, v, priority)?;
        self.writes.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }
    fn property_metadata(&self) -> Cow<'_, [bacnet_objects::property_metadata::PropertyMetadata]> {
        self.inner.property_metadata()
    }
}
struct Finished(Option<oneshot::Sender<()>>);
impl Drop for Finished {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}
struct Issued {
    npdu: Npdu,
    apdu: Apdu,
    release: oneshot::Sender<Result<(), Error>>,
    finished: oneshot::Receiver<()>,
}
/// The gated send path, installed as the shared test transport's send hook.
#[derive(Clone)]
struct Gate {
    issued: mpsc::UnboundedSender<Issued>,
}
impl Gate {
    async fn send(self, frame: SentFrame) -> Result<(), Error> {
        let npdu = decode_npdu(frame.npdu).unwrap();
        let apdu = apdu::decode_apdu(npdu.payload.clone()).unwrap();
        let (release, wait) = oneshot::channel();
        let (done, finished) = oneshot::channel();
        let _done = Finished(Some(done));
        self.issued
            .send(Issued {
                npdu,
                apdu,
                release,
                finished,
            })
            .unwrap();
        wait.await
            .unwrap_or_else(|_| Err(Error::Encoding("test send cancelled".into())))
    }
}
/// A link with local MAC `[2]` that exposes each unicast as [`Issued`] and
/// holds it until released; broadcasts succeed unrecorded. Without `incoming`
/// the link must never start.
fn gated_port(
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    issued: mpsc::UnboundedSender<Issued>,
) -> TestTransport {
    let gate = Gate { issued };
    TestTransport::builder()
        .local_mac(&[2])
        .start(match incoming {
            Some(incoming) => StartMode::Inbound(Some(incoming)),
            None => StartMode::Panic("gated port has no inbound channel"),
        })
        .broadcast(SendMode::Ignore)
        .on_send(move |frame| gate.clone().send(frame))
        .build()
}
struct Fixture {
    server: BACnetServer<TestTransport>,
    incoming: mpsc::Sender<ReceivedNpdu>,
    issued: mpsc::UnboundedReceiver<Issued>,
    held: Vec<Issued>,
    writes: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    panic_on_read: Arc<AtomicBool>,
}
impl Fixture {
    async fn new() -> Self {
        Self::configured(ServerConfig::default(), 16).await
    }
    async fn configured(config: ServerConfig, description_len: usize) -> Self {
        let writes = Arc::new(AtomicUsize::new(0));
        let reads = Arc::new(AtomicUsize::new(0));
        let panic_on_read = Arc::new(AtomicBool::new(false));
        let mut db = ObjectDatabase::new();
        db.add(Box::new(CountedValue {
            inner: CharacterStringValueObject::new(1, "value").unwrap(),
            writes: writes.clone(),
            reads: reads.clone(),
            panic_on_read: panic_on_read.clone(),
            description: "r".repeat(description_len),
        }))
        .unwrap();
        let (incoming, receive) = mpsc::channel(32);
        let (issued, observed) = mpsc::unbounded_channel();
        let server = BACnetServer::start(config, db, gated_port(Some(receive), issued))
            .await
            .unwrap();
        Self {
            server,
            incoming,
            issued: observed,
            held: Vec::new(),
            writes,
            reads,
            panic_on_read,
        }
    }
    async fn inject(&self, request: &Apdu, routed: bool) {
        self.inject_with_reply(request, routed, None).await;
    }
    async fn inject_with_reply(
        &self,
        request: &Apdu,
        routed: bool,
        reply: Option<oneshot::Sender<Bytes>>,
    ) {
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, request).unwrap();
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                source: routed.then(|| NpduAddress {
                    network: 123,
                    mac_address: MacAddr::from_slice(&[3]),
                }),
                payload: payload.freeze(),
                ..Default::default()
            },
        )
        .unwrap();
        self.incoming
            .send(ReceivedNpdu::unverified(
                npdu.freeze(),
                MacAddr::from_slice(&[1]),
                false,
                Vec::new(),
                reply,
            ))
            .await
            .unwrap();
    }
    async fn barrier(&mut self, invoke: u8, routed: bool) {
        self.inject(
            &request(invoke, ConfirmedServiceChoice::from_raw(254), Bytes::new()),
            routed,
        )
        .await;
        loop {
            let event = bounded(self.issued.recv()).await.unwrap();
            let found = matches!(&event.apdu, Apdu::Reject(r) if r.invoke_id == invoke);
            self.held.push(event);
            if found {
                return;
            }
        }
    }
    async fn observe_write_replies(&mut self, count: usize) {
        while self
            .held
            .iter()
            .filter(|e| matches!(e.apdu, Apdu::SimpleAck(_)))
            .count()
            < count
        {
            self.held.push(bounded(self.issued.recv()).await.unwrap());
        }
    }
    fn pending(&self, req: &Apdu, routed: bool) -> bool {
        let Apdu::ConfirmedRequest(req) = req else {
            panic!("not confirmed")
        };
        matches!(
            self.server.confirmed_request_tracker.begin(
                &[1],
                routed.then_some(&NpduAddress {
                    network: 123,
                    mac_address: MacAddr::from_slice(&[3])
                }),
                TransportProvenance::unverified(),
                req.clone()
            ),
            ConfirmedRequestAdmission::Duplicate
        )
    }
    async fn finish_next(&mut self, result: Result<(), Error>) -> Apdu {
        let event = bounded(self.issued.recv()).await.unwrap();
        event.release.send(result).unwrap();
        bounded(event.finished).await.unwrap();
        event.apdu
    }
    async fn stop(mut self) {
        self.server.stop().await.unwrap();
        for held in &mut self.held {
            bounded(&mut held.finished).await.unwrap();
        }
    }
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("issuance fixture stalled")
}
fn request(invoke_id: u8, service_choice: ConfirmedServiceChoice, service_request: Bytes) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request,
    })
}
fn write(invoke: u8) -> Apdu {
    let mut value = BytesMut::new();
    encode_property_value(
        &mut value,
        &PropertyValue::CharacterString("identical".into()),
    )
    .unwrap();
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    request(
        invoke,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        body.freeze(),
    )
}
async fn reuse_while_old_send_is_pending(routed: bool) {
    let mut f = Fixture::new().await;
    let db = f.server.db.clone();
    let blocked = db.write().await;
    f.inject(&write(1), routed).await;
    f.inject(&write(1), routed).await;
    f.barrier(240, routed).await;
    assert_eq!(
        f.server
            .request_admission_counters()
            .confirmed_admitted_total,
        2
    );
    assert_eq!(f.writes.load(Ordering::Acquire), 0);
    drop(blocked);
    f.observe_write_replies(1).await;
    assert_eq!(f.writes.load(Ordering::Acquire), 1);
    // The first encoded response is issued locally, but its transport future
    // and handler are still held. A legal new identical operation must execute.
    f.inject(&write(1), routed).await;
    f.inject(&write(2), routed).await; // fresh Invoke ID positive control
    f.barrier(241, routed).await;
    assert_eq!(
        f.server
            .request_admission_counters()
            .confirmed_admitted_total,
        5
    );
    f.observe_write_replies(3).await;
    assert_eq!(f.writes.load(Ordering::Acquire), 3);
    for held in &mut f.held {
        assert!(matches!(
            held.finished.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        assert_eq!(held.npdu.destination.is_some(), routed);
    }
    assert_eq!(f.server.request_admission_counters().confirmed_active, 5);
    f.stop().await;
}
#[tokio::test]
async fn confirmed_reuse_at_local_issuance_while_old_send_is_pending() {
    reuse_while_old_send_is_pending(false).await;
}
#[tokio::test]
async fn confirmed_routed_reuse_at_local_issuance_while_old_send_is_pending() {
    reuse_while_old_send_is_pending(true).await;
}

fn read(invoke: u8, property: PropertyIdentifier) -> Apdu {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::CHARACTERSTRING_VALUE, 1).unwrap(),
        property_identifier: property,
        property_array_index: None,
    }
    .encode(&mut body);
    request(invoke, ConfirmedServiceChoice::READ_PROPERTY, body.freeze())
}
async fn until(mut condition: impl FnMut() -> bool) {
    bounded(async {
        while !condition() {
            tokio::task::yield_now().await;
        }
    })
    .await;
}
#[path = "confirmed_response_lifetime_tests.rs"]
mod response_lifetimes;
#[path = "confirmed_segmented_lifetime_tests.rs"]
mod segmented_lifetimes;
