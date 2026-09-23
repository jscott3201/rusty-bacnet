//! RB-10 entry-path coverage for mutation authorization.
//!
//! In-memory, deterministic, no sleeps. Pins DenyAll dominance over an
//! allow-all authorizer; overload abort before mutation with zero side
//! effects; the read-only shared endpoint rejecting every mutation choice;
//! read-only and discovery controls unaffected by DenyAll; and direct-SC
//! versus hub-mediated-unknown through one verified-only gate.

use super::endpoint_responder::EndpointResponder;
use super::mutation_tests::{
    apdu, assert_denied, cases, oid, route, Fixture, TestTransport, SOURCE,
};
use super::*;
use crate::mutation::{
    MutationAuthorizationContext, MutationAuthorizer, MutationDecisionCounters, MutationPolicy,
    MutationTrust,
};
use crate::server::request_admission::{Class, RequestAdmissionPolicy};
use crate::server::request_peer::canonical_requester;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_endpoint_core::endpoint_ingress::EndpointIngress;
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_services::enrollment_summary::GetEnrollmentSummaryRequest;
use bacnet_services::file::{AtomicReadFileRequest, FileAccessMethod};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::read_range::ReadRangeRequest;
use bacnet_services::rpm::{ReadAccessSpecification, ReadPropertyMultipleRequest};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::EnableDisable;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex as StdMutex;

fn confirmed(service: ConfirmedServiceChoice, bytes: Bytes, id: u8) -> ConfirmedRequestPdu {
    ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: service,
        service_request: bytes,
    }
}

/// Drive one admitted request with an explicit ingress snapshot, bypassing
/// admission/duplicates exactly like `Fixture::dispatch` but threading
/// `provenance` into the mutation gate.
async fn dispatch_admitted(
    fixture: &Fixture,
    decisions: &Arc<crate::mutation::MutationDecisions>,
    service: ConfirmedServiceChoice,
    bytes: Bytes,
    id: u8,
    provenance: TransportProvenance,
) -> Option<Bytes> {
    let (tx, rx) = oneshot::channel();
    BACnetServer::<TestTransport>::handle_admitted_confirmed_request(
        &fixture.db,
        &fixture.network,
        &fixture.table,
        &Arc::new(segmented_send::SegmentedSendRegistry::default()),
        &Arc::new(Semaphore::new(MAX_SEG_SENDERS)),
        &Arc::new(Semaphore::new(1)),
        &Arc::new(Mutex::new(ServerTsm::new())),
        &NotificationTransactions::new(),
        &Arc::new(RwLock::new(DeviceBindingTable::new())),
        &fixture.state,
        &Arc::new(Mutex::new(Default::default())),
        &Arc::new(dcc_outcomes::DccOutcomes::default()),
        decisions,
        &fixture.config,
        &Arc::new(crate::server::request_tasks::RequestTasks::default()).spawner(),
        SOURCE,
        route(),
        provenance,
        confirmed(service, bytes, id),
        Some(tx),
    )
    .await;
    rx.await.ok()
}

fn capturing(
    authorizer: impl Fn(&MutationAuthorizationContext) -> bool + Send + Sync + 'static,
) -> (
    Option<MutationAuthorizer>,
    Arc<StdMutex<Vec<MutationAuthorizationContext>>>,
) {
    let seen = Arc::new(StdMutex::new(Vec::new()));
    let captured = seen.clone();
    let gate: MutationAuthorizer = Arc::new(move |context| {
        captured.lock().unwrap().push(context.clone());
        authorizer(context)
    });
    (Some(gate), seen)
}

