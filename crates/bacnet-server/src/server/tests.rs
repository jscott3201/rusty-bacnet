use super::*;

#[test]
fn server_config_cov_retry_timeout_default() {
    let config = ServerConfig::default();
    assert_eq!(config.cov_retry_timeout_ms, 3000);
}

#[test]
fn server_config_time_sync_callback_default_is_none() {
    let config = ServerConfig::default();
    assert!(config.on_time_sync.is_none());
    assert!(config.life_safety_operation_authorizer.is_none());
}

#[test]
fn all_builders_assign_life_safety_operation_authorizer() {
    let generic =
        BACnetServer::<BipTransport>::generic_builder().life_safety_operation_authorizer(|_| true);
    assert!(generic.config.life_safety_operation_authorizer.is_some());

    let bip =
        BACnetServer::<BipTransport>::bip_builder().life_safety_operation_authorizer(|_| true);
    assert!(bip.config.life_safety_operation_authorizer.is_some());

    #[cfg(feature = "sc-tls")]
    {
        let sc = BACnetServer::sc_builder().life_safety_operation_authorizer(|_| true);
        assert!(sc.config.life_safety_operation_authorizer.is_some());
    }
}

// -----------------------------------------------------------------------
// Event Enrollment lifecycle configuration (issue #133)
// -----------------------------------------------------------------------

#[test]
fn server_config_event_enrollment_defaults() {
    let config = ServerConfig::default();
    assert!(config.enable_event_enrollment);
    assert_eq!(config.event_enrollment_interval_secs, 10);
    // Enrollment evaluation defaults on while fault detection defaults off;
    // the two settings are unrelated.
    assert!(!config.enable_fault_detection);
}

#[test]
fn event_enrollment_period_clamps_zero_to_one_second() {
    // tokio::time::interval panics on a zero period, and that panic would land
    // inside a spawned task where it cannot be observed by the caller.
    assert_eq!(
        super::lifecycle::event_enrollment_period(0),
        Duration::from_secs(1)
    );
}

#[test]
fn event_enrollment_period_passes_through_nonzero() {
    assert_eq!(
        super::lifecycle::event_enrollment_period(30),
        Duration::from_secs(30)
    );
}

#[test]
fn all_builders_assign_segmentation_supported() {
    // Same copy-paste hazard as the enrollment test below: the setter targets
    // one ServerConfig field among look-alikes, and the dispatch loop now
    // enforces it (Clause 5.4.5.1), so a mis-assignment silently turns
    // segmented reception off.
    let generic = BACnetServer::<BipTransport>::generic_builder()
        .segmentation_supported(Segmentation::RECEIVE);
    assert_eq!(generic.config.segmentation_supported, Segmentation::RECEIVE);

    let bip =
        BACnetServer::<BipTransport>::bip_builder().segmentation_supported(Segmentation::BOTH);
    assert_eq!(bip.config.segmentation_supported, Segmentation::BOTH);

    #[cfg(feature = "sc-tls")]
    {
        let sc = BACnetServer::sc_builder().segmentation_supported(Segmentation::RECEIVE);
        assert_eq!(sc.config.segmentation_supported, Segmentation::RECEIVE);
    }
}

#[test]
fn all_builders_assign_event_enrollment_config() {
    // Six near-identical assignments across three builders. Every neighbouring
    // ServerConfig field is a bool or u64, so a copy-paste targeting the wrong one
    // compiles and stays silent under clippy — hence the neighbour assertions.
    let generic = BACnetServer::<BipTransport>::generic_builder()
        .enable_event_enrollment(false)
        .event_enrollment_interval_secs(7);
    assert!(!generic.config.enable_event_enrollment);
    assert_eq!(generic.config.event_enrollment_interval_secs, 7);
    assert_eq!(generic.config.cov_retry_timeout_ms, 3000);
    assert!(!generic.config.enable_fault_detection);

    let bip = BACnetServer::<BipTransport>::bip_builder()
        .enable_event_enrollment(false)
        .event_enrollment_interval_secs(11);
    assert!(!bip.config.enable_event_enrollment);
    assert_eq!(bip.config.event_enrollment_interval_secs, 11);
    assert_eq!(bip.config.cov_retry_timeout_ms, 3000);
    assert!(!bip.config.enable_fault_detection);

    #[cfg(feature = "sc-tls")]
    {
        let sc = BACnetServer::sc_builder()
            .enable_event_enrollment(false)
            .event_enrollment_interval_secs(13);
        assert!(!sc.config.enable_event_enrollment);
        assert_eq!(sc.config.event_enrollment_interval_secs, 13);
        assert_eq!(sc.config.cov_retry_timeout_ms, 3000);
    }
}

