use super::*;
use crate::mutation::{MutationAuthorizer, MutationDecisionCounters, MutationServiceCounters};
use crate::server::requests::mutation_tests::{
    apdu, assert_denied, cases, oid, route, value, wpm, Fixture, TestTransport, SOURCE,
};
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_services::device_mgmt::DeviceCommunicationControlRequest;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleError};
use bacnet_types::enums::EnableDisable;
use std::sync::atomic::AtomicUsize;

#[path = "mutation_runtime_tests.rs"]
mod runtime_tests;

async fn server(
    policy: MutationPolicy,
    authorizer: Option<MutationAuthorizer>,
) -> BACnetServer<TestTransport> {
    let fixture = Fixture::new(authorizer);
    let db = std::mem::take(&mut *fixture.db.write().await);
    let config = ServerConfig {
        mutation_policy: policy,
        ..fixture.config
    };
    BACnetServer::start(config, db, TestTransport::default())
        .await
        .unwrap()
}

fn request(service: ConfirmedServiceChoice, bytes: Bytes, id: u8) -> ConfirmedRequestPdu {
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

async fn dispatch(
    server: &BACnetServer<TestTransport>,
    service: ConfirmedServiceChoice,
    bytes: Bytes,
    id: u8,
) -> Option<Bytes> {
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
        Apdu::ConfirmedRequest(request(service, bytes, id)),
        ReceivedApdu {
            apdu: Bytes::new(),
            source_mac: MacAddr::from_slice(SOURCE),
            ingress_network: None,
            source_network: route(),
            link_layer_group: false,
            is_group: false,
            data_attributes: vec![],
            reply_tx: Some(tx),
        },
    )
    .await;
    tokio::time::timeout(Duration::from_secs(2), rx)
        .await
        .expect("handler completed")
        .ok()
}

async fn snapshot(
    server: &BACnetServer<TestTransport>,
) -> (
    Vec<(ObjectIdentifier, PropertyIdentifier, PropertyValue)>,
    Vec<u8>,
    usize,
) {
    let db = server.db.read().await;
    let mut objects = db.list_objects();
    objects.sort_by_key(|id| (id.object_type().to_raw(), id.instance_number()));
    let mut values = Vec::new();
    for id in objects {
        let object = db.get(&id).unwrap();
        for property in [
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::ALARM_VALUES,
            PropertyIdentifier::FILE_SIZE,
            PropertyIdentifier::ARCHIVE,
        ] {
            if let Ok(value) = object.read_property(property, None) {
                values.push((id, property, value));
            }
        }
    }
    let file = db
        .get(&oid(ObjectType::FILE, 1))
        .unwrap()
        .file_storage_internal()
        .unwrap()
        .read_stream(0, 100)
        .unwrap()
        .data;
    (values, file, server.cov_table.read().await.len())
}

fn expected(
    service: ConfirmedServiceChoice,
    row: MutationServiceCounters,
) -> MutationDecisionCounters {
    let mut counters = MutationDecisionCounters::default();
    *match service {
        ConfirmedServiceChoice::WRITE_PROPERTY => &mut counters.write_property,
        ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE => &mut counters.write_property_multiple,
        ConfirmedServiceChoice::CREATE_OBJECT => &mut counters.create_object,
        ConfirmedServiceChoice::DELETE_OBJECT => &mut counters.delete_object,
        ConfirmedServiceChoice::ADD_LIST_ELEMENT => &mut counters.add_list_element,
        ConfirmedServiceChoice::REMOVE_LIST_ELEMENT => &mut counters.remove_list_element,
        ConfirmedServiceChoice::ATOMIC_WRITE_FILE => &mut counters.atomic_write_file,
        ConfirmedServiceChoice::SUBSCRIBE_COV => &mut counters.subscribe_cov,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY => &mut counters.subscribe_cov_property,
        ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE => {
            &mut counters.subscribe_cov_property_multiple
        }
        _ => panic!("not a covered mutation"),
    } = row;
    counters
}

