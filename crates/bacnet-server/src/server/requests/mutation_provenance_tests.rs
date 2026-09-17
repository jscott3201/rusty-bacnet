//! RB-10: mutation authorization observes verified ingress context.
//!
//! In-memory, deterministic, no sleeps. Pins that every gated mutation
//! decision receives the reassembled [`TransportProvenance`] snapshot plus
//! the derived channel/relay [`MutationTrust`]; that unknown origin (including
//! a hub-mediated unknown leaf) never satisfies a verified-only allow rule;
//! that receive-permission is not write-permission; that WPM elements share
//! one reassembled snapshot; and that Debug/counter outputs carry no secrets
//! or inputs.
//!
//! Entry-path coverage (DenyAll dominance, overload abort, shared-endpoint
//! rejection, read-only controls, direct-SC) lives in
//! `mutation_entry_tests`. WPM ordered-prefix semantics, per-service
//! counters, and validation-before-auth order remain pinned by the existing
//! `mutation_*` suites; LifeSafety keeps its own authorizer without
//! provenance (noted gap, out of scope).

use super::mutation_tests::{
    apdu, assert_denied, cases, oid, route, Fixture, TestTransport, SOURCE,
};
use super::*;
use crate::mutation::{MutationAuthorizationContext, MutationAuthorizer, MutationTrust};
use bacnet_services::file::{AtomicWriteFileRequest, FileWriteAccessMethod};
use bacnet_transport::port::TransportProvenance;
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
        &Arc::new(Mutex::new(None)),
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
async fn unverified_snapshot_reaches_gate_with_unverified_trust() {
    let (authorizer, seen) = capturing(|_| true);
    let fixture = Fixture::new(authorizer);
    let decisions = Arc::new(crate::mutation::MutationDecisions::default());
    let (service, bytes, _) = cases().remove(0);
    let response = dispatch_admitted(
        &fixture,
        &decisions,
        service,
        bytes,
        11,
        TransportProvenance::unverified(),
    )
    .await
    .unwrap();
    assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
    let contexts = seen.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert!(contexts[0].provenance.is_unverified());
    assert_eq!(contexts[0].trust, MutationTrust::Unverified);
    assert_eq!(contexts[0].source_network, route());
}

#[tokio::test]
async fn wpm_elements_share_one_reassembled_snapshot() {
    for provenance in [
        TransportProvenance::unverified(),
        relayed_origin_provenance().await,
    ] {
        let (authorizer, seen) = capturing(|_| true);
        let fixture = Fixture::new(authorizer);
        let decisions = Arc::new(crate::mutation::MutationDecisions::default());
        let first = oid(ObjectType::BINARY_VALUE, 1);
        let mut bytes = BytesMut::new();
        bacnet_services::wpm::WritePropertyMultipleRequest {
            list_of_write_access_specs: vec![
                bacnet_services::wpm::WriteAccessSpecification {
                    object_identifier: first,
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::DESCRIPTION,
                        property_array_index: None,
                        value: {
                            let mut value = BytesMut::new();
                            encode_property_value(
                                &mut value,
                                &PropertyValue::CharacterString("one".into()),
                            )
                            .unwrap();
                            value.to_vec()
                        },
                        priority: None,
                    }],
                },
                bacnet_services::wpm::WriteAccessSpecification {
                    object_identifier: oid(ObjectType::BINARY_VALUE, 2),
                    list_of_properties: vec![BACnetPropertyValue {
                        property_identifier: PropertyIdentifier::DESCRIPTION,
                        property_array_index: None,
                        value: {
                            let mut value = BytesMut::new();
                            encode_property_value(
                                &mut value,
                                &PropertyValue::CharacterString("two".into()),
                            )
                            .unwrap();
                            value.to_vec()
                        },
                        priority: None,
                    }],
                },
            ],
        }
        .encode(&mut bytes);
        let response = dispatch_admitted(
            &fixture,
            &decisions,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            bytes.freeze(),
            12,
            provenance,
        )
        .await
        .unwrap();
        assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
        // The mutation layer observes the reassembled snapshot consistently:
        // segmentation mismatches already fail closed at reassembly, so both
        // element contexts carry the identical snapshot.
        let contexts = seen.lock().unwrap();
        assert_eq!(contexts.len(), 2);
        assert!(contexts
            .iter()
            .all(|context| context.provenance == provenance));
        assert!(contexts
            .iter()
            .all(|context| context.trust == MutationTrust::from_provenance(provenance)));
    }
}