#[tokio::test(start_paused = true)]
async fn server_enrollment_task_evaluates_at_startup_on_its_configured_interval() {
    use bacnet_objects::analog::AnalogInputObject;
    use bacnet_objects::event_enrollment::EventEnrollmentObject;
    use bacnet_objects::traits::BACnetObject;
    use bacnet_types::constructed::{BACnetDeviceObjectPropertyReference, BACnetEventParameter};
    use bacnet_types::enums::{EventState, EventType, PropertyIdentifier};

    // Exercises the spawned task, not `tokio::time::interval` in isolation. Two
    // regressions only this can reach: switching to `interval_at` (first pass one
    // interval late, contradicting the documented startup behavior) and hardcoding
    // the period so the config field is silently ignored.
    let mut db = ObjectDatabase::new();

    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_present_value(150.0); // above high_limit
    let ai_oid = ai.object_identifier();
    db.add(Box::new(ai)).unwrap();

    let mut ee = EventEnrollmentObject::new(1, "EE-1", EventType::OUT_OF_RANGE.to_raw()).unwrap();
    ee.set_object_property_reference(Some(BACnetDeviceObjectPropertyReference::new_local(
        ai_oid,
        PropertyIdentifier::PRESENT_VALUE.to_raw(),
    )));
    ee.set_event_parameters(BACnetEventParameter::OutOfRange {
        time_delay: 0,
        low_limit: 0.0,
        high_limit: 100.0,
        deadband: 1.0,
    });
    ee.set_event_enable(0x07);
    let ee_oid = ee.object_identifier();
    db.add(Box::new(ee)).unwrap();

    // An hour, so a first pass deferred by one interval could never be observed.
    let config = ServerConfig {
        event_enrollment_interval_secs: 3600,
        ..ServerConfig::default()
    };
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = BACnetServer::start(config, db, transport)
        .await
        .expect("server should start");

    // The first pass runs at spawn, well inside the configured hour.
    tokio::time::sleep(Duration::from_millis(50)).await;
    {
        let guard = server.db.read().await;
        let state = guard
            .get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(
            state,
            PropertyValue::Enumerated(EventState::HIGH_LIMIT.to_raw())
        );
    }

    // Force it back, then advance far past a hardcoded 10s default but nowhere near
    // the configured hour. A second pass would re-detect the (still out-of-range)
    // value and revert this.
    {
        let mut guard = server.db.write().await;
        guard
            .get_mut(&ee_oid)
            .unwrap()
            .set_event_state_internal(EventState::NORMAL)
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(60)).await;
    {
        let guard = server.db.read().await;
        let state = guard
            .get(&ee_oid)
            .unwrap()
            .read_property(PropertyIdentifier::EVENT_STATE, None)
            .unwrap();
        assert_eq!(
            state,
            PropertyValue::Enumerated(EventState::NORMAL.to_raw()),
            "a pass ran within 60s, so the configured interval was not honored"
        );
    }

    server.stop().await.expect("server should stop");
}

#[tokio::test]
async fn server_spawns_enrollment_task_without_fault_detection() {
    // Regression for #133: enrollment evaluation used to be gated on
    // enable_fault_detection, so this configuration silently skipped it.
    let config = ServerConfig {
        enable_fault_detection: false,
        enable_event_enrollment: true,
        ..ServerConfig::default()
    };
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = BACnetServer::start(config, ObjectDatabase::new(), transport)
        .await
        .expect("server should start");

    assert!(server.event_enrollment_task.is_some());
    assert!(server.fault_detection_task.is_none());

    server.stop().await.expect("server should stop");
}

