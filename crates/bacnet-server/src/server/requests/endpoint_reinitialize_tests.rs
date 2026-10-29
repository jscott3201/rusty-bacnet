use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::device_mgmt::ReinitializeDeviceRequest;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportPort, TransportProvenance};
use bacnet_types::enums::ReinitializedState;

type Calls = Arc<std::sync::Mutex<Vec<ReinitializeContext>>>;

/// A responder whose ReinitializeDevice handler records each context, then
/// returns what `outcome` does.
async fn fixture(
    outcome: impl Fn() -> Result<(), Error> + Send + Sync + 'static,
) -> (EndpointResponder, EndpointIngress<LoopbackTransport>, Calls) {
    let (responder, ingress, calls, _peer) = linked(None, move |_| outcome()).await;
    (responder, ingress, calls)
}

/// [`fixture`] with `password` required, the link's peer kept and `outcome`
/// given the context, so a test can register an after-reply hook and watch
/// the wire.
async fn linked(
    password: Option<&str>,
    outcome: impl Fn(&ReinitializeContext) -> Result<(), Error> + Send + Sync + 'static,
) -> (
    EndpointResponder,
    EndpointIngress<LoopbackTransport>,
    Calls,
    LoopbackTransport,
) {
    let (transport, peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut ingress = EndpointIngress::new(transport, 4);
    let receivers = ingress.start().await.unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(123, 0).expect("valid Device identity"),
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let calls = Calls::default();
    let recorded = Arc::clone(&calls);
    let handler: ReinitializeHandler = Arc::new(
        move |context: &ReinitializeContext, _db: &mut ObjectDatabase| {
            recorded.lock().unwrap().push(context.clone());
            outcome(context)
        },
    );
    let responder = EndpointResponder::new(Arc::new(RwLock::new(db)), receivers.egress)
        .with_reinitialize(handler, password.map(str::to_owned));
    (responder, ingress, calls, peer)
}

fn received(service_choice: ConfirmedServiceChoice, service_request: Bytes) -> ReceivedApdu {
    let mut bytes = BytesMut::new();
    let request = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 71,
        sequence_number: None,
        proposed_window_size: None,
        service_choice,
        service_request,
    };
    encode_apdu(&mut bytes, &Apdu::ConfirmedRequest(request)).unwrap();
    ReceivedApdu {
        router_response: None,
        direct_response: None,
        apdu: bytes.freeze(),
        source_mac: MacAddr::from_slice(&[2]),
        source_network: Some(NpduAddress {
            network: 77,
            mac_address: MacAddr::from_slice(&[44]),
        }),
        ingress_network: None,
        link_layer_group: false,
        is_group: false,
        global_broadcast: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn reinitialize(state: ReinitializedState) -> ReceivedApdu {
    let mut data = BytesMut::new();
    ReinitializeDeviceRequest {
        reinitialized_state: state,
        password: None,
    }
    .encode(&mut data)
    .unwrap();
    received(ConfirmedServiceChoice::REINITIALIZE_DEVICE, data.freeze())
}

async fn reply(responder: &EndpointResponder, mut received: ReceivedApdu) -> Apdu {
    let (tx, rx) = oneshot::channel();
    received.reply_tx = Some(tx);
    assert!(responder.handle(received).await.unwrap());
    decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap()
}

fn assert_services_other(response: Apdu) {
    let Apdu::Error(error) = response else {
        panic!("expected Error, got {response:?}")
    };
    assert_eq!(error.invoke_id, 71);
    assert_eq!(
        error.service_choice,
        ConfirmedServiceChoice::REINITIALIZE_DEVICE
    );
    assert_eq!(
        (error.error_class, error.error_code),
        (ErrorClass::SERVICES, ErrorCode::OTHER)
    );
}

/// The context comes from the received request: link source, routed source,
/// ingress provenance and invoke ID.
#[tokio::test]
async fn endpoint_reinitialize_context_names_the_requester() {
    let (responder, mut ingress, calls) = fixture(|| Ok(())).await;
    let response = reply(&responder, reinitialize(ReinitializedState::WARMSTART)).await;
    assert!(
        matches!(response, Apdu::SimpleAck(ref ack) if ack.invoke_id == 71),
        "got {response:?}"
    );
    let context = calls.lock().unwrap().pop().unwrap();
    assert_eq!(context.state, ReinitializedState::WARMSTART);
    assert_eq!(context.source_mac.as_slice(), &[2]);
    assert_eq!(
        context.source_network,
        Some(NpduAddress {
            network: 77,
            mac_address: MacAddr::from_slice(&[44]),
        })
    );
    assert_eq!(context.provenance, TransportProvenance::unverified());
    assert_eq!(context.invoke_id, 71);
    ingress.stop().await.unwrap();
}

/// A panicking handler is answered SERVICES / OTHER, and the responder
/// goes on answering the next request.
#[tokio::test]
async fn endpoint_reinitialize_panic_is_answered_and_the_responder_keeps_serving() {
    let (responder, mut ingress, calls) = fixture(|| panic!("reinitialize handler panic")).await;
    assert_services_other(reply(&responder, reinitialize(ReinitializedState::COLDSTART)).await);
    assert_eq!(calls.lock().unwrap().len(), 1);

    let mut read = BytesMut::new();
    bacnet_services::read_property::ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut read);
    let response = reply(
        &responder,
        received(ConfirmedServiceChoice::READ_PROPERTY, read.freeze()),
    )
    .await;
    assert!(
        matches!(response, Apdu::ComplexAck(ref ack) if ack.invoke_id == 71),
        "got {response:?}"
    );
    ingress.stop().await.unwrap();
}

/// Close may win while the request waits for the database owner. The
/// handler then never runs, and the request is refused, as a Device write
/// is in the same race.
#[tokio::test]
async fn endpoint_reinitialize_close_while_waiting_for_database_never_runs_the_handler() {
    let (responder, mut ingress, calls) = fixture(|| Ok(())).await;
    let held = responder.db().write().await;
    let mut received = reinitialize(ReinitializedState::WARMSTART);
    let (tx, rx) = oneshot::channel();
    received.reply_tx = Some(tx);
    let mut handle = Box::pin(responder.handle(received));
    // Poll to the database wait without timers or a racing spawned task.
    assert!(futures_util::poll!(&mut handle).is_pending());
    responder.close();
    drop(held);
    handle.await.unwrap();
    assert_services_other(decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap());
    assert!(calls.lock().unwrap().is_empty());
    ingress.stop().await.unwrap();
}

/// The link peer's receive half, set once the peer starts.
type Inbox = Arc<std::sync::Mutex<Option<tokio::sync::mpsc::Receiver<ReceivedNpdu>>>>;
/// What the peer had received each time an after-reply hook ran.
type Seen = Arc<std::sync::Mutex<Vec<Vec<Apdu>>>>;
/// How a test handler answers once it has registered its hook.
type Answer = fn() -> Result<(), Error>;

/// An outcome that registers a hook recording what the peer has received
/// by then, before answering with `answer`.
fn watching(
    inbox: &Inbox,
    seen: &Seen,
    answer: Answer,
) -> impl Fn(&ReinitializeContext) -> Result<(), Error> + Send + Sync + 'static + use<> {
    let (inbox, seen) = (Arc::clone(inbox), Arc::clone(seen));
    move |context| {
        let (inbox, seen) = (Arc::clone(&inbox), Arc::clone(&seen));
        context.after_reply(move || {
            let mut frames = Vec::new();
            if let Some(inbox) = inbox.lock().unwrap().as_mut() {
                while let Ok(frame) = inbox.try_recv() {
                    frames.push(decode_apdu(decode_npdu(frame.npdu).unwrap().payload).unwrap());
                }
            }
            seen.lock().unwrap().push(frames);
        });
        answer()
    }
}