#[tokio::test]
async fn mutation_policy_matrix_all_ten_decisions_tracking_and_retention() {
    assert_eq!(
        ServerConfig::default().mutation_policy,
        MutationPolicy::Permissive
    );
    for policy in [MutationPolicy::Permissive, MutationPolicy::DenyAll] {
        for callback in 0..4 {
            for (service, bytes, _) in cases() {
                let calls = Arc::new(AtomicUsize::new(0));
                let seen = calls.clone();
                let authorizer: Option<MutationAuthorizer> = (callback != 0).then(|| {
                    Arc::new(move |_: &MutationAuthorizationContext| {
                        seen.fetch_add(1, Ordering::Relaxed);
                        assert_ne!(callback, 3, "authorizer panic");
                        callback == 1
                    }) as MutationAuthorizer
                });
                let mut server = server(policy, authorizer).await;
                assert_eq!(
                    server.mutation_decision_counters(),
                    MutationDecisionCounters::default()
                );
                let before = snapshot(&server).await;
                let response = dispatch(&server, service, bytes.clone(), 1).await.unwrap();
                let allowed = policy == MutationPolicy::Permissive && callback <= 1;
                if allowed {
                    assert!(
                        matches!(apdu(response), Apdu::SimpleAck(_) | Apdu::ComplexAck(_)),
                        "{service:?}"
                    );
                    assert_ne!(snapshot(&server).await, before, "{service:?}");
                } else {
                    assert_denied(response, service, 1);
                    assert_eq!(snapshot(&server).await, before, "{service:?}");
                    assert!(server.network.transport().sent.lock().unwrap().is_empty());
                }
                let counters = expected(
                    service,
                    MutationServiceCounters {
                        allow_total: u64::from(allowed),
                        deny_total: u64::from(!allowed),
                        policy_deny_total: u64::from(policy == MutationPolicy::DenyAll),
                    },
                );
                assert_eq!(server.mutation_decision_counters(), counters, "{service:?}");
                // Completed exact retries do not make another decision, even on denial.
                assert!(dispatch(&server, service, bytes, 1).await.is_none());
                assert_eq!(server.mutation_decision_counters(), counters);
                assert_eq!(
                    calls.load(Ordering::Relaxed),
                    usize::from(policy == MutationPolicy::Permissive && callback != 0)
                );
                server.stop().await.unwrap();
                assert_eq!(server.mutation_decision_counters(), counters);
            }
        }
    }
}

#[test]
fn mutation_policy_builders_preserve_authorizers_and_defaults() {
    assert_eq!(
        BACnetServer::bip_builder().config.mutation_policy,
        MutationPolicy::Permissive
    );
    assert_eq!(
        BACnetServer::<TestTransport>::generic_builder()
            .config
            .mutation_policy,
        MutationPolicy::Permissive
    );
    let generic = BACnetServer::<TestTransport>::generic_builder()
        .mutation_authorizer(|_| true)
        .mutation_policy(MutationPolicy::DenyAll)
        .config;
    let bip = BACnetServer::bip_builder()
        .mutation_policy(MutationPolicy::DenyAll)
        .mutation_authorizer(|_| true)
        .config;
    for config in [generic, bip] {
        assert_eq!(config.mutation_policy, MutationPolicy::DenyAll);
        assert!(config.mutation_authorizer.is_some());
        assert!(format!("{config:?}").contains("mutation_policy: DenyAll"));
    }
    #[cfg(feature = "sc-tls")]
    {
        assert_eq!(
            BACnetServer::sc_builder().config.mutation_policy,
            MutationPolicy::Permissive
        );
        let config = BACnetServer::sc_builder()
            .mutation_policy(MutationPolicy::DenyAll)
            .mutation_authorizer(|_| true)
            .config;
        assert_eq!(config.mutation_policy, MutationPolicy::DenyAll);
        assert!(config.mutation_authorizer.is_some());
    }
}

fn descriptions() -> Bytes {
    wpm(vec![WriteAccessSpecification {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        list_of_properties: ["first", "second", "third"]
            .map(|text| BACnetPropertyValue {
                property_identifier: PropertyIdentifier::DESCRIPTION,
                property_array_index: None,
                value: value(PropertyValue::CharacterString(text.into())),
                priority: None,
            })
            .to_vec(),
    }])
}

