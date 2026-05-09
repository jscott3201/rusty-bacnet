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

#[test]
fn server_tsm_allocate_increments() {
    let mut tsm = ServerTsm::new();
    let peer = test_mac(1);
    assert_eq!(tsm.allocate(peer.clone()).0, 0);
    assert_eq!(tsm.allocate(peer.clone()).0, 1);
    assert_eq!(tsm.allocate(peer).0, 2);
}

#[test]
fn server_tsm_allocate_wraps_at_255() {
    let mut tsm = ServerTsm::new();
    let peer = test_mac(1);
    tsm.next_invoke_id = 255;
    assert_eq!(tsm.allocate(peer.clone()).0, 255);
    assert_eq!(tsm.allocate(peer).0, 0); // wraps
}

#[test]
fn server_tsm_record_and_take_ack() {
    let mut tsm = ServerTsm::new();
    let peer = test_mac(1);
    let (id, rx) = tsm.allocate(peer.clone());
    assert!(tsm.record_result(&peer, id, CovAckResult::Ack));
    // Result should be delivered via the oneshot channel
    assert_eq!(rx.blocking_recv(), Ok(CovAckResult::Ack));
}

#[test]
fn server_tsm_record_and_take_error() {
    let mut tsm = ServerTsm::new();
    let peer = test_mac(1);
    let (id, rx) = tsm.allocate(peer.clone());
    assert!(tsm.record_result(&peer, id, CovAckResult::Error));
    // Oneshot delivers immediately
    assert_eq!(rx.blocking_recv(), Ok(CovAckResult::Error));
}

#[test]
fn server_tsm_record_nonexistent_is_noop() {
    let mut tsm = ServerTsm::new();
    // Recording a result for an ID with no receiver is a no-op
    assert!(!tsm.record_result(&test_mac(1), 99, CovAckResult::Ack));
    assert!(tsm.pending.is_empty());
}

#[test]
fn server_tsm_remove_cleans_up() {
    let mut tsm = ServerTsm::new();
    let peer = test_mac(1);
    let (id, _rx) = tsm.allocate(peer.clone());
    tsm.remove(&peer, id);
    assert!(!tsm.pending.contains_key(&(peer, id)));
}

#[test]
fn server_tsm_multiple_pending() {
    let mut tsm = ServerTsm::new();
    let peer = test_mac(1);
    let (id1, rx1) = tsm.allocate(peer.clone());
    let (id2, rx2) = tsm.allocate(peer.clone());
    let (id3, rx3) = tsm.allocate(peer.clone());

    assert!(tsm.record_result(&peer, id2, CovAckResult::Error));
    assert!(tsm.record_result(&peer, id1, CovAckResult::Ack));
    assert!(tsm.record_result(&peer, id3, CovAckResult::Ack));

    assert_eq!(rx2.blocking_recv(), Ok(CovAckResult::Error));
    assert_eq!(rx1.blocking_recv(), Ok(CovAckResult::Ack));
    assert_eq!(rx3.blocking_recv(), Ok(CovAckResult::Ack));
}

#[test]
fn server_tsm_keys_results_by_peer() {
    let mut tsm = ServerTsm::new();
    let peer_a = test_mac(1);
    let peer_b = test_mac(2);

    let rx_a = tsm.register(peer_a.clone(), 7);
    let rx_b = tsm.register(peer_b.clone(), 7);

    assert!(tsm.record_result(&peer_b, 7, CovAckResult::Error));
    assert_eq!(rx_b.blocking_recv(), Ok(CovAckResult::Error));
    assert_eq!(tsm.pending.len(), 1);

    assert!(tsm.record_result(&peer_a, 7, CovAckResult::Ack));
    assert_eq!(rx_a.blocking_recv(), Ok(CovAckResult::Ack));
    assert!(tsm.pending.is_empty());
}

#[tokio::test]
async fn server_tsm_timeout_cleanup_removes_pending() {
    let tsm = Arc::new(Mutex::new(ServerTsm::new()));
    let peer = test_mac(1);
    let (id, rx) = {
        let mut tsm = tsm.lock().await;
        tsm.allocate(peer.clone())
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
fn max_neg_segment_ack_retries_constant() {
    assert_eq!(MAX_NEG_SEGMENT_ACK_RETRIES, 3);
}
