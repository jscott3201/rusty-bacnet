//! Original accepted connection replies; fallback observations are not proof
//! of delivery to another peer. Every ingress envelope comes from real TLS.
use super::*;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_transport::sc_frame::ScFunction;

fn read_name(invoke: u8) -> Apdu {
    let mut data = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::OBJECT_NAME,
        property_array_index: None,
    }
    .encode(&mut data);
    confirmed(ConfirmedServiceChoice::READ_PROPERTY, invoke, data.freeze())
}

async fn socket_response(f: &mut Fixture, peer: &Peer) -> Apdu {
    let wire = bounded(async {
        tokio::select! {
            reply = peer.ws.recv() => reply.unwrap(),
            fallback = f.responses.recv() => panic!("direct reply used address-based fallback: {fallback:?}"),
        }
    })
    .await;
    let frame = decode_sc_message(&wire).unwrap();
    assert_eq!(frame.function, ScFunction::EncapsulatedNpdu);
    assert_eq!(frame.originating_vmac, None);
    assert_eq!(frame.destination_vmac, None);
    let npdu = decode_npdu(frame.payload).unwrap();
    let destination = npdu.destination.unwrap();
    assert_eq!(destination.network, 123);
    assert_eq!(destination.mac_address.as_ref(), &[3]);
    apdu::decode_apdu(npdu.payload).unwrap()
}

fn assert_name(response: Apdu, invoke: u8) {
    let Apdu::ComplexAck(ack) = response else {
        panic!("expected data-bearing ReadProperty response");
    };
    assert_eq!(ack.invoke_id, invoke);
    let ack = ReadPropertyACK::decode(&ack.service_ack).unwrap();
    assert_eq!(ack.object_identifier, csv_oid());
    assert_eq!(ack.property_identifier, PropertyIdentifier::OBJECT_NAME);
    let mut expected = BytesMut::new();
    encode_property_value(
        &mut expected,
        &PropertyValue::CharacterString("value".into()),
    )
    .unwrap();
    assert_eq!(ack.property_value, expected.to_vec());
}

#[tokio::test]
async fn direct_response_same_socket_read_property() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let envelope = f.capture(&mut a, &read_name(41)).await;
    f.feed(envelope).await;
    assert_name(socket_response(&mut f, &a).await, 41);
    f.active(0).await;
    assert!(f.responses.try_recv().is_err());
    f.stop().await;
}

#[tokio::test]
async fn direct_response_held_read_replacement_has_no_address_fallback() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let envelope = f.capture(&mut a, &read_name(41)).await;
    let identity_a = envelope.provenance.direct_sc_identity().unwrap();
    let db = f.server.db.clone();
    let held = db.write().await;
    f.feed(envelope).await;
    f.active(1).await;

    let mut b = f.peer(ca.tls("b")).await;
    let b_request = f.capture(&mut b, &read_name(42)).await;
    let identity_b = b_request.provenance.direct_sc_identity().unwrap();
    assert_ne!(identity_a.leaf_sha256(), identity_b.leaf_sha256());
    assert_ne!(identity_a.incarnation(), identity_b.incarnation());
    drop(held);
    f.active(0).await;
    assert!(
        f.responses.try_recv().is_err(),
        "retired A reply reached address-based egress"
    );

    // FIFO socket observation after A's task completes: B's first application
    // frame must answer B, never the held A transaction.
    f.feed(b_request).await;
    assert_name(socket_response(&mut f, &b).await, 42);
    f.stop().await;
}