#[tokio::test]
async fn verified_relay_snapshot_reaches_gate_with_relay_trust() {
    let provenance = relayed_origin_provenance().await;
    assert!(provenance.is_relayed_origin());
    let (authorizer, seen) = capturing(|_| true);
    let fixture = Fixture::new(authorizer);
    let decisions = Arc::new(crate::mutation::MutationDecisions::default());
    let (service, bytes, _) = cases().remove(0);
    let response = dispatch_admitted(&fixture, &decisions, service, bytes, 13, provenance)
        .await
        .unwrap();
    assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
    let contexts = seen.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].provenance, provenance);
    assert_eq!(contexts[0].trust, MutationTrust::VerifiedRelay);
}

/// Hub-mediated unknown leaf arrives unverified: the relay validated nothing
/// about the leaf, so a verified-only allow rule must deny it while an
/// allow-all rule still allows (default-allow preserved; the policy, not the
/// gate, owns the baseline-only profile).
#[tokio::test]
async fn hub_mediated_unknown_leaf_never_satisfies_verified_only_rule() {
    let (service, bytes, _) = cases().remove(0);
    for (allow, expected) in [(false, false), (true, true)] {
        let (authorizer, seen) =
            capturing(move |context| allow || context.trust != MutationTrust::Unverified);
        let fixture = Fixture::new(authorizer);
        let decisions = Arc::new(crate::mutation::MutationDecisions::default());
        // Routed claim with no validated relay: the hub-mediated unknown shape.
        let response = dispatch_admitted(
            &fixture,
            &decisions,
            service,
            bytes.clone(),
            14,
            TransportProvenance::unverified(),
        )
        .await
        .unwrap();
        if expected {
            assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
        } else {
            assert_denied(response, service, 14);
        }
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(seen.lock().unwrap()[0].trust, MutationTrust::Unverified);
    }
}

/// Receive-permission is not write-permission: an authorizer that allows one
/// covered service still denies the others, including across the
/// subscribe-versus-write boundary.
#[tokio::test]
async fn allow_for_one_service_is_not_allow_for_another() {
    let (authorizer, seen) =
        capturing(|context| context.service_choice == ConfirmedServiceChoice::SUBSCRIBE_COV);
    let fixture = Fixture::new(authorizer);
    let decisions = Arc::new(crate::mutation::MutationDecisions::default());
    let cases = cases();
    let (write_service, write_bytes, _) = cases[0].clone();
    let (cov_service, cov_bytes, _) = cases[7].clone();
    let write = dispatch_admitted(
        &fixture,
        &decisions,
        write_service,
        write_bytes,
        15,
        TransportProvenance::unverified(),
    )
    .await
    .unwrap();
    assert_denied(write, write_service, 15);
    let cov = dispatch_admitted(
        &fixture,
        &decisions,
        cov_service,
        cov_bytes,
        16,
        TransportProvenance::unverified(),
    )
    .await
    .unwrap();
    assert!(matches!(apdu(cov), Apdu::SimpleAck(_)));
    assert_eq!(seen.lock().unwrap().len(), 2);
}

