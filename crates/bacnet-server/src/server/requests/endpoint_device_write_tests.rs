use super::*;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportProvenance;
use std::sync::atomic::AtomicUsize;

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, 123).unwrap()
}

fn write() -> WritePropertyRequest {
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value, "updated").unwrap();
    WritePropertyRequest {
        object_identifier: device(),
        property_identifier: PropertyIdentifier::DESCRIPTION,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
}

fn request(write: &WritePropertyRequest) -> ConfirmedRequestPdu {
    let mut bytes = BytesMut::new();
    write.encode(&mut bytes);
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 71,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: bytes.freeze(),
    }
}

fn received(request: ConfirmedRequestPdu) -> ReceivedApdu {
    let mut bytes = BytesMut::new();
    encode_apdu(&mut bytes, &Apdu::ConfirmedRequest(request)).unwrap();
    ReceivedApdu {
        apdu: bytes.freeze(),
        source_mac: MacAddr::from_slice(&[2]),
        source_network: Some(NpduAddress {
            network: 77,
            mac_address: MacAddr::from_slice(&[44]),
        }),
        ingress_network: None,
        link_layer_group: false,
        is_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

async fn fixture(
    authorizer: Option<MutationAuthorizer>,
) -> (EndpointResponder, EndpointIngress<LoopbackTransport>) {
    let (transport, _peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut ingress = EndpointIngress::new(transport, 4);
    let receivers = ingress.start().await.unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 123,
            ..Default::default()
        })
        .unwrap(),
    ))
    .unwrap();
    let mut responder = EndpointResponder::new(Arc::new(RwLock::new(db)), receivers.egress);
    if let Some(authorizer) = authorizer {
        responder = responder.with_device_writes(device(), authorizer);
    }
    (responder, ingress)
}

async fn reply(
    responder: &EndpointResponder,
    mut received: ReceivedApdu,
) -> (Apdu, Option<NpduAddress>) {
    let (tx, rx) = oneshot::channel();
    received.reply_tx = Some(tx);
    assert!(responder.handle(received).await.unwrap());
    let npdu = decode_npdu(rx.await.unwrap()).unwrap();
    (decode_apdu(npdu.payload).unwrap(), npdu.destination)
}

async fn description(responder: &EndpointResponder) -> PropertyValue {
    responder
        .db
        .read()
        .await
        .get(&device())
        .unwrap()
        .read_property(PropertyIdentifier::DESCRIPTION, None)
        .unwrap()
}

fn assert_error(response: Apdu, class: ErrorClass, code: ErrorCode) {
    let Apdu::Error(error) = response else {
        panic!("expected Error, got {response:?}")
    };
    assert_eq!(error.invoke_id, 71);
    assert_eq!(error.service_choice, ConfirmedServiceChoice::WRITE_PROPERTY);
    assert_eq!((error.error_class, error.error_code), (class, code));
}

#[tokio::test]
async fn endpoint_device_write_authorizes_exact_context_and_preserves_reply_tx() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = seen.clone();
    let (responder, mut ingress) = fixture(Some(Arc::new(move |context| {
        captured.lock().unwrap().push(context.clone());
        true
    })))
    .await;
    for priority in [None, Some(1), Some(16)] {
        let mut write = write();
        write.priority = priority;
        let received = received(request(&write));
        let expected_source = received.source_network.clone();
        let (response, destination) = reply(&responder, received).await;
        assert!(
            matches!(response, Apdu::SimpleAck(ack) if ack.invoke_id == 71 && ack.service_choice == ConfirmedServiceChoice::WRITE_PROPERTY)
        );
        assert_eq!(destination, expected_source);
        assert_eq!(
            description(&responder).await,
            PropertyValue::CharacterString("updated".into())
        );
        let context = seen.lock().unwrap().last().unwrap().clone();
        assert_eq!(context.source_mac.as_slice(), &[2]);
        assert_eq!(context.source_network, expected_source);
        assert_eq!(context.provenance, TransportProvenance::unverified());
        assert_eq!(context.trust, MutationTrust::Unverified);
        assert_eq!(context.invoke_id, 71);
        assert_eq!(
            context.service_choice,
            ConfirmedServiceChoice::WRITE_PROPERTY
        );
        assert_eq!(context.target, MutationTarget::WriteProperty(write));
    }
    responder.close();
    assert!(responder.handle(received(request(&write()))).await.is_err());
    ingress.stop().await.unwrap();
}

#[tokio::test]
async fn endpoint_device_write_default_refusal_and_panic_never_mutate() {
    for mode in 0..3 {
        let authorizer: Option<MutationAuthorizer> = match mode {
            0 => None,
            1 => Some(Arc::new(|_| false)),
            _ => Some(Arc::new(|_| panic!("policy panic"))),
        };
        let (responder, mut ingress) = fixture(authorizer).await;
        let (response, _) = reply(&responder, received(request(&write()))).await;
        if mode == 0 {
            assert!(
                matches!(response, Apdu::Reject(reject) if reject.invoke_id == 71 && reject.reject_reason == RejectReason::UNRECOGNIZED_SERVICE)
            );
        } else {
            assert_error(
                response,
                ErrorClass::SERVICES,
                ErrorCode::SERVICE_REQUEST_DENIED,
            );
        }
        assert_eq!(
            description(&responder).await,
            PropertyValue::CharacterString(String::new())
        );
        ingress.stop().await.unwrap();
    }
}