#[tokio::test]
async fn mutation_wpm_counts_elements_not_objects_requests_or_unvisited_suffixes() {
    let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
    for policy in [MutationPolicy::Permissive, MutationPolicy::DenyAll] {
        for callback in 0..4 {
            let calls = Arc::new(AtomicUsize::new(0));
            let seen = calls.clone();
            let authorizer: Option<MutationAuthorizer> = (callback != 0).then(|| {
                Arc::new(move |_: &MutationAuthorizationContext| {
                    let n = seen.fetch_add(1, Ordering::Relaxed);
                    if n == 1 && callback >= 2 {
                        assert_ne!(callback, 3, "second element panic");
                        return false;
                    }
                    true
                }) as MutationAuthorizer
            });
            let mut server = server(policy, authorizer).await;
            let before = snapshot(&server).await;
            let response = dispatch(&server, service, descriptions(), 1).await.unwrap();
            let hard = policy == MutationPolicy::DenyAll;
            let denied = hard || callback >= 2;
            if denied {
                assert_denied(response.clone(), service, 1);
                let Apdu::Error(error) = apdu(response) else {
                    unreachable!()
                };
                let detail = WritePropertyMultipleError::from_error_pdu(&error).unwrap();
                assert_eq!(
                    detail.first_failed_write_attempt.object_identifier,
                    oid(ObjectType::BINARY_VALUE, 1)
                );
                assert_eq!(
                    detail.first_failed_write_attempt.property_identifier,
                    PropertyIdentifier::DESCRIPTION.to_raw()
                );
            } else {
                assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
            }
            if hard {
                assert_eq!(snapshot(&server).await, before);
            } else {
                let db = server.db.read().await;
                let description = db
                    .get(&oid(ObjectType::BINARY_VALUE, 1))
                    .unwrap()
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap();
                assert_eq!(
                    description,
                    PropertyValue::CharacterString(if denied { "first" } else { "third" }.into())
                );
            }
            assert_eq!(
                server.mutation_decision_counters(),
                expected(
                    service,
                    MutationServiceCounters {
                        allow_total: if hard {
                            0
                        } else if denied {
                            1
                        } else {
                            3
                        },
                        deny_total: u64::from(denied),
                        policy_deny_total: u64::from(hard),
                    }
                )
            );
            assert_eq!(
                calls.load(Ordering::Relaxed),
                if hard || callback == 0 {
                    0
                } else if denied {
                    2
                } else {
                    3
                }
            );
            server.stop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn mutation_pre_gate_failures_and_dcc_drops_do_not_count() {
    for policy in [MutationPolicy::Permissive, MutationPolicy::DenyAll] {
        let mut server = server(policy, Some(Arc::new(|_| panic!("must not authorize")))).await;
        for (index, (service, bytes, _)) in cases().into_iter().enumerate() {
            let malformed = bytes.slice(..1);
            let response = dispatch(&server, service, malformed, index as u8)
                .await
                .unwrap();
            assert!(matches!(apdu(response), Apdu::Error(_) | Apdu::Reject(_)));
        }
        let response = dispatch(
            &server,
            ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE,
            Bytes::new(),
            20,
        )
        .await
        .unwrap();
        assert!(matches!(apdu(response), Apdu::SimpleAck(_)));
        server.comm_state.store(1, Ordering::Release);
        for (index, (service, bytes, _)) in cases().into_iter().enumerate() {
            assert!(dispatch(&server, service, bytes, 30 + index as u8)
                .await
                .is_none());
        }
        assert_eq!(
            server.mutation_decision_counters(),
            MutationDecisionCounters::default()
        );
        server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn mutation_wpm_malformed_suffix_counts_only_reached_prefix() {
    for authorizer in [
        None,
        Some(Arc::new(|_: &MutationAuthorizationContext| true) as MutationAuthorizer),
    ] {
        let mut server = server(MutationPolicy::Permissive, authorizer).await;
        let mut bytes = BytesMut::from(descriptions().as_ref());
        bytes.extend_from_slice(&[0x0c]); // incomplete next object after three writes
        let service = ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE;
        let response = dispatch(&server, service, bytes.freeze(), 1).await.unwrap();
        let Apdu::Error(error) = apdu(response) else {
            panic!("expected suffix Error")
        };
        assert_eq!(error.error_code, ErrorCode::INVALID_TAG);
        assert_eq!(
            server.mutation_decision_counters(),
            expected(
                service,
                MutationServiceCounters {
                    allow_total: 3,
                    ..Default::default()
                }
            )
        );
        server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn mutation_deny_all_does_not_gate_reads_discovery_or_dcc() {
    let mut server = server(
        MutationPolicy::DenyAll,
        Some(Arc::new(|_| panic!("not a mutation"))),
    )
    .await;
    let mut data = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: oid(ObjectType::BINARY_VALUE, 1),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
    .encode(&mut data);
    assert!(matches!(
        apdu(
            dispatch(
                &server,
                ConfirmedServiceChoice::READ_PROPERTY,
                data.freeze(),
                1
            )
            .await
            .unwrap()
        ),
        Apdu::ComplexAck(_)
    ));
    server
        .db
        .write()
        .await
        .add(Box::new(
            DeviceObject::new(DeviceConfig::default()).unwrap(),
        ))
        .unwrap();
    BACnetServer::<TestTransport>::handle_unconfirmed_request(
        &server.db,
        &server.network,
        &server.config,
        None,
        &server.comm_state,
        &server.device_bindings,
        &server.discovery_limiter,
        &server.time_sync_limiter,
        UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::WHO_IS,
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
            reply_tx: None,
        },
    )
    .await;
    let sent = server
        .network
        .transport()
        .sent
        .lock()
        .unwrap()
        .pop()
        .unwrap();
    let Apdu::UnconfirmedRequest(i_am) = apdu(sent) else {
        panic!("expected I-Am")
    };
    assert_eq!(i_am.service_choice, UnconfirmedServiceChoice::I_AM);
    server.config.dcc_policy = DccPolicy::RequirePassword;
    server.config.dcc_password = Some("boundary-test".into());
    server.comm_state.store(1, Ordering::Release);
    let mut data = BytesMut::new();
    DeviceCommunicationControlRequest {
        time_duration: None,
        enable_disable: EnableDisable::ENABLE,
        password: Some("boundary-test".into()),
    }
    .encode(&mut data)
    .unwrap();
    assert!(matches!(
        apdu(
            dispatch(
                &server,
                ConfirmedServiceChoice::DEVICE_COMMUNICATION_CONTROL,
                data.freeze(),
                2
            )
            .await
            .unwrap()
        ),
        Apdu::SimpleAck(_)
    ));
    assert_eq!(server.comm_state.load(Ordering::Acquire), 0);
    assert_eq!(server.dcc_outcome_counters().accepted_total, 1);
    assert_eq!(
        server.mutation_decision_counters(),
        MutationDecisionCounters::default()
    );
    server.stop().await.unwrap();
}