#[tokio::test]
async fn deny_all_dominates_allow_all_without_invoking_callback() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut fixture = Fixture::new(Some(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        true
    })));
    fixture.config.mutation_policy = MutationPolicy::DenyAll;
    let decisions = Arc::new(crate::mutation::MutationDecisions::default());
    for (service, bytes, _) in cases() {
        let before = fixture
            .db
            .read()
            .await
            .get(&oid(ObjectType::BINARY_VALUE, 1))
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        let response = dispatch_admitted(
            &fixture,
            &decisions,
            service,
            bytes,
            17,
            TransportProvenance::unverified(),
        )
        .await
        .unwrap();
        if service == ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE {
            // WPM denial keeps its first-failed error shape, not a bare denial.
            let Apdu::Error(error) = apdu(response) else {
                panic!("expected WPM first-failed Error")
            };
            assert_eq!(error.invoke_id, 17);
            let detail =
                bacnet_services::wpm::WritePropertyMultipleError::from_error_pdu(&error).unwrap();
            assert_eq!(
                detail.first_failed_write_attempt.object_identifier,
                oid(ObjectType::BINARY_VALUE, 1)
            );
        } else {
            assert_denied(response, service, 17);
        }
        let after = fixture
            .db
            .read()
            .await
            .get(&oid(ObjectType::BINARY_VALUE, 1))
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(before, after);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let counters = decisions.snapshot();
    assert_eq!(counters.write_property.policy_deny_total, 1);
    assert_eq!(
        counters.subscribe_cov_property_multiple.policy_deny_total,
        1
    );
}

/// Overload aborts before decoding, authorization, or mutation: no authorizer
/// call, no decision counter, no database change, no fan-out.
#[tokio::test]
async fn overload_abort_precedes_mutation_with_zero_side_effect() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let fixture = Fixture::new(Some(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        true
    })));
    let db = std::mem::take(&mut *fixture.db.write().await);
    let config = ServerConfig {
        request_admission_policy: RequestAdmissionPolicy {
            max_confirmed_in_flight: 1,
            confirmed_recovery_reserve: 0,
            ..Default::default()
        },
        ..fixture.config
    };
    let mut server = BACnetServer::start(config, db, TestTransport::default())
        .await
        .unwrap();
    // Occupy the single confirmed slot with a never-completing guard holder.
    let peer = canonical_requester(SOURCE, route().as_ref());
    server
        .request_tasks
        .try_spawn(Class::Confirmed, peer, || std::future::pending::<()>())
        .unwrap();
    let before = server
        .db
        .read()
        .await
        .get(&oid(ObjectType::BINARY_VALUE, 1))
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    let (service, bytes, _) = cases().remove(0);
    let (tx, rx) = oneshot::channel();
    BACnetServer::dispatch(
        &server.db,
        &server.network,
        &server.cov_table,
        &server.seg_ack_senders,
        &server.seg_send_permits,
        &server.cov_in_flight,
        &server.server_tsm,
        &server.notification_transactions,
        &server.confirmed_request_tracker,
        &server.device_bindings,
        &server.comm_state,
        &server.dcc_timer,
        &server.dcc_outcomes,
        &server.mutation_decisions,
        &Arc::new(server.config.clone()),
        &server._clock,
        &server.discovery_limiter,
        &server.time_sync_limiter,
        &server.request_tasks,
        SOURCE,
        Apdu::ConfirmedRequest(confirmed(service, bytes, 18)),
        ReceivedApdu {
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: route(),
            link_layer_group: false,
            is_group: false,
            data_attributes: vec![],
            provenance: TransportProvenance::unverified(),
            reply_tx: Some(tx),
        },
    )
    .await;
    let response = tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("overload abort completed")
        .unwrap();
    assert!(
        matches!(apdu(response), Apdu::Abort(abort) if abort.abort_reason == AbortReason::OUT_OF_RESOURCES)
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        server.mutation_decision_counters(),
        MutationDecisionCounters::default()
    );
    let after = server
        .db
        .read()
        .await
        .get(&oid(ObjectType::BINARY_VALUE, 1))
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(before, after);
    assert!(server.network.transport().sent.lock().unwrap().is_empty());
    server.stop().await.unwrap();
}