/// The hook runs once the egress has handed the SimpleACK to the link, so
/// the peer already holds it, for a routed and a direct requester alike.
#[tokio::test]
async fn endpoint_after_reply_hook_runs_once_the_ack_is_sent() {
    for routed in [true, false] {
        let (inbox, seen) = (Inbox::default(), Seen::default());
        let (responder, mut ingress, _calls, mut peer) =
            linked(None, watching(&inbox, &seen, || Ok(()))).await;
        *inbox.lock().unwrap() = Some(peer.start().await.unwrap());
        let mut request = reinitialize(ReinitializedState::COLDSTART);
        if !routed {
            request.source_network = None;
        }
        assert!(responder.handle(request).await.unwrap());
        let runs = seen.lock().unwrap().clone();
        assert_eq!(runs.len(), 1, "routed={routed}");
        assert!(
            matches!(runs[0].as_slice(), [Apdu::SimpleAck(ack)] if ack.invoke_id == 71),
            "routed={routed}: the SimpleACK was sent before the hook ran: {runs:?}"
        );
        ingress.stop().await.unwrap();
    }
}

/// On a reply channel, the handoff to the channel is the send.
#[tokio::test]
async fn endpoint_after_reply_hook_follows_the_reply_channel_handoff() {
    let (tx, rx) = oneshot::channel();
    let rx = Arc::new(std::sync::Mutex::new(rx));
    let handed_off = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (seen, waiting) = (Arc::clone(&handed_off), Arc::clone(&rx));
    let (responder, mut ingress, _calls, _peer) = linked(None, move |context| {
        let (seen, waiting) = (Arc::clone(&seen), Arc::clone(&waiting));
        context.after_reply(move || {
            let reply = waiting.lock().unwrap().try_recv().is_ok();
            seen.lock().unwrap().push(reply);
        });
        Ok(())
    })
    .await;
    let mut request = reinitialize(ReinitializedState::WARMSTART);
    request.reply_tx = Some(tx);
    assert!(responder.handle(request).await.unwrap());
    assert_eq!(*handed_off.lock().unwrap(), [true]);
    ingress.stop().await.unwrap();
}

/// A refusal of any kind, or a reply the channel no longer takes, drops the
/// hook unrun.
#[tokio::test]
async fn endpoint_after_reply_hook_never_runs_for_a_refusal() {
    let refusal = || {
        Err(Error::protocol(
            ErrorClass::DEVICE,
            ErrorCode::CONFIGURATION_IN_PROGRESS,
        ))
    };
    let cases: [(&str, Option<&str>, Answer); 5] = [
        ("refusal", None, refusal),
        ("error", None, || Err(Error::Encoding("disk full".into()))),
        ("panic", None, || panic!("handler panic after registering")),
        ("password", Some("required"), || Ok(())),
        ("closed reply channel", None, || Ok(())),
    ];
    for (case, password, answer) in cases {
        let (inbox, seen) = (Inbox::default(), Seen::default());
        let (responder, mut ingress, _calls, _peer) =
            linked(password, watching(&inbox, &seen, answer)).await;
        let mut request = reinitialize(ReinitializedState::WARMSTART);
        if case == "closed reply channel" {
            let (tx, rx) = oneshot::channel();
            drop(rx);
            request.reply_tx = Some(tx);
            assert!(responder.handle(request).await.unwrap());
        } else {
            let response = reply(&responder, request).await;
            assert!(matches!(response, Apdu::Error(_)), "{case}: {response:?}");
        }
        assert!(seen.lock().unwrap().is_empty(), "{case}: hook ran");
        ingress.stop().await.unwrap();
    }
}
