use super::*;
use crate::sc_frame::ScFunction;

fn item(source: &MacAddr) -> ReceivedNpdu {
    ReceivedNpdu {
        direct_response: None,
        npdu: Bytes::from_static(&[0x01, 0x00, 0x30]),
        source_mac: source.clone(),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn hub_source(id: u8) -> (ScIngressPath, MacAddr) {
    (ScIngressPath::Hub, MacAddr::from_slice(&[id; 6]))
}

fn admitted(
    admission: &ScNpduAdmission,
    tx: &mpsc::Sender<ReceivedNpdu>,
    path: ScIngressPath,
    source: &MacAddr,
) {
    match admission.admit(tx, path, source, item(source), true) {
        AdmissionOutcome::Admitted => {}
        outcome => panic!("expected admission, got {outcome:?}"),
    }
}

fn fairness_dropped(
    admission: &ScNpduAdmission,
    tx: &mpsc::Sender<ReceivedNpdu>,
    path: ScIngressPath,
    source: &MacAddr,
) {
    match admission.admit(tx, path, source, item(source), true) {
        AdmissionOutcome::FairnessDrop(_) => {}
        outcome => panic!("expected fairness drop, got {outcome:?}"),
    }
}

fn full_dropped(
    admission: &ScNpduAdmission,
    tx: &mpsc::Sender<ReceivedNpdu>,
    path: ScIngressPath,
    source: &MacAddr,
) {
    match admission.admit(tx, path, source, item(source), true) {
        AdmissionOutcome::FullDrop(_) => {}
        outcome => panic!("expected full drop, got {outcome:?}"),
    }
}

fn encapsulated(dest: Option<Vmac>) -> ScMessage {
    ScMessage {
        function: ScFunction::EncapsulatedNpdu,
        message_id: 0x1234,
        originating_vmac: None,
        destination_vmac: dest,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: Bytes::from_static(&[0x01, 0x00, 0x30]),
    }
}

#[test]
fn policy_defaults_to_one_eighth_of_the_queue_and_validates_bounds() {
    assert_eq!(
        ScNpduAdmissionPolicy::default().per_origin_limit,
        DEFAULT_NPDU_PER_ORIGIN_LIMIT
    );
    assert_eq!(SC_NPDU_QUEUE_CAPACITY, 256);
    assert_eq!(DEFAULT_NPDU_PER_ORIGIN_LIMIT, 32);
    ScNpduAdmissionPolicy::default().validate().unwrap();
    for limit in [1, 4, 32, SC_NPDU_QUEUE_CAPACITY - 1, SC_NPDU_QUEUE_CAPACITY] {
        ScNpduAdmissionPolicy {
            per_origin_limit: limit,
            ..Default::default()
        }
        .validate()
        .unwrap();
    }
    for limit in [0, SC_NPDU_QUEUE_CAPACITY + 1, usize::MAX] {
        let err = ScNpduAdmissionPolicy {
            per_origin_limit: limit,
            ..Default::default()
        }
        .validate()
        .unwrap_err();
        assert!(err.to_string().contains("per-origin limit"), "{err}");
    }
}

#[test]
fn single_source_flood_caps_at_quota_and_releases_on_drain() {
    let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
    let (path, source) = hub_source(2);
    for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
        admitted(&admission, &tx, path, &source);
    }
    assert_eq!(
        admission.test_live_entries(),
        (1, DEFAULT_NPDU_PER_ORIGIN_LIMIT)
    );
    fairness_dropped(&admission, &tx, path, &source);
    assert_eq!(admission.drop_counts().fairness_drops, 1);
    assert_eq!(admission.drop_counts().full_drops, 0);
    // Draining releases quota: the previously capped source progresses.
    rx.try_recv().unwrap();
    rx.try_recv().unwrap();
    admitted(&admission, &tx, path, &source);
    admitted(&admission, &tx, path, &source);
    fairness_dropped(&admission, &tx, path, &source);
    assert_eq!(
        admission.test_live_entries(),
        (1, DEFAULT_NPDU_PER_ORIGIN_LIMIT)
    );
    assert_eq!(admission.drop_counts().fairness_drops, 2);
}

#[test]
fn eight_origins_compose_to_capacity_and_fairness_precedes_full() {
    let (tx, _rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
    let origins = SC_NPDU_QUEUE_CAPACITY / DEFAULT_NPDU_PER_ORIGIN_LIMIT;
    assert_eq!(origins, 8);
    for id in 0..origins as u8 {
        let (path, source) = hub_source(id);
        for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
            admitted(&admission, &tx, path, &source);
        }
    }
    assert_eq!(
        admission.test_live_entries(),
        (origins, SC_NPDU_QUEUE_CAPACITY)
    );
    // Below quota but globally full: aggregate drop.
    let fresh = MacAddr::from_slice(&[0xFF; 6]);
    full_dropped(&admission, &tx, ScIngressPath::Hub, &fresh);
    // Over quota AND globally full: fairness takes precedence.
    let (path, capped) = hub_source(0);
    fairness_dropped(&admission, &tx, path, &capped);
    assert_eq!(
        admission.drop_counts(),
        ScNpduDropCounts {
            fairness_drops: 1,
            full_drops: 1,
            closed_drops: 0,
            ..Default::default()
        }
    );
}

#[test]
fn unicast_and_broadcast_share_one_quota_while_paths_stay_separate() {
    let (tx, rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
    let source: Vmac = [0x22; 6];
    let npdu = Bytes::from_static(&[0x01, 0x00, 0x30]);
    let quota = DEFAULT_NPDU_PER_ORIGIN_LIMIT;
    // Unicast- and broadcast-shaped hub NPDUs, half each, share the
    // single hub-path quota; neither shape bypasses as "broadcast".
    for index in 0..quota {
        let dest = (index % 2 == 1).then_some(BROADCAST_VMAC);
        admission.admit_hub_relayed(&tx, &encapsulated(dest), npdu.clone(), source);
    }
    assert_eq!(rx.len(), quota);
    admission.admit_hub_relayed(&tx, &encapsulated(None), npdu.clone(), source);
    admission.admit_hub_relayed(
        &tx,
        &encapsulated(Some(BROADCAST_VMAC)),
        npdu.clone(),
        source,
    );
    assert_eq!(rx.len(), quota);
    assert_eq!(admission.drop_counts().fairness_drops, 2);
    assert_eq!(admission.drop_counts().full_drops, 0);
    // Identical bytes on the direct path are a separate key and progress.
    let peer_addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
    admission.admit_direct_peer(
        &tx,
        &encapsulated(None),
        npdu.clone(),
        DirectPeer {
            vmac: source,
            addr: peer_addr,
            identity: crate::port::DirectScIdentity::verified([1; 32], 1),
            response: None,
        },
    );
    admission.admit_direct_peer(
        &tx,
        &encapsulated(None),
        npdu.clone(),
        DirectPeer {
            vmac: source,
            addr: peer_addr,
            identity: crate::port::DirectScIdentity::verified([1; 32], 1),
            response: None,
        },
    );
    assert_eq!(rx.len(), quota + 2);
    assert_eq!(admission.drop_counts().fairness_drops, 2);
    assert_eq!(admission.test_live_entries(), (2, quota + 2));
}

#[test]
fn churn_cannot_grow_map_beyond_queued_items() {
    let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
    for round in 0..3u16 {
        for id in 0..512u16 {
            let mut bytes = [0u8; 6];
            bytes[0..2].copy_from_slice(&round.to_be_bytes());
            bytes[2..4].copy_from_slice(&id.to_be_bytes());
            let source = MacAddr::from_slice(&bytes);
            let _ = admission.admit(&tx, ScIngressPath::Hub, &source, item(&source), true);
        }
        // Every queued item has at most one key; overflow is Full, never
        // fairness (each churned key appears once).
        let capacity = SC_NPDU_QUEUE_CAPACITY;
        assert_eq!(admission.test_live_entries(), (capacity, capacity));
        assert_eq!(admission.drop_counts().fairness_drops, 0);
        assert_eq!(
            admission.drop_counts().full_drops,
            u64::from(round + 1) * (512 - capacity as u64)
        );
        for _ in 0..capacity / 2 {
            rx.try_recv().unwrap();
        }
        // Half-drained: the next admission reaps before charging.
        let fresh = MacAddr::from_slice(&[0xEE; 6]);
        admitted(&admission, &tx, ScIngressPath::Hub, &fresh);
        let (entries, queued) = admission.test_live_entries();
        assert!(entries <= queued, "{entries} entries vs {queued} queued");
        assert!(queued <= SC_NPDU_QUEUE_CAPACITY);
        for _ in 0..capacity / 2 + 1 {
            rx.try_recv().unwrap();
        }
        // Fully drained: the next admission reaps every churned key, so
        // only the fresh key remains live.
        let fresh_tail = MacAddr::from_slice(&[0xEF; 6]);
        admitted(&admission, &tx, ScIngressPath::Hub, &fresh_tail);
        assert_eq!(admission.test_live_entries(), (1, 1));
        rx.try_recv().unwrap();
    }
}

#[test]
fn close_and_drop_clear_map_and_count_closed_first() {
    for drop_receiver in [false, true] {
        let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
        let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
        let (path, source) = hub_source(2);
        for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
            admitted(&admission, &tx, path, &source);
        }
        assert_eq!(
            admission.test_live_entries(),
            (1, DEFAULT_NPDU_PER_ORIGIN_LIMIT)
        );
        if drop_receiver {
            drop(rx);
        } else {
            rx.close();
        }
        // Closed takes precedence over fairness even for a capped source.
        match admission.admit(&tx, path, &source, item(&source), true) {
            AdmissionOutcome::ClosedDrop(_) => {}
            outcome => panic!("expected closed drop, got {outcome:?}"),
        }
        assert_eq!(admission.test_live_entries(), (0, 0));
        assert_eq!(
            admission.drop_counts(),
            ScNpduDropCounts {
                closed_drops: 1,
                ..Default::default()
            }
        );
    }
}

#[test]
fn counters_saturate_without_affecting_depth_or_other_drops() {
    let (tx, _rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
    let (path, source) = hub_source(2);
    for _ in 0..DEFAULT_NPDU_PER_ORIGIN_LIMIT {
        admitted(&admission, &tx, path, &source);
    }
    admission.test_set_fairness_drops(u64::MAX - 1);
    fairness_dropped(&admission, &tx, path, &source);
    fairness_dropped(&admission, &tx, path, &source);
    assert_eq!(admission.drop_counts().fairness_drops, u64::MAX);
    assert_eq!(admission.drop_counts().full_drops, 0);
    assert_eq!(admission.drop_counts().closed_drops, 0);
    assert_eq!(
        admission.test_live_entries(),
        (1, DEFAULT_NPDU_PER_ORIGIN_LIMIT)
    );
}

/// The `dropped` field of each admission-drop warning, and the number of
/// drops logged at debug instead.
fn drop_logs(logs: &crate::log_capture::LogCapture) -> (Vec<String>, usize) {
    let warnings = logs
        .matching("SC transport: NPDU admission dropped incoming NPDUs")
        .into_iter()
        .map(|event| {
            assert_eq!(event.level, tracing::Level::WARN);
            event.fields["dropped"].clone()
        })
        .collect();
    let debug = logs
        .levels("SC transport: NPDU admission dropped an incoming NPDU")
        .into_iter()
        .inspect(|level| assert_eq!(*level, tracing::Level::DEBUG))
        .count();
    (warnings, debug)
}

#[tokio::test(start_paused = true)]
async fn quota_and_full_drops_warn_at_most_once_per_interval_with_the_count() {
    use crate::warn_throttle::WARNING_INTERVAL;
    let (logs, _guard) = crate::log_capture::LogCapture::start();
    let (tx, _rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy {
        per_origin_limit: 1,
        ..Default::default()
    });
    let npdu = Bytes::from_static(&[0x01, 0x00, 0x30]);
    let hub = |source: Vmac| {
        admission.admit_hub_relayed(&tx, &encapsulated(None), npdu.clone(), source);
    };
    // Admitted, then three over the quota: one warning, two at debug.
    for _ in 0..4 {
        hub([0x22; 6]);
    }
    assert_eq!(drop_logs(&logs), (vec!["1".to_owned()], 2));
    tokio::time::advance(WARNING_INTERVAL - std::time::Duration::from_millis(1)).await;
    hub([0x22; 6]);
    assert_eq!(drop_logs(&logs), (vec!["1".to_owned()], 3));
    // The next warning counts the three held back and its own drop.
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    hub([0x22; 6]);
    assert_eq!(drop_logs(&logs).0, ["1", "4"]);
    // Full drops share the throttle: fill the queue with fresh origins.
    for id in 1..SC_NPDU_QUEUE_CAPACITY as u16 {
        let [high, low] = id.to_be_bytes();
        hub([0x30, high, low, 0, 0, 0]);
    }
    hub([0x40; 6]);
    tokio::time::advance(WARNING_INTERVAL).await;
    hub([0x41; 6]);
    assert_eq!(
        drop_logs(&logs),
        (vec!["1".into(), "4".into(), "2".into()], 4)
    );
    // Every drop is still counted.
    assert_eq!(
        admission.drop_counts(),
        ScNpduDropCounts {
            fairness_drops: 5,
            full_drops: 2,
            closed_drops: 0,
            ..Default::default()
        }
    );
}

#[tokio::test]
async fn cancelled_recv_changes_no_counts() {
    let (tx, mut rx) = mpsc::channel(SC_NPDU_QUEUE_CAPACITY);
    let admission = ScNpduAdmission::new(ScNpduAdmissionPolicy::default());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(10), rx.recv())
            .await
            .is_err()
    );
    assert_eq!(admission.drop_counts(), ScNpduDropCounts::default());
    assert_eq!(admission.test_live_entries(), (0, 0));
    let (path, source) = hub_source(2);
    admitted(&admission, &tx, path, &source);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
            .await
            .expect("admitted item must arrive")
            .expect("channel open")
            .source_mac,
        source
    );
    assert_eq!(admission.test_live_entries(), (1, 1));
    // The dequeue is observed on the next admission, not by the recv.
    let other = MacAddr::from_slice(&[0x03; 6]);
    admitted(&admission, &tx, ScIngressPath::Hub, &other);
    assert_eq!(admission.test_live_entries(), (1, 1));
    assert_eq!(admission.drop_counts(), ScNpduDropCounts::default());
}

#[path = "control_rate_admission_tests.rs"]
mod control_rate_admission;
