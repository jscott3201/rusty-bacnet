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
fn seg_key_distinguishes_routed_sources_behind_same_router() {
    let router = test_mac(1);
    let remote_a = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0A]),
    };
    let remote_b = NpduAddress {
        network: 100,
        mac_address: MacAddr::from_slice(&[0x0B]),
    };

    let key_a: SegKey = (router.clone(), Some(remote_a), 7);
    let key_b: SegKey = (router, Some(remote_b), 7);

    assert_ne!(key_a, key_b);
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
    let cov_in_flight = Arc::new(Semaphore::new(1));
    let server_tsm = Arc::new(Mutex::new(ServerTsm::new()));
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
        &cov_in_flight,
        &server_tsm,
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