#[tokio::test]
async fn server_runs_fault_detection_without_enrollment() {
    // The inverse pairing must also hold, or the two settings are still coupled.
    let config = ServerConfig {
        enable_fault_detection: true,
        enable_event_enrollment: false,
        ..ServerConfig::default()
    };
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);
    let mut server = BACnetServer::start(config, ObjectDatabase::new(), transport)
        .await
        .expect("server should start");

    assert!(server.fault_detection_task.is_some());
    assert!(server.event_enrollment_task.is_none());

    server.stop().await.expect("server should stop");
}

#[tokio::test]
async fn server_rejects_invalid_max_apdu_length() {
    let config = ServerConfig {
        max_apdu_length: 1000,
        ..ServerConfig::default()
    };
    let transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST);

    let result = BACnetServer::start(config, ObjectDatabase::new(), transport).await;
    assert!(result.is_err());
}

// -----------------------------------------------------------------------
// ServerTsm unit tests
// -----------------------------------------------------------------------

fn test_mac(byte: u8) -> MacAddr {
    MacAddr::from_slice(&[127, 0, 0, byte, 0xBA, 0xC0])
}

fn test_peer(byte: u8) -> TsmPeer {
    (test_mac(byte), None)
}

#[test]
fn server_tsm_allocate_increments() {
    let mut tsm = ServerTsm::new();
    let peer = test_peer(1);
    assert_eq!(tsm.allocate(peer.clone()).unwrap().0, 0);
    assert_eq!(tsm.allocate(peer.clone()).unwrap().0, 1);
    assert_eq!(tsm.allocate(peer).unwrap().0, 2);
}

#[test]
fn server_tsm_allocate_wraps_at_255() {
    let mut tsm = ServerTsm::new();
    let peer = test_peer(1);
    tsm.next_invoke_id = 255;
    assert_eq!(tsm.allocate(peer.clone()).unwrap().0, 255);
    assert_eq!(tsm.allocate(peer).unwrap().0, 0); // wraps
}

#[test]
fn server_tsm_allocate_wrap_skips_active_invoke_id() {
    let mut tsm = ServerTsm::new();
    let peer = test_peer(1);

    let (id0, _rx0) = tsm.allocate(peer.clone()).unwrap();
    assert_eq!(id0, 0);

    tsm.next_invoke_id = 0;
    let (id1, _rx1) = tsm.allocate(peer).unwrap();
    assert_eq!(id1, 1);
}

#[test]
fn server_tsm_record_and_take_ack() {
    let mut tsm = ServerTsm::new();
    let peer = test_peer(1);
    let (id, rx) = tsm.allocate(peer.clone()).unwrap();
    assert!(tsm.record_result(&peer.0, peer.1.as_ref(), id, CovAckResult::Ack));
    // Result should be delivered via the oneshot channel
    assert_eq!(rx.blocking_recv(), Ok(CovAckResult::Ack));
}

#[test]
fn server_tsm_record_and_take_error() {
    let mut tsm = ServerTsm::new();
    let peer = test_peer(1);
    let (id, rx) = tsm.allocate(peer.clone()).unwrap();
    assert!(tsm.record_result(&peer.0, peer.1.as_ref(), id, CovAckResult::Error));
    // Oneshot delivers immediately
    assert_eq!(rx.blocking_recv(), Ok(CovAckResult::Error));
}

#[test]
fn server_tsm_record_nonexistent_is_noop() {
    let mut tsm = ServerTsm::new();
    // Recording a result for an ID with no receiver is a no-op
    assert!(!tsm.record_result(&test_mac(1), None, 99, CovAckResult::Ack));
    assert!(tsm.pending.is_empty());
}