/// A's request is held behind the database lock while B, another CA-valid
/// leaf claiming the same VMAC and UUID, replaces A. A's reply, whatever it
/// is, must reach neither B's socket nor address-based egress (#524).
/// `refused` is the warning the server logs when it builds A's reply and the
/// original connection refuses it, which shows the reply was reached.
async fn held_reply_never_reaches_replacement(request_a: Apdu, long_value: bool, refused: &str) {
    let (spy, _guard) = super::warn_spy::WarnSpy::install();
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    if long_value {
        // Too long for a 50-octet APDU, for the Abort case.
        f.server
            .db
            .write()
            .await
            .get_mut(&csv_oid())
            .unwrap()
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::CharacterString("x".repeat(200)),
                None,
            )
            .unwrap();
    }
    let mut a = f.peer(ca.tls("a")).await;
    let envelope = f.capture(&mut a, &request_a).await;
    let db = f.server.db.clone();
    let held = db.write().await;
    f.feed(envelope).await;
    f.active(1).await;

    let mut b = f.peer(ca.tls("b")).await;
    let b_request = f.capture(&mut b, &read_name(42)).await;
    drop(held);
    f.active(0).await;
    assert!(
        f.responses.try_recv().is_err(),
        "retired A reply reached address-based egress"
    );
    let warnings = spy.warnings();
    assert!(
        warnings.iter().any(|warning| warning.message == refused
            && warning
                .error
                .contains("original direct response connection unavailable")),
        "A's reply was never built and refused: {warnings:?}"
    );
    // B's first application frame answers B, never A's held transaction.
    f.feed(b_request).await;
    assert_name(socket_response(&mut f, &b).await, 42);
    f.stop().await;
}

#[tokio::test]
async fn direct_response_held_error_never_reaches_a_replacement() {
    let mut missing = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::from_raw(5555),
        property_array_index: None,
    }
    .encode(&mut missing);
    held_reply_never_reaches_replacement(
        confirmed(ConfirmedServiceChoice::READ_PROPERTY, 41, missing.freeze()),
        false,
        "Failed to send response",
    )
    .await;
}

#[tokio::test]
async fn direct_response_held_abort_never_reaches_a_replacement() {
    let mut encoded = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut encoded);
    let Apdu::ConfirmedRequest(mut request) =
        confirmed(ConfirmedServiceChoice::READ_PROPERTY, 41, encoded.freeze())
    else {
        unreachable!()
    };
    // Too small for the reply, with no segmentation: SEGMENTATION_NOT_SUPPORTED.
    request.max_apdu_length = 50;
    held_reply_never_reaches_replacement(
        Apdu::ConfirmedRequest(request),
        true,
        "Failed to send Abort for segmentation-not-supported",
    )
    .await;
}

async fn held_reject_never_reaches_replacement(same_leaf: bool) {
    let (spy, _guard) = super::warn_spy::WarnSpy::install();
    let ca = TestCa::new();
    let a_tls = ca.tls("a");
    let b_tls = if same_leaf {
        a_tls.clone()
    } else {
        ca.tls("b")
    };
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(a_tls).await;
    // A valid outer APDU reaches ReadProperty's decoder, whose database read
    // is the existing asynchronous gate before it rejects the missing fields.
    let malformed = |invoke| confirmed(ConfirmedServiceChoice::READ_PROPERTY, invoke, Bytes::new());
    let live = f.capture(&mut a, &malformed(40)).await;
    let identity_a = live.provenance.direct_sc_identity().unwrap();
    f.feed(live).await;
    assert!(matches!(socket_response(&mut f, &a).await,
        Apdu::Reject(reject) if reject.invoke_id == 40
            && reject.reject_reason == RejectReason::MISSING_REQUIRED_PARAMETER));
    f.active(0).await;

    let envelope = f.capture(&mut a, &malformed(41)).await;
    assert_eq!(envelope.provenance.direct_sc_identity(), Some(identity_a));
    assert_eq!(
        envelope.direct_response.as_ref().unwrap().identity(),
        identity_a
    );
    let admitted_before = f.server.request_tasks.counters().confirmed_admitted_total;
    let db = f.server.db.clone();
    let held = db.write().await;
    f.feed(envelope).await;
    f.active(1).await;
    assert_eq!(
        f.server.request_tasks.counters().confirmed_admitted_total,
        admitted_before + 1
    );

    let mut b = f.peer(b_tls).await;
    let b_request = f.capture(&mut b, &read_name(42)).await;
    let identity_b = b_request.provenance.direct_sc_identity().unwrap();
    assert_eq!(
        identity_a.leaf_sha256() == identity_b.leaf_sha256(),
        same_leaf
    );
    assert_ne!(identity_a.incarnation(), identity_b.incarnation());
    assert_eq!(
        b_request.direct_response.as_ref().unwrap().identity(),
        identity_b
    );
    drop(held);
    f.active(0).await;
    let warnings = spy.warnings();
    assert_eq!(
        warnings
            .iter()
            .filter(|warning| warning.message == "Failed to send response"
                && warning
                    .error
                    .contains("original direct response connection unavailable"))
            .count(),
        1,
        "the held Reject must be built and refused: {warnings:?}"
    );
    assert!(
        f.responses.try_recv().is_err(),
        "retired Reject used generic egress"
    );

    // Old work has completed before B's FIFO observation barrier. Its first
    // application reply must be its own data, never A's retired Reject.
    f.feed(b_request).await;
    assert_name(socket_response(&mut f, &b).await, 42);
    f.active(0).await;
    assert!(f.responses.try_recv().is_err(), "reply used generic egress");
    f.stop().await;
}