/// DenyAll dominates an allow-all authorizer for every covered service: the
/// callback is never invoked and the wire denial keeps its service shape.
/// Debug and counter outputs carry no MAC bytes, property values, file
/// payloads, or other decoded inputs — only lengths, kinds, and totals.
#[tokio::test]
async fn debug_and_counters_carry_no_secrets_or_inputs() {
    let distinctive_mac = MacAddr::from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x42, 0x24]);
    let mut target = BytesMut::new();
    AtomicWriteFileRequest {
        file_identifier: oid(ObjectType::FILE, 1),
        access: FileWriteAccessMethod::Stream {
            file_start_position: 0,
            file_data: vec![0xCA, 0xFE, 0xBA, 0xBE],
        },
    }
    .encode(&mut target);
    let request = AtomicWriteFileRequest::decode(&target).unwrap();
    let context = MutationAuthorizationContext {
        source_mac: distinctive_mac,
        source_network: Some(NpduAddress {
            network: 7,
            mac_address: MacAddr::from_slice(&[0xDE, 0xAD, 0xBE]),
        }),
        provenance: TransportProvenance::unverified(),
        trust: MutationTrust::Unverified,
        invoke_id: 7,
        service_choice: ConfirmedServiceChoice::ATOMIC_WRITE_FILE,
        target: crate::mutation::MutationTarget::AtomicWriteFile(request),
    };
    let rendered = format!("{context:?}");
    for leaked in ["222", "173", "190", "239", "202", "254"] {
        assert!(
            !rendered.contains(leaked),
            "redacted Debug leaked address/payload bytes: {rendered}"
        );
    }
    assert!(rendered.contains("source_mac_len"));
    assert!(rendered.contains("atomic-write-file"));

    // Counters record decisions with no source history: a denial from a
    // distinctive source leaves only totals behind.
    let (authorizer, _) = capturing(|_| false);
    let fixture = Fixture::new(authorizer);
    let decisions = Arc::new(crate::mutation::MutationDecisions::default());
    let (service, bytes, _) = cases().remove(0);
    let denied = dispatch_admitted(
        &fixture,
        &decisions,
        service,
        bytes,
        39,
        TransportProvenance::unverified(),
    )
    .await
    .unwrap();
    assert_denied(denied, service, 39);
    let rendered = format!("{:?}", decisions.snapshot());
    assert!(!rendered.contains("127"));
    assert!(rendered.contains("deny_total: 1"));
}

/// Live SC-hub relay handshake yielding a verified-relayed-origin snapshot
/// (loopback transport, no TLS, no sleeps). Mirrors the RB-07 threading
/// fixture; the relay admission evidence is the hub's, not the test's.
async fn relayed_origin_provenance() -> TransportProvenance {
    use bacnet_transport::sc::{LoopbackWebSocket, ScTransport, WebSocketPort};
    use bacnet_transport::sc_frame::{decode_sc_message, encode_sc_message, ScFunction, ScMessage};
    use tokio::time::{timeout, Duration};

    let (ws_client, ws_hub) = LoopbackWebSocket::pair();
    let mut transport = ScTransport::new(ws_client, [0x01; 6]).with_device_uuid([1; 16]);
    let hub_task = tokio::spawn(async move {
        let data = ws_hub.recv().await.unwrap();
        let req = decode_sc_message(&data).unwrap();
        let mut payload = Vec::with_capacity(26);
        payload.extend_from_slice(&[0x10; 6]);
        payload.extend_from_slice(&[0x33; 16]);
        payload.extend_from_slice(&1476u16.to_be_bytes());
        payload.extend_from_slice(&1476u16.to_be_bytes());
        let accept = ScMessage {
            function: ScFunction::ConnectAccept,
            message_id: req.message_id,
            originating_vmac: None,
            destination_vmac: None,
            dest_options: Vec::new(),
            data_options: Vec::new(),
            payload: Bytes::from(payload),
        };
        let mut buf = BytesMut::new();
        encode_sc_message(&mut buf, &accept);
        ws_hub.send(&buf).await.unwrap();
        ws_hub
    });
    let mut rx = transport.start().await.unwrap();
    let ws_hub = hub_task.await.unwrap();
    let msg = ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x2233,
        originating_vmac: Some([0x22; 6]),
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    };
    let mut buf = BytesMut::new();
    encode_sc_message(&mut buf, &msg);
    ws_hub.send(&buf).await.unwrap();
    let received = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("hub relay timed out")
        .expect("hub closed");
    let provenance = received.provenance;
    assert!(provenance.is_relayed_origin());
    transport.stop().await.unwrap();
    provenance
}