#[test]
fn server_tsm_remove_cleans_up() {
    let mut tsm = ServerTsm::new();
    let peer = test_peer(1);
    let (id, _rx) = tsm.allocate(peer.clone()).unwrap();
    tsm.remove(&peer, id);
    assert!(!tsm.pending.contains_key(&(peer.0, peer.1, id)));
}

#[test]
fn server_tsm_multiple_pending() {
    let mut tsm = ServerTsm::new();
    let peer = test_peer(1);
    let (id1, rx1) = tsm.allocate(peer.clone()).unwrap();
    let (id2, rx2) = tsm.allocate(peer.clone()).unwrap();
    let (id3, rx3) = tsm.allocate(peer.clone()).unwrap();

    assert!(tsm.record_result(&peer.0, peer.1.as_ref(), id2, CovAckResult::Error));
    assert!(tsm.record_result(&peer.0, peer.1.as_ref(), id1, CovAckResult::Ack));
    assert!(tsm.record_result(&peer.0, peer.1.as_ref(), id3, CovAckResult::Ack));

    assert_eq!(rx2.blocking_recv(), Ok(CovAckResult::Error));
    assert_eq!(rx1.blocking_recv(), Ok(CovAckResult::Ack));
    assert_eq!(rx3.blocking_recv(), Ok(CovAckResult::Ack));
}

#[test]
fn server_tsm_keys_results_by_peer() {
    let mut tsm = ServerTsm::new();
    let peer_a = test_peer(1);
    let peer_b = test_peer(2);

    let rx_a = tsm.register(peer_a.clone(), 7);
    let rx_b = tsm.register(peer_b.clone(), 7);

    assert!(tsm.record_result(&peer_b.0, peer_b.1.as_ref(), 7, CovAckResult::Error));
    assert_eq!(rx_b.blocking_recv(), Ok(CovAckResult::Error));
    assert_eq!(tsm.pending.len(), 1);

    assert!(tsm.record_result(&peer_a.0, peer_a.1.as_ref(), 7, CovAckResult::Ack));
    assert_eq!(rx_a.blocking_recv(), Ok(CovAckResult::Ack));
    assert!(tsm.pending.is_empty());
}

#[tokio::test]
async fn server_tsm_timeout_cleanup_removes_pending() {
    let tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let peer = test_peer(1);
    let (id, rx) = {
        let mut tsm = tsm.lock().await;
        tsm.allocate(peer.clone()).unwrap()
    };

    assert!(tokio::time::timeout(Duration::from_millis(1), rx)
        .await
        .is_err());
    {
        let mut tsm = tsm.lock().await;
        tsm.remove(&peer, id);
        assert!(tsm.pending.is_empty());
    }
}

#[test]
fn cov_ack_result_debug_and_eq() {
    // Ensure derived traits work.
    assert_eq!(CovAckResult::Ack, CovAckResult::Ack);
    assert_ne!(CovAckResult::Ack, CovAckResult::Error);
    let _debug = format!("{:?}", CovAckResult::Ack);
}

#[test]
fn default_apdu_retries_constant() {
    assert_eq!(DEFAULT_APDU_RETRIES, 3);
}

#[test]
fn seg_receiver_timeout_is_4s() {
    assert_eq!(SEG_RECEIVER_TIMEOUT, Duration::from_secs(4));
}