/// The shared endpoint stays read-only: every mutation choice is rejected
/// through the existing responder with no policy contact, while the same
/// WriteProperty bytes succeed through ordinary dispatch.
#[tokio::test]
async fn shared_endpoint_rejects_every_mutation_choice() {
    let (service, bytes, _) = cases().remove(0);
    let ordinary = Fixture::new(None);
    let accepted = ordinary.dispatch(service, bytes.clone(), 19).await.unwrap();
    assert!(matches!(apdu(accepted), Apdu::SimpleAck(_)));

    let (endpoint_transport, mut peer_transport) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    let mut peer_rx = peer_transport.start().await.unwrap();
    let mut endpoint = EndpointIngress::new(endpoint_transport, 2);
    let ingress = endpoint.start().await.unwrap();
    let responder = EndpointResponder::new(
        Arc::new(RwLock::new(ObjectDatabase::new())),
        ingress.egress.clone(),
    );
    for (service, bytes, _) in cases() {
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::ConfirmedRequest(confirmed(service, bytes, 20)),
        )
        .unwrap();
        assert!(responder
            .handle(ReceivedApdu {
                apdu: encoded.freeze(),
                source_mac: MacAddr::from_slice(SOURCE),
                ingress_network: None,
                source_network: route(),
                link_layer_group: false,
                is_group: false,
                data_attributes: vec![],
                provenance: TransportProvenance::unverified(),
                reply_tx: Some(reply_tx),
            })
            .await
            .unwrap());
        let reply = reply_rx.await.unwrap();
        let rejected = decode_apdu(decode_npdu(reply).unwrap().payload).unwrap();
        assert!(
            matches!(rejected, Apdu::Reject(reject) if reject.reject_reason == RejectReason::UNRECOGNIZED_SERVICE && reject.invoke_id == 20),
            "{service:?}"
        );
    }
    // Segmented mutations draw segmentation-not-supported, still read-only.
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut segmented = confirmed(service, bytes, 21);
    segmented.segmented = true;
    let mut encoded = BytesMut::new();
    encode_apdu(&mut encoded, &Apdu::ConfirmedRequest(segmented)).unwrap();
    assert!(responder
        .handle(ReceivedApdu {
            apdu: encoded.freeze(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: route(),
            link_layer_group: false,
            is_group: false,
            data_attributes: vec![],
            provenance: TransportProvenance::unverified(),
            reply_tx: Some(reply_tx),
        })
        .await
        .unwrap());
    let reply = reply_rx.await.unwrap();
    let aborted = decode_apdu(decode_npdu(reply).unwrap().payload).unwrap();
    assert!(
        matches!(aborted, Apdu::Abort(abort) if abort.abort_reason == AbortReason::SEGMENTATION_NOT_SUPPORTED && abort.invoke_id == 21)
    );
    assert!(matches!(
        peer_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    responder.close();
    endpoint.stop().await.unwrap();
    peer_transport.stop().await.unwrap();
}

fn assert_not_policy_denied(bytes: Bytes, id: u8) {
    match apdu(bytes) {
        Apdu::Error(error) => {
            assert_eq!(error.invoke_id, id);
            assert!(
                !(error.error_class == ErrorClass::SERVICES
                    && error.error_code == ErrorCode::SERVICE_REQUEST_DENIED),
                "read-only service drew a policy denial"
            );
        }
        Apdu::ComplexAck(_) | Apdu::SimpleAck(_) => {}
        other => panic!("unexpected read-only response: {other:?}"),
    }
}

/// Read-only and discovery controls are unaffected by DenyAll: no authorizer
/// contact and never a policy denial.
#[tokio::test]
async fn read_only_controls_unaffected_under_deny_all() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let mut fixture = Fixture::new(Some(Arc::new(move |_| {
        seen.fetch_add(1, Ordering::Relaxed);
        panic!("read-only path must not authorize");
    })));
    fixture.config.mutation_policy = MutationPolicy::DenyAll;

    let mut read = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut read);
    let response = fixture
        .dispatch(ConfirmedServiceChoice::READ_PROPERTY, read.freeze(), 30)
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::ComplexAck(_)));

    let mut rpm = BytesMut::new();
    ReadPropertyMultipleRequest {
        list_of_read_access_specs: vec![ReadAccessSpecification {
            object_identifier: oid(ObjectType::BINARY_VALUE, 1),
            list_of_property_references: vec![bacnet_services::common::PropertyReference {
                property_identifier: PropertyIdentifier::PRESENT_VALUE,
                property_array_index: None,
            }],
        }],
    }
    .encode(&mut rpm);
    let response = fixture
        .dispatch(
            ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE,
            rpm.freeze(),
            31,
        )
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::ComplexAck(_)));

    let mut range = BytesMut::new();
    ReadRangeRequest {
        object_identifier: oid(ObjectType::FILE, 1),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: None,
    }
    .encode(&mut range);
    let response = fixture
        .dispatch(ConfirmedServiceChoice::READ_RANGE, range.freeze(), 32)
        .await
        .unwrap();
    assert_not_policy_denied(response, 32);

    let mut file = BytesMut::new();
    AtomicReadFileRequest {
        file_identifier: oid(ObjectType::FILE, 1),
        access: FileAccessMethod::Stream {
            file_start_position: 0,
            requested_octet_count: 8,
        },
    }
    .encode(&mut file);
    let response = fixture
        .dispatch(ConfirmedServiceChoice::ATOMIC_READ_FILE, file.freeze(), 33)
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::ComplexAck(_)));

    let response = fixture
        .dispatch(ConfirmedServiceChoice::GET_ALARM_SUMMARY, Bytes::new(), 34)
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::ComplexAck(_)));

    let mut enrollment = BytesMut::new();
    GetEnrollmentSummaryRequest {
        acknowledgment_filter: 0,
        enrollment_filter: None,
        event_state_filter: None,
        event_type_filter: None,
        priority_filter: None,
        notification_class_filter: None,
    }
    .encode(&mut enrollment);
    let response = fixture
        .dispatch(
            ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
            enrollment.freeze(),
            35,
        )
        .await
        .unwrap();
    assert_not_policy_denied(response, 35);

    for (service, bytes, id) in [
        (ConfirmedServiceChoice::AUDIT_LOG_QUERY, Bytes::new(), 36),
        (ConfirmedServiceChoice::ACKNOWLEDGE_ALARM, Bytes::new(), 37),
    ] {
        let response = fixture.dispatch(service, bytes, id).await.unwrap();
        assert_not_policy_denied(response, id);
    }

    fixture
        .db
        .write()
        .await
        .add(Box::new(
            DeviceObject::new(DeviceConfig::default()).unwrap(),
        ))
        .unwrap();
    for service in [
        UnconfirmedServiceChoice::WHO_IS,
        UnconfirmedServiceChoice::WHO_HAS,
    ] {
        BACnetServer::<TestTransport>::handle_unconfirmed_request(
            &fixture.db,
            &fixture.network,
            &fixture.config,
            None,
            &fixture.state,
            &Arc::new(RwLock::new(DeviceBindingTable::new())),
            &Arc::new(DiscoveryLimiter::new(DiscoveryPolicy::default(), Some(1))),
            &Arc::new(TimeSyncLimiter::new(TimeSyncPolicy::default())),
            &NotificationTransactions::new(),
            UnconfirmedRequestPdu {
                service_choice: service,
                service_request: Bytes::new(),
            },
            &ReceivedApdu {
                apdu: Bytes::new(),
                source_mac: MacAddr::from_slice(SOURCE),
                ingress_network: None,
                source_network: route(),
                link_layer_group: false,
                is_group: false,
                data_attributes: vec![],
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            },
        )
        .await;
    }

    // Password-authorized DCC still works under DenyAll without policy contact.
    fixture.config.dcc_policy = DccPolicy::RequirePassword;
    fixture.config.dcc_password = Some("rb10-boundary".into());
    fixture.state.store(1, Ordering::Release);
    let mut dcc = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::ENABLE,
        password: Some("rb10-boundary".into()),
    }
    .encode(&mut dcc)
    .unwrap();
    let response = fixture
        .dispatch(
            ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
            dcc.freeze(),
            38,
        )
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