#[tokio::test]
async fn direct_response_held_reject_never_reaches_new_leaf_replacement() {
    held_reject_never_reaches_replacement(false).await;
}

#[tokio::test]
async fn direct_response_held_reject_never_reaches_same_leaf_reconnect() {
    held_reject_never_reaches_replacement(true).await;
}

#[tokio::test]
async fn direct_response_success_error_reject_and_oversize_abort_use_original_socket() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let write = confirmed(
        ConfirmedServiceChoice::WRITE_PROPERTY,
        1,
        write_payload(&"x".repeat(200)),
    );
    let envelope = f.capture(&mut a, &write).await;
    f.feed(envelope).await;
    assert!(
        matches!(socket_response(&mut f, &a).await, Apdu::SimpleAck(ack) if ack.invoke_id == 1)
    );

    let mut missing = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::from_raw(5555),
        property_array_index: None,
    }
    .encode(&mut missing);
    let invalid = confirmed(ConfirmedServiceChoice::READ_PROPERTY, 2, missing.freeze());
    let envelope = f.capture(&mut a, &invalid).await;
    f.feed(envelope).await;
    assert!(
        matches!(socket_response(&mut f, &a).await, Apdu::Error(error) if error.invoke_id == 2 && error.error_class == ErrorClass::PROPERTY && error.error_code == ErrorCode::UNKNOWN_PROPERTY)
    );
    let unknown = confirmed(ConfirmedServiceChoice::from_raw(254), 3, Bytes::new());
    let envelope = f.capture(&mut a, &unknown).await;
    f.feed(envelope).await;
    assert!(matches!(socket_response(&mut f, &a).await, Apdu::Reject(ack) if ack.invoke_id == 3));

    let mut encoded = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: csv_oid(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut encoded);
    let Apdu::ConfirmedRequest(mut request) =
        confirmed(ConfirmedServiceChoice::READ_PROPERTY, 4, encoded.freeze())
    else {
        unreachable!()
    };
    request.max_apdu_length = 50;
    let envelope = f.capture(&mut a, &Apdu::ConfirmedRequest(request)).await;
    f.feed(envelope).await;
    assert!(
        matches!(socket_response(&mut f, &a).await, Apdu::Abort(ack) if ack.invoke_id == 4 && ack.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED)
    );
    f.stop().await;
}

