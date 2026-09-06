use super::*;
use bacnet_encoding::npdu::Npdu;
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::Bytes;

#[test]
fn conservative_and_configured_npdu_envelopes_use_forwarded_source() {
    let header = forwarded_npci_len(6, 6).unwrap();
    assert_eq!(header, 21);
    assert_eq!(CONSERVATIVE_ROUTED_PATH_MAX_NPDU - header, 207);
    assert_eq!(1497 - header, 1476);
    assert_eq!(forwarded_npci_len(1, 1).unwrap(), 11);
    assert_eq!(forwarded_npci_len(4, 6).unwrap(), 19);
}

#[test]
fn forwarded_npci_rejects_unrepresentable_addresses() {
    assert!(forwarded_npci_len(256, 6).is_err());
    assert!(forwarded_npci_len(6, 0).is_err());
    assert!(forwarded_npci_len(6, 256).is_err());
}

#[tokio::test]
async fn claimed_control_cannot_complete_reused_invoke_id_for_new_owner() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(
        4,
        Duration::from_millis(10),
    ));
    let router = [2];
    let dadr = [3];
    let target = ConfirmedTarget::Routed {
        router_mac: &router,
        dest_network: 100,
        dest_mac: &dadr,
    };
    let _lease = limits.acquire(&router, 100).await.unwrap();
    let tsm_mac = target.transaction_peer().tsm_mac;
    let mut tsm = Tsm::new(TsmConfig::default());
    let old = tsm.register_transaction_with_progress(
        tsm_mac.clone(),
        7,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    limits.install_active(target, tsm_mac.clone(), 7, old.owner.clone(), 11, 0);
    assert!(limits.authorize_attempt(target, &tsm_mac, 7, &old.owner, 100));
    let claimed = limits.claim_active(&router, 100, 1).unwrap();

    assert!(tsm.cancel_transaction_for_owner(&tsm_mac, 7, &old.owner));
    let replacement = tsm.register_transaction_with_progress(
        tsm_mac.clone(),
        7,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    assert_eq!(
        tsm.complete_network_path_too_long_for_owner(
            &claimed.tsm_mac,
            claimed.invoke_id,
            &claimed.owner,
            100,
        ),
        CompletionOutcome::NoTransaction
    );
    assert!(tsm.owner_is_current(&tsm_mac, 7, &replacement.owner));
    assert_eq!(tsm.pending_count(), 1);
}

#[tokio::test]
async fn hostile_pre_tsm_paths_reclaim_idle_entries_at_capacity() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(
        3,
        Duration::from_millis(10),
    ));
    for path in 1..=40u16 {
        let lease = limits.acquire(&path.to_be_bytes(), path).await.unwrap();
        assert_eq!(lease.max_apdu(255, 255).unwrap(), 0);
        drop(lease);
        assert!(limits.state().entries.len() <= 3);
    }
    assert_eq!(limits.state().entries.len(), 3);
}

#[tokio::test]
async fn configured_and_learned_evidence_are_never_evicted_for_capacity() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(
        2,
        Duration::from_millis(10),
    ));
    let configured_key = RoutedPathKey::new(&[1], 100);
    let learned_key = RoutedPathKey::new(&[2], 200);

    let configured = limits.acquire(&[1], 100).await.unwrap();
    configured.configure(1497);
    drop(configured);

    let learned = limits.acquire(&[2], 200).await.unwrap();
    limits
        .state()
        .entries
        .get_mut(&learned_key)
        .unwrap()
        .learned = Some(LearnedPathLimit {
        exclusive_max_npdu: 300,
        observed_at: StdInstant::now(),
    });
    drop(learned);

    assert!(matches!(
        limits.acquire(&[3], 300).await,
        Err(Error::RoutedPathCapacityExceeded { capacity: 2 })
    ));
    let state = limits.state();
    assert_eq!(state.entries.len(), 2);
    assert_eq!(
        state.entries[&configured_key]
            .configured
            .as_ref()
            .unwrap()
            .max_npdu,
        1497
    );
    assert_eq!(
        state.entries[&learned_key]
            .learned
            .as_ref()
            .unwrap()
            .exclusive_max_npdu,
        300
    );
}

#[tokio::test]
async fn capacity_cleanup_never_splits_a_held_or_waiting_same_path_gate() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(
        2,
        Duration::from_millis(10),
    ));
    let held = limits.acquire(&[1], 100).await.unwrap();
    drop(limits.acquire(&[2], 200).await.unwrap());

    let waiter_limits = Arc::clone(&limits);
    let (acquired_tx, mut acquired_rx) = tokio::sync::mpsc::unbounded_channel();
    let waiter = tokio::spawn(async move {
        let _lease = waiter_limits.acquire(&[1], 100).await.unwrap();
        acquired_tx.send(()).unwrap();
    });
    tokio::task::yield_now().await;

    let replacement = limits.acquire(&[3], 300).await.unwrap();
    {
        let state = limits.state();
        assert!(state.entries.contains_key(&RoutedPathKey::new(&[1], 100)));
        assert!(!state.entries.contains_key(&RoutedPathKey::new(&[2], 200)));
    }
    assert!(acquired_rx.try_recv().is_err());

    drop(replacement);
    drop(held);
    tokio::time::timeout(Duration::from_millis(100), acquired_rx.recv())
        .await
        .expect("same-path waiter remained blocked")
        .expect("same-path waiter exited without acquiring the original gate");
    waiter.await.unwrap();
}

fn reason_4_control(router: &[u8], dnet: u16, ingress_sequence: u64) -> ReceivedNetworkControl {
    ReceivedNetworkControl {
        npdu: Npdu {
            is_network_message: true,
            message_type: Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw()),
            payload: Bytes::from(vec![
                RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
                (dnet >> 8) as u8,
                dnet as u8,
            ]),
            ..Npdu::default()
        },
        source_mac: MacAddr::from_slice(router),
        link_layer_group: false,
        data_attributes: Vec::new(),
        ingress_sequence,
    }
}

#[tokio::test]
async fn control_queued_before_activation_cannot_claim_the_new_generation() {
    let limits = Arc::new(RoutedPathLimits::with_capacity(
        2,
        Duration::from_millis(10),
    ));
    let router = [2];
    let dadr = [3];
    let target = ConfirmedTarget::Routed {
        router_mac: &router,
        dest_network: 100,
        dest_mac: &dadr,
    };
    let lease = limits.acquire(&router, 100).await.unwrap();
    let tsm_mac = target.transaction_peer().tsm_mac;
    let mut tsm = Tsm::new(TsmConfig::default());
    let mut registration = tsm.register_transaction_with_progress(
        tsm_mac.clone(),
        9,
        ConfirmedServiceChoice::READ_PROPERTY,
    );
    let tsm = Arc::new(Mutex::new(tsm));
    limits.install_active(
        target,
        tsm_mac.clone(),
        9,
        registration.owner.clone(),
        11,
        5,
    );
    assert!(limits.authorize_attempt(target, &tsm_mac, 9, &registration.owner, 100));

    limits
        .handle_network_control(&tsm, reason_4_control(&router, 100, 5))
        .await;
    assert!(tsm
        .lock()
        .await
        .owner_is_current(&tsm_mac, 9, &registration.owner));
    assert!(registration.response.try_recv().is_err());

    limits
        .handle_network_control(&tsm, reason_4_control(&router, 100, 6))
        .await;
    assert!(matches!(
        registration.response.await.unwrap(),
        TsmResponse::NetworkPathTooLong { dnet: 100 }
    ));
    lease.mark_terminal_observed();
}
