//! Hold genuine outbound admission before network dispatch, then replace it.
use super::*;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::primitives::PropertyValue;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn outbound_admitted_write_keeps_identity_after_replacement_without_reply_retarget() {
    let ca = TestCa::new();
    let (mut peer, mut port) = Peer::start(&ca).await;
    let mut original = port.rx.take().unwrap();
    let (forward, rx) = mpsc::channel(4);
    port.rx = Some(rx);
    let (observed, mut observations) = mpsc::channel(2);
    let (release_a, wait_a) = tokio::sync::oneshot::channel();
    let (release_b, wait_b) = tokio::sync::oneshot::channel();
    // No synthetic provenance or route: only the scheduling boundary between
    // real transport admission and the unchanged NetworkLayer is controlled.
    let bridge = tokio::spawn(async move {
        let a = original.recv().await.unwrap();
        observed
            .send(a.provenance.direct_sc_identity().unwrap())
            .await
            .unwrap();
        let b = original.recv().await.unwrap();
        observed
            .send(b.provenance.direct_sc_identity().unwrap())
            .await
            .unwrap();
        wait_a.await.unwrap();
        forward.send(a).await.unwrap();
        wait_b.await.unwrap();
        forward.send(b).await.unwrap();
        while let Some(next) = original.recv().await {
            if forward.send(next).await.is_err() {
                break;
            }
        }
    });
    let authorized = Arc::new(Mutex::new(Vec::new()));
    let seen = authorized.clone();
    let config = bacnet_server::server::ServerConfig {
        policy: Some(Arc::new(bacnet_server::FnPolicy::new().mutation(
            move |context| {
                seen.lock()
                    .unwrap()
                    .push(context.direct_sc_identity().unwrap());
                true
            },
        ))),
        ..Default::default()
    };
    let mut server = bacnet_server::server::BACnetServer::start_clockless(
        config,
        database("held-outbound"),
        port,
    )
    .await
    .unwrap();
    let mut data = BytesMut::new();
    WritePropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: vec![0x72, 0, b'A'],
        priority: None,
    }
    .encode(&mut data)
    .unwrap();
    peer.send(&confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        90,
        data.freeze(),
    ))
    .await;
    let a = bounded(observations.recv()).await.unwrap();
    let b = bacnet_transport::sc_tls::TlsWebSocket::connect_direct(
        &format!(
            "wss://localhost:{}/.bacnet/sc",
            peer.local.local_addr().port()
        ),
        ca.tls("replacement"),
    )
    .await
    .unwrap();
    let mut connection = bacnet_transport::sc::ScConnection::new(REMOTE, [2; 16]);
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &connection.build_connect_request());
    b.send(&bytes).await.unwrap();
    assert_eq!(
        decode_sc_message(&b.recv().await.unwrap())
            .unwrap()
            .function,
        ScFunction::ConnectAccept
    );
    bytes.clear();
    encode_sc_message(
        &mut bytes,
        &connection
            .build_direct_encapsulated_npdu(&wire(&read_name(91)), &[])
            .unwrap(),
    );
    b.send(&bytes).await.unwrap();
    let identity_b = bounded(observations.recv()).await.unwrap();
    assert_ne!(a.leaf_sha256(), identity_b.leaf_sha256());
    assert_ne!(a.incarnation(), identity_b.incarnation());
    release_a.send(()).unwrap();
    bounded(async {
        loop {
            if server
                .read_local(&csv_oid(), PropertyIdentifier::DESCRIPTION, None)
                .await
                .unwrap()
                == PropertyValue::CharacterString("A".into())
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(*authorized.lock().unwrap(), vec![a]);
    release_b.send(()).unwrap();
    let reply = bounded(async {
        tokio::select! {
            direct=b.recv()=>direct.unwrap(),
            hub=peer.hub.recv()=>panic!("retired request escaped through Hub: {hub:?}"),
        }
    })
    .await;
    let reply = decode_apdu(
        decode_npdu(decode_sc_message(&reply).unwrap().payload)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert!(
        matches!(reply,Apdu::ComplexAck(ack) if ack.invoke_id==91),
        "B's first application reply must belong to B"
    );
    assert!(futures_util::poll!(Box::pin(peer.hub.recv()).as_mut()).is_pending());
    server.stop().await.unwrap();
    bridge.abort();
    let _ = bridge.await;
    peer.stop().await;
}

#[derive(Clone, Copy, Debug)]
enum Refusal {
    Error,
    Abort,
    Reject,
}

impl Refusal {
    fn request(self, invoke: u8) -> Apdu {
        use bacnet_services::read_property::ReadPropertyRequest;
        match self {
            Self::Error => {
                let mut data = BytesMut::new();
                ReadPropertyRequest {
                    object_identifier: csv_oid(),
                    property_identifier: PropertyIdentifier::from_raw(5555),
                    property_array_index: None,
                }
                .encode(&mut data);
                confirmed(ConfirmedServiceChoice::READ_PROPERTY, invoke, data.freeze())
            }
            Self::Abort => {
                let Apdu::ConfirmedRequest(mut request) = read_name(invoke) else {
                    unreachable!()
                };
                request.max_apdu_length = 50;
                Apdu::ConfirmedRequest(request)
            }
            Self::Reject => unsupported(invoke),
        }
    }

    fn assert_response(self, response: Apdu, invoke: u8) {
        use bacnet_types::enums::{AbortReason, ErrorClass, ErrorCode, RejectReason};
        let expected = match (&self, &response) {
            (Self::Error, Apdu::Error(error)) => {
                error.invoke_id == invoke
                    && error.service_choice == ConfirmedServiceChoice::READ_PROPERTY
                    && error.error_class == ErrorClass::PROPERTY
                    && error.error_code == ErrorCode::UNKNOWN_PROPERTY
            }
            (Self::Abort, Apdu::Abort(abort)) => {
                abort.invoke_id == invoke
                    && abort.sent_by_server
                    && abort.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED
            }
            (Self::Reject, Apdu::Reject(reject)) => {
                reject.invoke_id == invoke
                    && reject.reject_reason == RejectReason::UNRECOGNIZED_SERVICE
            }
            _ => false,
        };
        assert!(expected, "live {self:?} control returned {response:?}");
    }
}

fn admitted_identity(incoming: &ReceivedNpdu) -> bacnet_transport::port::DirectScIdentity {
    assert_eq!(incoming.source_mac.as_ref(), REMOTE);
    let identity = incoming.provenance.direct_sc_identity().unwrap();
    assert_eq!(
        incoming.direct_response.as_ref().unwrap().identity(),
        identity
    );
    identity
}

async fn outbound_endpoint_refusal_after_replacement(refusal: Refusal, same_leaf: bool) {
    let ca = TestCa::new();
    let (mut peer, mut port) = Peer::start(&ca).await;
    let ordinary_sends = port.ordinary_sends.clone();
    let mut original = port.rx.take().unwrap();
    let (forward, rx) = mpsc::channel(4);
    port.rx = Some(rx);
    let (observed, mut observations) = mpsc::channel(3);
    let (release_a, wait_a) = tokio::sync::oneshot::channel();
    let (release_b, wait_b) = tokio::sync::oneshot::channel();
    let bridge = tokio::spawn(async move {
        // The live control traverses the same real transport intake and
        // consumer. Later envelopes retain their original sealed routes.
        let live = original.recv().await.unwrap();
        observed.send(admitted_identity(&live)).await.unwrap();
        forward.send(live).await.unwrap();
        let a = original.recv().await.unwrap();
        observed.send(admitted_identity(&a)).await.unwrap();
        let b = original.recv().await.unwrap();
        observed.send(admitted_identity(&b)).await.unwrap();
        wait_a.await.unwrap();
        forward.send(a).await.unwrap();
        wait_b.await.unwrap();
        forward.send(b).await.unwrap();
        while let Some(next) = original.recv().await {
            if forward.send(next).await.is_err() {
                break;
            }
        }
    });
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        // A 50-octet requester cannot receive this name without segmentation.
        .with_database(database(&"x".repeat(200)));
    session.start().await.unwrap();
    let ordinary_before = ordinary_sends.load(Ordering::Acquire);
    peer.send(&refusal.request(80)).await;
    let live_identity = bounded(observations.recv()).await.unwrap();
    refusal.assert_response(peer.receive().await, 80);
    assert_eq!(session.policy_counters().await.responder_declined, 0);
    assert_eq!(ordinary_sends.load(Ordering::Acquire), ordinary_before);

    peer.send(&refusal.request(81)).await;
    let identity_a = bounded(observations.recv()).await.unwrap();
    assert_eq!(identity_a, live_identity);
    let b_tls = if same_leaf {
        peer.tls.clone()
    } else {
        ca.tls("replacement")
    };
    let b = bounded(bacnet_transport::sc_tls::TlsWebSocket::connect_direct(
        &format!(
            "wss://localhost:{}/.bacnet/sc",
            peer.local.local_addr().port()
        ),
        b_tls,
    ))
    .await
    .unwrap();
    let mut connection = bacnet_transport::sc::ScConnection::new(REMOTE, [2; 16]);
    let mut bytes = BytesMut::new();
    encode_sc_message(&mut bytes, &connection.build_connect_request());
    b.send(&bytes).await.unwrap();
    let accept = decode_sc_message(&bounded(b.recv()).await.unwrap()).unwrap();
    assert!(connection.handle_connect_accept(&accept));
    bytes.clear();
    encode_sc_message(
        &mut bytes,
        &connection
            .build_direct_encapsulated_npdu(&wire(&read_name(82)), &[])
            .unwrap(),
    );
    b.send(&bytes).await.unwrap();
    let identity_b = bounded(observations.recv()).await.unwrap();
    assert_eq!(
        identity_a.leaf_sha256() == identity_b.leaf_sha256(),
        same_leaf
    );
    assert_ne!(identity_a.incarnation(), identity_b.incarnation());

    release_a.send(()).unwrap();
    // The old response must be attempted and fail before B becomes a FIFO
    // observation barrier. An intake drop or unfinished task cannot pass.
    bounded(async {
        while session.policy_counters().await.responder_declined == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(session.policy_counters().await.responder_declined, 1);
    assert_eq!(ordinary_sends.load(Ordering::Acquire), ordinary_before);
    assert!(futures_util::poll!(Box::pin(peer.hub.recv()).as_mut()).is_pending());

    release_b.send(()).unwrap();
    let reply = bounded(async {
        tokio::select! {
            direct = b.recv() => direct.unwrap(),
            hub = peer.hub.recv() => panic!("retired response escaped through Hub: {hub:?}"),
        }
    })
    .await;
    let frame = decode_sc_message(&reply).unwrap();
    assert_eq!(frame.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(frame.originating_vmac, None);
    assert_eq!(frame.destination_vmac, None);
    let reply = decode_apdu(decode_npdu(frame.payload).unwrap().payload).unwrap();
    let Apdu::ComplexAck(ack) = reply else {
        panic!("B's first application reply must answer B: {reply:?}");
    };
    assert_eq!(ack.invoke_id, 82);
    assert_eq!(ack.service_choice, ConfirmedServiceChoice::READ_PROPERTY);
    let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.object_identifier, csv_oid());
    assert_eq!(ack.property_identifier, PropertyIdentifier::OBJECT_NAME);
    assert_eq!(session.policy_counters().await.responder_declined, 1);
    assert_eq!(ordinary_sends.load(Ordering::Acquire), ordinary_before);
    assert!(futures_util::poll!(Box::pin(peer.hub.recv()).as_mut()).is_pending());
    session.stop().await.unwrap();
    bridge.abort();
    if let Err(error) = bridge.await {
        assert!(error.is_cancelled(), "intake bridge failed: {error}");
    }
    peer.stop().await;
}

#[tokio::test]
async fn outbound_endpoint_error_never_reaches_new_leaf_replacement() {
    outbound_endpoint_refusal_after_replacement(Refusal::Error, false).await;
}

#[tokio::test]
async fn outbound_endpoint_error_never_reaches_same_leaf_reconnect() {
    outbound_endpoint_refusal_after_replacement(Refusal::Error, true).await;
}

#[tokio::test]
async fn outbound_endpoint_abort_never_reaches_new_leaf_replacement() {
    outbound_endpoint_refusal_after_replacement(Refusal::Abort, false).await;
}

#[tokio::test]
async fn outbound_endpoint_abort_never_reaches_same_leaf_reconnect() {
    outbound_endpoint_refusal_after_replacement(Refusal::Abort, true).await;
}

#[tokio::test]
async fn outbound_endpoint_reject_never_reaches_new_leaf_replacement() {
    outbound_endpoint_refusal_after_replacement(Refusal::Reject, false).await;
}

#[tokio::test]
async fn outbound_endpoint_reject_never_reaches_same_leaf_reconnect() {
    outbound_endpoint_refusal_after_replacement(Refusal::Reject, true).await;
}