/// Live direct-SC TLS handshake yielding a verified-direct-peer snapshot
/// (loopback TCP, in-test rcgen CA, no committed keys, no sleeps). The
/// handshake evidence is the listener's post-handshake accept, not the test's.
#[cfg(feature = "sc-tls")]
async fn direct_peer_provenance() -> TransportProvenance {
    use bacnet_transport::sc::{ScConnection, WebSocketPort};
    use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
    use bacnet_transport::sc_tls::{
        DirectAcceptConfig, DirectListener, ScNodeTlsConfig, TlsWebSocket,
    };
    use tokio::time::{timeout, Duration};
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().unwrap();
    let ca = ca_params.self_signed(&ca_key).unwrap();
    let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
    let leaf = |sans: Vec<String>| {
        let params = rcgen::CertificateParams::new(sans).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.signed_by(&key, &issuer).unwrap();
        let chain: Vec<CertificateDer<'static>> = vec![cert.der().clone()];
        ScNodeTlsConfig::from_der(
            vec![ca.der().clone()],
            chain,
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
        .unwrap()
    };
    let listener_tls = leaf(vec!["localhost".into(), "127.0.0.1".into()]);
    let config = DirectAcceptConfig::new(
        "127.0.0.1:0".parse().unwrap(),
        [0xAA; 6],
        [9; 16],
        listener_tls,
    );
    let (mut listener, mut rx) = DirectListener::start(config).await.unwrap();
    let url = format!(
        "wss://localhost:{}/.bacnet/sc",
        listener.local_addr().port()
    );
    let ws = timeout(
        Duration::from_secs(5),
        TlsWebSocket::connect_direct(&url, leaf(vec!["node".into()])),
    )
    .await
    .expect("direct dial timed out")
    .expect("direct dial must succeed");
    let mut conn = ScConnection::new([0x22; 6], [7; 16]);
    let request = conn.build_connect_request();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &request);
    ws.send(&buf).await.unwrap();
    let accept_bytes = timeout(Duration::from_secs(5), ws.recv())
        .await
        .expect("accept timed out")
        .unwrap();
    assert!(conn.handle_connect_accept(&decode_sc_message(&accept_bytes).unwrap()));
    let direct = conn
        .build_direct_encapsulated_npdu(&[0x01, 0x00, 0x30], &[])
        .unwrap();
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &direct);
    ws.send(&buf).await.unwrap();
    let received = timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("direct NPDU timed out")
        .expect("listener closed");
    assert!(received.provenance.is_direct_peer());
    let provenance = received.provenance;
    listener.stop().await;
    provenance
}