#[tokio::test]
async fn direct_response_missing_and_mismatched_capabilities_never_fall_back() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig::default(),
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let mut missing = f.capture(&mut a, &read_name(1)).await;
    missing.direct_response = None;
    f.feed(missing).await;
    f.dispatch_barrier(&mut a, 240).await;
    f.active(0).await;
    assert!(f.responses.try_recv().is_err());

    let mut old = f.capture(&mut a, &read_name(2)).await;
    let mut b = f.peer(ca.tls("b")).await;
    let new = f.capture(&mut b, &read_name(3)).await;
    old.direct_response = new.direct_response.clone();
    f.feed(old).await;
    f.dispatch_barrier(&mut b, 241).await;
    f.active(0).await;
    assert!(f.responses.try_recv().is_err());
    f.feed(new).await;
    assert_name(socket_response(&mut f, &b).await, 3);
    f.stop().await;
}

#[tokio::test]
async fn direct_response_lso_replay_and_same_leaf_reconnect_keep_original_route() {
    let ca = TestCa::new();
    let tls = ca.tls("a");
    let authorized = Arc::new(AtomicUsize::new(0));
    let observed = authorized.clone();
    let executions = Arc::new(AtomicUsize::new(0));
    let mut f = Fixture::new(
        &ca,
        ServerConfig {
            policy: Some(Arc::new(crate::FnPolicy::new().life_safety_operation(
                move |_| {
                    observed.fetch_add(1, Ordering::AcqRel);
                    true
                },
            ))),
            ..Default::default()
        },
        database(executions.clone()),
    )
    .await;
    let mut a = f.peer(tls.clone()).await;
    let request = request(true, 42);
    let envelope = f.capture(&mut a, &request).await;
    f.feed(envelope).await;
    let first = socket_response(&mut f, &a).await;
    assert!(matches!(first, Apdu::SimpleAck(_)));
    let replay = f.capture(&mut a, &request).await;
    f.feed(replay).await;
    assert_eq!(socket_response(&mut f, &a).await, first);
    assert_eq!(authorized.load(Ordering::Acquire), 1);
    assert_eq!(executions.load(Ordering::Acquire), 1);
    let old_replay = f.capture(&mut a, &request).await;
    let mut b = f.peer(tls).await;
    let new = f.capture(&mut b, &request).await;
    assert_ne!(old_replay.provenance, new.provenance);
    f.feed(old_replay).await;
    f.dispatch_barrier(&mut b, 240).await;
    assert_eq!(authorized.load(Ordering::Acquire), 1);
    assert!(f.responses.try_recv().is_err());
    f.feed(new).await;
    // RESET completed on A and cleared Operation_Expected. A fresh B
    // incarnation reauthorizes and observes current state instead of replaying
    // A's successful ACK.
    assert!(matches!(socket_response(&mut f, &b).await,
        Apdu::Error(error) if error.error_code == ErrorCode::INVALID_OPERATION_IN_THIS_STATE));
    assert_eq!(authorized.load(Ordering::Acquire), 2);
    assert_eq!(executions.load(Ordering::Acquire), 1);
    f.stop().await;
}

#[tokio::test]
async fn direct_response_overload_abort_uses_current_original_socket() {
    let ca = TestCa::new();
    let mut f = Fixture::new(
        &ca,
        ServerConfig {
            request_admission_policy: RequestAdmissionPolicy {
                max_confirmed_in_flight: 1,
                confirmed_recovery_reserve: 0,
                ..Default::default()
            },
            ..Default::default()
        },
        database(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let mut a = f.peer(ca.tls("a")).await;
    let db = f.server.db.clone();
    let held = db.write().await;
    let first = f.capture(&mut a, &read_name(1)).await;
    f.feed(first).await;
    f.active(1).await;
    let overload = f.capture(&mut a, &read_name(2)).await;
    f.feed(overload).await;
    assert!(
        matches!(socket_response(&mut f, &a).await, Apdu::Abort(a) if a.invoke_id == 2 && a.abort_reason == AbortReason::OUT_OF_RESOURCES)
    );
    drop(held);
    assert_name(socket_response(&mut f, &a).await, 1);
    f.stop().await;
}

#[path = "direct_response_segment_tests.rs"]
mod segments;

#[path = "direct_response_owner_tests.rs"]
mod owners;

#[path = "direct_response_budget_tests.rs"]
mod budgets;