#[tokio::test]
async fn endpoint_device_write_rejects_scope_index_type_priority_and_malformed_before_policy() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let (responder, mut ingress) = fixture(Some(Arc::new(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        true
    })))
    .await;
    let mut cases = Vec::new();
    for oid in [
        ObjectIdentifier::new(ObjectType::DEVICE, 456).unwrap(),
        ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap(),
    ] {
        let mut write = write();
        write.object_identifier = oid;
        cases.push((
            request(&write),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ));
    }
    for property in [
        PropertyIdentifier::OBJECT_NAME,
        PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
    ] {
        let mut write = write();
        write.property_identifier = property;
        cases.push((
            request(&write),
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ));
    }
    for index in [0, 1] {
        let mut write = write();
        write.property_array_index = Some(index);
        cases.push((
            request(&write),
            ErrorClass::PROPERTY,
            ErrorCode::PROPERTY_IS_NOT_AN_ARRAY,
        ));
    }
    for value in [
        vec![0x00],
        vec![0x21, 0x01],
        [write().property_value.as_slice(), &[0x00]].concat(),
    ] {
        let mut write = write();
        write.property_value = value;
        cases.push((
            request(&write),
            ErrorClass::PROPERTY,
            ErrorCode::INVALID_DATA_TYPE,
        ));
    }
    let mut empty = write();
    empty.property_value.clear();
    cases.push((
        request(&empty),
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_ENCODING,
    ));
    for priority in [0, 17] {
        let mut write = write();
        write.priority = Some(priority);
        cases.push((request(&write), ErrorClass::SERVICES, ErrorCode::OTHER));
    }
    let mut malformed = request(&write());
    malformed.service_request = Bytes::from_static(&[0x0c]);
    cases.push((malformed, ErrorClass::SERVICES, ErrorCode::OTHER));
    for (request, class, code) in cases {
        assert_error(reply(&responder, received(request)).await.0, class, code);
        assert_eq!(
            description(&responder).await,
            PropertyValue::CharacterString(String::new())
        );
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    ingress.stop().await.unwrap();
}

#[tokio::test]
async fn endpoint_device_write_group_silence_segment_abort_and_wpm_reject() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = calls.clone();
    let (responder, mut ingress) = fixture(Some(Arc::new(move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        true
    })))
    .await;
    let mut group = received(request(&write()));
    group.is_group = true;
    let (tx, rx) = oneshot::channel();
    group.reply_tx = Some(tx);
    assert!(!responder.handle(group).await.unwrap());
    assert!(rx.await.is_err());
    let mut segmented = request(&write());
    segmented.segmented = true;
    segmented.sequence_number = Some(0);
    segmented.proposed_window_size = Some(1);
    assert!(
        matches!(reply(&responder, received(segmented)).await.0, Apdu::Abort(abort) if abort.invoke_id == 71 && abort.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED)
    );
    let mut wpm = request(&write());
    wpm.service_choice = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    assert!(
        matches!(reply(&responder, received(wpm)).await.0, Apdu::Reject(reject) if reject.reject_reason == RejectReason::UNRECOGNIZED_SERVICE)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        description(&responder).await,
        PropertyValue::CharacterString(String::new())
    );
    ingress.stop().await.unwrap();
}

#[tokio::test]
async fn endpoint_device_write_direct_and_routed_egress_preserve_destination() {
    for routed in [false, true] {
        let (transport, mut peer) = LoopbackTransport::pair(vec![1], vec![2]);
        let mut rx = peer.start().await.unwrap();
        let mut ingress = EndpointIngress::new(transport, 4);
        let receivers = ingress.start().await.unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            DeviceObject::new(DeviceConfig {
                instance: 123,
                ..Default::default()
            })
            .unwrap(),
        ))
        .unwrap();
        let responder = EndpointResponder::new(Arc::new(RwLock::new(db)), receivers.egress)
            .with_device_writes(device(), Arc::new(|_| true));
        let mut received = received(request(&write()));
        if !routed {
            received.source_network = None;
        }
        let destination = received.source_network.clone();
        assert!(responder.handle(received).await.unwrap());
        let sent = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .unwrap()
            .unwrap();
        let npdu = decode_npdu(sent.npdu).unwrap();
        assert_eq!(npdu.destination, destination);
        assert!(!npdu.expecting_reply);
        assert_eq!(npdu.priority, NetworkPriority::NORMAL);
        assert!(
            matches!(decode_apdu(npdu.payload).unwrap(),Apdu::SimpleAck(ack) if ack.invoke_id == 71)
        );
        ingress.stop().await.unwrap();
        peer.stop().await.unwrap();
    }
}

#[tokio::test]
async fn endpoint_device_write_shutdown_while_waiting_for_database_never_commits() {
    let (responder, mut ingress) = fixture(Some(Arc::new(|_| true))).await;
    let held = responder.db.write().await;
    let mut received = received(request(&write()));
    let (tx, rx) = oneshot::channel();
    received.reply_tx = Some(tx);
    let mut handle = Box::pin(responder.handle(received));
    // Poll to the database wait without timers or a racing spawned task.
    assert!(futures_util::poll!(&mut handle).is_pending());
    responder.close();
    drop(held);
    handle.await.unwrap();
    assert!(matches!(
        decode_apdu(decode_npdu(rx.await.unwrap()).unwrap().payload).unwrap(),
        Apdu::Error(_)
    ));
    assert_eq!(
        description(&responder).await,
        PropertyValue::CharacterString(String::new())
    );
    ingress.stop().await.unwrap();
}