/// Direct-SC versus hub-mediated-unknown through one verified-only gate: the
/// authenticated channel allows, the unknown leaf denies, and the denied
/// request leaves no database or counter trace beyond its own denial.
#[cfg(feature = "sc-tls")]
#[tokio::test]
async fn direct_channel_allows_where_unknown_leaf_denies() {
    let direct = direct_peer_provenance().await;
    assert_eq!(
        MutationTrust::from_provenance(direct),
        MutationTrust::VerifiedChannel
    );
    let (authorizer, seen) = capturing(|context| context.trust != MutationTrust::Unverified);
    let fixture = Fixture::new(authorizer);
    let decisions = Arc::new(crate::mutation::MutationDecisions::default());
    let (service, bytes, _) = cases().remove(0);
    let before = fixture
        .db
        .read()
        .await
        .get(&oid(ObjectType::BINARY_VALUE, 1))
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();

    let allowed = dispatch_admitted(&fixture, &decisions, service, bytes.clone(), 40, direct)
        .await
        .unwrap();
    assert!(matches!(apdu(allowed), Apdu::SimpleAck(_)));

    let denied = dispatch_admitted(
        &fixture,
        &decisions,
        service,
        bytes,
        41,
        TransportProvenance::unverified(),
    )
    .await
    .unwrap();
    assert_denied(denied, service, 41);
    let after = fixture
        .db
        .read()
        .await
        .get(&oid(ObjectType::BINARY_VALUE, 1))
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    // The denial changed nothing; only the allowed write is visible.
    assert_ne!(before, after);
    let contexts = seen.lock().unwrap();
    assert_eq!(contexts.len(), 2);
    assert_eq!(contexts[0].trust, MutationTrust::VerifiedChannel);
    assert_eq!(contexts[1].trust, MutationTrust::Unverified);
    assert_eq!(decisions.snapshot().write_property.allow_total, 1);
    assert_eq!(decisions.snapshot().write_property.deny_total, 1);
}