#[test]
fn segmented_transaction_key_identity_matrix() {
    let router_a = test_mac(1);
    let router_b = test_mac(2);
    let remote_a = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0A]),
    };
    let remote_b = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0B]),
    };
    let other_network = NpduAddress {
        network: 101,
        mac_address: remote_a.mac_address.clone(),
    };

    let routed_a = segmented_transaction_key(&router_a, Some(&remote_a), 7);
    assert_eq!(
        routed_a,
        segmented_transaction_key(&router_b, Some(&remote_a), 7)
    );
    assert_eq!(routed_a.0, MacAddr::new());
    assert_ne!(
        routed_a,
        segmented_transaction_key(&router_a, Some(&remote_b), 7)
    );
    assert_ne!(
        routed_a,
        segmented_transaction_key(&router_a, Some(&other_network), 7)
    );
    assert_ne!(
        routed_a,
        segmented_transaction_key(&router_a, Some(&remote_a), 8)
    );

    let local_a = segmented_transaction_key(&router_a, None, 7);
    let local_b = segmented_transaction_key(&router_b, None, 7);
    assert_ne!(local_a, local_b);
    assert_ne!(local_a, routed_a);
    assert_ne!(local_a, segmented_transaction_key(&router_a, None, 8));

    for invalid in [
        NpduAddress {
            network: 0,
            mac_address: MacAddr::from_slice(&[0x0A]),
        },
        NpduAddress {
            network: 0xFFFF,
            mac_address: MacAddr::from_slice(&[0x0A]),
        },
        NpduAddress {
            network: 100,
            mac_address: MacAddr::new(),
        },
    ] {
        let key_a = segmented_transaction_key(&router_a, Some(&invalid), 7);
        let key_b = segmented_transaction_key(&router_b, Some(&invalid), 7);
        assert_ne!(key_a, key_b);
        assert_eq!(key_a.0, router_a);
        assert_eq!(key_a.1.as_ref(), Some(&invalid));
    }
}

#[test]
fn service_reject_error_preserves_reason_on_wire_response() {
    let response = BACnetServer::<BipTransport>::error_apdu_from_error(
        0x31,
        ConfirmedServiceChoice::GET_ENROLLMENT_SUMMARY,
        &Error::Reject {
            reason: RejectReason::UNDEFINED_ENUMERATION.to_raw(),
        },
    );
    assert_eq!(
        response,
        Apdu::Reject(RejectPdu {
            invoke_id: 0x31,
            reject_reason: RejectReason::UNDEFINED_ENUMERATION,
        })
    );
}

#[tokio::test]
async fn reply_tx_response_preserves_routed_npdu_destination() {
    use bacnet_encoding::apdu::decode_apdu;
    use bacnet_encoding::npdu::decode_npdu;

    let network = Arc::new(NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    )));
    let db = Arc::new(RwLock::new(ObjectDatabase::new()));
    let cov_table = Arc::new(RwLock::new(CovSubscriptionTable::new()));
    let seg_ack_senders = Arc::new(Mutex::new(HashMap::new()));
    let seg_send_permits = Arc::new(Semaphore::new(MAX_SEG_SENDERS));
    let cov_in_flight = Arc::new(Semaphore::new(1));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let notification_transactions = NotificationTransactions::new();
    let comm_state = Arc::new(AtomicU8::new(0));
    let dcc_timer = Arc::new(Mutex::new(None::<JoinHandle<()>>));
    let config = ServerConfig::default();
    let source_mac = test_mac(1);
    let routed_source = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0A, 0x0B]),
    };
    let req = ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 480,
        invoke_id: 0x31,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::AUDIT_LOG_QUERY,
        service_request: Bytes::new(),
    };
    let (tx, rx) = oneshot::channel();

    BACnetServer::<BipTransport>::handle_confirmed_request(
        &db,
        &network,
        &cov_table,
        &seg_ack_senders,
        &seg_send_permits,
        &cov_in_flight,
        &server_tsm,
        &notification_transactions,
        &comm_state,
        &dcc_timer,
        &config,
        &source_mac,
        Some(routed_source.clone()),
        req,
        Some(tx),
    )
    .await;

    let npdu = decode_npdu(rx.await.expect("reply_tx should receive response")).unwrap();
    assert_eq!(npdu.destination, Some(routed_source));
    match decode_apdu(npdu.payload).unwrap() {
        Apdu::Reject(reject) => {
            assert_eq!(reject.invoke_id, 0x31);
            assert_eq!(reject.reject_reason, RejectReason::UNRECOGNIZED_SERVICE);
        }
        other => panic!("expected Reject, got {other:?}"),
    }
}

#[test]
fn max_neg_segment_ack_retries_constant() {
    assert_eq!(MAX_NEG_SEGMENT_ACK_RETRIES, 3);
}
