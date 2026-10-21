//! Routed local replies through real router delivery and production handlers.
use super::*;
use crate::server::test_transport::{SendMode, TestTransport};
use bacnet_encoding::{
    apdu::decode_apdu,
    npdu::{decode_npdu, encode_npdu, Npdu},
};
use bacnet_network::{
    layer::ReceivedApdu,
    router::{BACnetRouter, RouterOptions, RouterPort},
};
use bacnet_objects::analog::AnalogInputObject;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::enums::EngineeringUnits;
use std::sync::atomic::AtomicUsize;

struct Routed {
    router: BACnetRouter,
    frames: Vec<mpsc::Sender<ReceivedNpdu>>,
    local: mpsc::Receiver<ReceivedApdu>,
    wire: mpsc::UnboundedReceiver<(u16, Bytes, MacAddr)>,
}
fn address(network: u16, mac: &[u8]) -> NpduAddress {
    NpduAddress {
        network,
        mac_address: MacAddr::from_slice(mac),
    }
}
async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(3), future)
        .await
        .expect("bounded test operation")
}
impl Routed {
    async fn new() -> Self {
        let (wire, receiver) = mpsc::unbounded_channel();
        let mut frames = Vec::new();
        let mut ports = Vec::new();
        for (number, mac) in [(1000, 1), (2000, 2), (3000, 3)] {
            let (tx, rx) = mpsc::channel(16);
            frames.push(tx);
            let wire = wire.clone();
            let transport = TestTransport::builder()
                .local_mac(&[mac])
                .inbound(rx)
                .broadcast(SendMode::Ignore)
                .on_send(move |sent| {
                    let wire = wire.clone();
                    Box::pin(async move {
                        wire.send((number, sent.npdu, sent.mac)).unwrap();
                        Ok(())
                    })
                })
                .build();
            ports.push(RouterPort {
                transport,
                network_number: number,
            });
        }
        let started = BACnetRouter::start(ports, RouterOptions::new())
            .await
            .unwrap();
        Self {
            router: started.router,
            local: started.apdus,
            frames,
            wire: receiver,
        }
    }
    async fn receive(
        &mut self,
        network: u16,
        apdu: &Apdu,
        reply: Option<oneshot::Sender<Bytes>>,
    ) -> ReceivedApdu {
        self.receive_via(
            0,
            &[9],
            Some(address(9000, &[7])),
            Some(address(network, &[(network / 1000) as u8])),
            apdu,
            reply,
        )
        .await
    }
    async fn receive_via(
        &mut self,
        port: usize,
        peer: &[u8],
        source: Option<NpduAddress>,
        destination: Option<NpduAddress>,
        apdu: &Apdu,
        reply: Option<oneshot::Sender<Bytes>>,
    ) -> ReceivedApdu {
        let mut payload = BytesMut::new();
        encode_apdu(&mut payload, apdu).unwrap();
        let mut bytes = BytesMut::new();
        encode_npdu(
            &mut bytes,
            &Npdu {
                destination,
                source,
                payload: payload.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.frames[port]
            .send(ReceivedNpdu::unverified(
                bytes.freeze(),
                MacAddr::from_slice(peer),
                false,
                vec![],
                reply,
            ))
            .await
            .unwrap();
        bounded(self.local.recv()).await.unwrap()
    }
    async fn response(&mut self, network: u16, invoke: u8) -> (Apdu, Bytes) {
        let (port, bytes, peer) = bounded(self.wire.recv()).await.unwrap();
        assert_eq!(port, 1000);
        assert_eq!(peer.as_slice(), [9]);
        decode_response(bytes, network, invoke)
    }
}
fn decode_response(bytes: Bytes, network: u16, invoke: u8) -> (Apdu, Bytes) {
    let npdu = decode_npdu(bytes).unwrap();
    assert_eq!(
        npdu.source,
        Some(address(network, &[(network / 1000) as u8]))
    );
    assert_eq!(npdu.destination, Some(address(9000, &[7])));
    let apdu = decode_apdu(npdu.payload.clone()).unwrap();
    let id = match &apdu {
        Apdu::SimpleAck(x) => x.invoke_id,
        Apdu::ComplexAck(x) => x.invoke_id,
        Apdu::Abort(x) => x.invoke_id,
        Apdu::SegmentAck(x) => x.invoke_id,
        Apdu::Reject(x) => x.invoke_id,
        other => panic!("unexpected response: {other:?}"),
    };
    assert_eq!(id, invoke, "strict invoke ID with endpoint identity above");
    (apdu, npdu.payload)
}
fn request(service: ConfirmedServiceChoice, payload: Bytes, invoke: u8) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: invoke,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: payload,
    })
}
fn read(invoke: u8) -> Apdu {
    let mut payload = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 7).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut payload);
    request(
        ConfirmedServiceChoice::READ_PROPERTY,
        payload.freeze(),
        invoke,
    )
}
fn context(db: ObjectDatabase, config: ServerConfig) -> DispatchContext<TestTransport> {
    let transport = TestTransport::builder()
        .unicast(SendMode::Panic("router response used fallback"))
        .build();
    DispatchContext::for_test(RequestServices {
        db: Arc::new(RwLock::new(db)),
        ..RequestServices::for_test(Arc::new(NetworkLayer::new(transport)), config)
    })
}
async fn dispatch(context: &DispatchContext<TestTransport>, received: ReceivedApdu) {
    let source = received.source_mac.clone();
    let apdu = decode_apdu(received.apdu.clone()).unwrap();
    BACnetServer::dispatch(context, &source, apdu, received).await;
}
async fn settled(context: &DispatchContext<TestTransport>) {
    bounded(async {
        loop {
            context.request_tasks.reap_finished();
            if context.request_tasks.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
}

#[tokio::test]
async fn r7_deferred_read_and_overload_abort_keep_each_addressed_endpoint() {
    let mut routed = Routed::new().await;
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogInputObject::new(7, "input", EngineeringUnits::SQUARE_METERS).unwrap(),
    ))
    .unwrap();
    let config = ServerConfig {
        request_admission_policy: RequestAdmissionPolicy {
            max_confirmed_in_flight: 1,
            confirmed_recovery_reserve: 0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut context = context(db, config.clone());
    context.request_tasks =
        Arc::new(request_tasks::RequestTasks::new(config.request_admission_policy).unwrap());
    let held = context.services.db.write().await;
    let first = routed.receive(2000, &read(1), None).await;
    dispatch(&context, first).await;
    let second = routed.receive(3000, &read(2), None).await;
    dispatch(&context, second).await;
    assert!(
        matches!(routed.response(3000,2).await.0,Apdu::Abort(x) if x.abort_reason==AbortReason::OUT_OF_RESOURCES)
    );
    drop(held);
    assert!(matches!(
        routed.response(2000, 1).await.0,
        Apdu::ComplexAck(_)
    ));
    settled(&context).await;
    assert_eq!(context.request_tasks.counters().confirmed_active, 0);
    routed.router.stop().await;
}

#[tokio::test(start_paused = true)]
async fn r7_lso_replay_keeps_apdu_only_and_uses_current_der_or_router_context() {
    use bacnet_objects::life_safety::{LifeSafetyPointObject, LifeSafetyPointResetCommit};
    use bacnet_types::enums::{LifeSafetyOperation, LifeSafetyState};
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let mut point = LifeSafetyPointObject::new(1, "point").unwrap();
    point.set_present_value(LifeSafetyState::ALARM);
    point.set_operation_expected(LifeSafetyOperation::RESET);
    point.set_reset_executor(Arc::new(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        Ok(LifeSafetyPointResetCommit {
            present_value: Some(LifeSafetyState::QUIET),
            ..Default::default()
        })
    }));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(point)).unwrap();
    let context = context(
        db,
        ServerConfig {
            policy: Some(Arc::new(
                crate::FnPolicy::new().life_safety_operation(|_| true),
            )),
            ..Default::default()
        },
    );
    let mut payload = BytesMut::new();
    bacnet_services::life_safety::LifeSafetyOperationRequest {
        requesting_process_identifier: 41,
        requesting_source: "operator".into(),
        request: LifeSafetyOperation::RESET,
        object_identifier: Some(ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 1).unwrap()),
    }
    .encode(&mut payload)
    .unwrap();
    let request = request(
        ConfirmedServiceChoice::LIFE_SAFETY_OPERATION,
        payload.freeze(),
        3,
    );
    let mut routed = Routed::new().await;
    let (tx, rx) = oneshot::channel();
    let first = routed.receive(2000, &request, Some(tx)).await;
    dispatch(&context, first).await;
    let (ack, original) = decode_response(bounded(rx).await.unwrap(), 2000, 3);
    assert!(matches!(ack, Apdu::SimpleAck(_)));
    settled(&context).await;
    let (tx, rx) = oneshot::channel();
    let replay = routed.receive(3000, &request, Some(tx)).await;
    dispatch(&context, replay).await;
    let (_, replayed) = decode_response(bounded(rx).await.unwrap(), 3000, 3);
    assert_eq!(original, replayed);
    settled(&context).await;
    let (tx, rx) = oneshot::channel();
    drop(rx);
    let closed = routed.receive(3000, &request, Some(tx)).await;
    dispatch(&context, closed).await;
    settled(&context).await;
    assert!(
        routed.wire.try_recv().is_err(),
        "closed DER replay cannot create a second transport response"
    );
    let replay = routed.receive(3000, &request, None).await;
    dispatch(&context, replay).await;
    assert_eq!(routed.response(3000, 3).await.1, original);
    settled(&context).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    routed.router.stop().await;
}

#[path = "r7_endpoint_tests.rs"]
mod endpoint;
#[path = "r7_segment_tests.rs"]
mod segments;
