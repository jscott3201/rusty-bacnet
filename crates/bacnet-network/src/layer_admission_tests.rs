use super::*;
use bacnet_transport::port::ReceivedNpdu;
use tokio::sync::mpsc;
use tokio::time::{timeout, Duration};

// Injection retains real ReceivedNpdu ownership, including the non-cloneable
// reply sender. No socket, wall clock, or additional dispatch task is involved.
struct IngressTransport {
    incoming: Option<mpsc::Receiver<ReceivedNpdu>>,
    outgoing: mpsc::Sender<Bytes>,
}

impl TransportPort for IngressTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.incoming
            .take()
            .ok_or_else(|| Error::Encoding("already started".into()))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        self.outgoing
            .try_send(Bytes::copy_from_slice(npdu))
            .map_err(|_| Error::Encoding("outgoing queue unavailable".into()))
    }

    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.send_unicast(npdu, &[]).await
    }

    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

fn fixture() -> (
    NetworkLayer<IngressTransport>,
    mpsc::Sender<ReceivedNpdu>,
    mpsc::Receiver<Bytes>,
) {
    // Only the test's injected transport backlog is larger; production receive
    // capacities remain 256. This can hold both saturated queues plus barriers.
    let (tx, rx) = mpsc::channel(1024);
    let (outgoing, wire) = mpsc::channel(8);
    (
        NetworkLayer::new(IngressTransport {
            incoming: Some(rx),
            outgoing,
        }),
        tx,
        wire,
    )
}

fn incoming(control: bool, id: u16) -> ReceivedNpdu {
    let payload = if control {
        vec![4, (id >> 8) as u8, id as u8]
    } else {
        id.to_be_bytes().to_vec()
    };
    let npdu = Npdu {
        is_network_message: control,
        message_type: control.then_some(3),
        expecting_reply: !control,
        source: Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[0x55]),
        }),
        payload: Bytes::from(payload),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &npdu).unwrap();
    ReceivedNpdu {
        npdu: bytes.freeze(),
        source_mac: MacAddr::from_slice(&[2]),
        link_layer_group: true,
        data_attributes: vec![DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34],
        }],
        reply_tx: None,
    }
}

fn assert_apdu(apdu: &ReceivedApdu, id: u16) {
    assert_apdu_from(apdu, id, &[2]);
}

fn assert_apdu_from(apdu: &ReceivedApdu, id: u16, source: &[u8]) {
    let expected = incoming(false, id);
    assert_eq!(apdu.apdu.as_ref(), id.to_be_bytes());
    assert_eq!(apdu.source_mac.as_slice(), source);
    assert_eq!(
        apdu.source_network,
        Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[0x55]),
        })
    );
    assert!(apdu.link_layer_group);
    assert!(apdu.is_group);
    assert_eq!(apdu.data_attributes, expected.data_attributes);
}

// Sixteen sources with sixteen queued APDUs each still fill the shared queue.
fn balanced_incoming(id: u16) -> ReceivedNpdu {
    let mut arrival = incoming(false, id);
    arrival.source_mac = MacAddr::from_slice(&[2 + (id / 16) as u8]);
    arrival
}

fn assert_balanced_apdu(apdu: &ReceivedApdu, id: u16) {
    assert_apdu_from(apdu, id, &[2 + (id / 16) as u8]);
}

fn assert_control(control: &ReceivedNetworkControl, id: u16, sequence: u64) {
    let expected = incoming(true, id);
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &control.npdu).unwrap();
    assert_eq!(bytes.freeze(), expected.npdu);
    assert_eq!(control.source_mac, expected.source_mac);
    assert_eq!(control.link_layer_group, expected.link_layer_group);
    assert_eq!(control.data_attributes, expected.data_attributes);
    assert_eq!(control.ingress_sequence, sequence);
}

async fn receive<T>(rx: &mut AdmissionReceiver<T>) -> T {
    timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("dispatch stalled")
        .expect("receiver closed unexpectedly")
}

#[tokio::test(start_paused = true)]
async fn full_apdu_queue_drops_arrival_releases_reply_and_allows_control_progress() {
    let (mut network, tx, mut wire) = fixture();
    let mut controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    let (reply_tx, mut accepted_reply) = oneshot::channel();
    let mut first = balanced_incoming(0);
    first.reply_tx = Some(reply_tx);
    tx.send(first).await.unwrap();
    for id in 1..256 {
        tx.send(balanced_incoming(id)).await.unwrap();
    }
    let (reply_tx, rejected_reply) = oneshot::channel();
    let mut overflow = balanced_incoming(256);
    overflow.reply_tx = Some(reply_tx);
    tx.send(overflow).await.unwrap();
    tx.send(incoming(true, 42)).await.unwrap();

    // Receiving the later control is a FIFO ingress barrier, without consuming
    // any APDU. The 257th APDU must already have been dropped, not deferred.
    assert_control(&receive(&mut controls).await, 42, 1);
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 1,
            fairness_drops: 0,
            closed_drops: 0,
        }
    );
    assert!(timeout(Duration::from_secs(1), rejected_reply)
        .await
        .unwrap()
        .is_err());
    assert_eq!(
        accepted_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );

    let first = apdus.try_recv().unwrap();
    assert_apdu(&first, 0);
    first
        .reply_tx
        .unwrap()
        .send(Bytes::from_static(b"reply"))
        .unwrap();
    assert_eq!(accepted_reply.await.unwrap(), Bytes::from_static(b"reply"));
    for id in 1..256 {
        assert_balanced_apdu(&apdus.try_recv().unwrap(), id);
    }
    assert!(matches!(
        apdus.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    assert_eq!(counters.snapshot().current_depth, 0);
    assert_eq!(counters.snapshot().high_water, 256);

    tx.send(incoming(false, 257)).await.unwrap();
    assert_apdu(&receive(&mut apdus).await, 257);
    assert_eq!(counters.snapshot().full_drops, 1);
    assert_eq!(controls.counters().snapshot().full_drops, 0);
    assert!(matches!(
        wire.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn full_control_queue_drops_arrival_and_allows_apdu_progress() {
    let (mut network, tx, mut wire) = fixture();
    let mut controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    let counters = controls.counters();
    let mut apdus = network.start_with_admission().await.unwrap();
    for id in 0..257 {
        tx.send(incoming(true, id)).await.unwrap();
    }
    tx.send(incoming(false, 42)).await.unwrap();
    assert_apdu(&receive(&mut apdus).await, 42);
    assert_eq!(network.network_control_ingress_sequence(), 257);
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 1,
            fairness_drops: 0,
            closed_drops: 0,
        }
    );
    for id in 0..256 {
        assert_control(&controls.try_recv().unwrap(), id, u64::from(id) + 1);
    }
    assert!(matches!(
        controls.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    tx.send(incoming(true, 257)).await.unwrap();
    assert_control(&receive(&mut controls).await, 257, 258);
    assert_eq!(counters.snapshot().current_depth, 0);
    assert_eq!(counters.snapshot().high_water, 256);
    assert_eq!(counters.snapshot().full_drops, 1);
    assert_eq!(apdus.counters().snapshot().full_drops, 0);
    assert!(matches!(
        wire.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn closed_control_stream_counts_once_then_resumes_discard_with_legacy_apdus() {
    let (mut network, tx, mut wire) = fixture();
    let controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    let counters = controls.counters();
    let mut apdus: mpsc::Receiver<ReceivedApdu> = network.start().await.unwrap();
    drop(controls);
    for id in 0..3 {
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut control = incoming(true, id);
        control.reply_tx = Some(reply_tx);
        tx.send(control).await.unwrap();
        assert!(timeout(Duration::from_secs(1), reply_rx)
            .await
            .unwrap()
            .is_err());
    }
    tx.send(incoming(false, 42)).await.unwrap();
    let apdu = timeout(Duration::from_secs(1), apdus.recv())
        .await
        .unwrap()
        .unwrap();
    assert_apdu(&apdu, 42);
    assert_eq!(network.network_control_ingress_sequence(), 1);
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            closed_drops: 1,
            ..Default::default()
        }
    );
    assert!(matches!(
        wire.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn closed_apdu_receiver_releases_reply_and_ends_dispatch_with_legacy_controls() {
    let (mut network, tx, mut wire) = fixture();
    let mut controls: mpsc::Receiver<ReceivedNetworkControl> =
        network.enable_network_control_receiver().unwrap();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    apdus.close();
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut apdu = incoming(false, 42);
    apdu.reply_tx = Some(reply_tx);
    tx.send(apdu).await.unwrap();
    tx.send(incoming(true, 42)).await.unwrap();

    assert!(timeout(Duration::from_secs(1), reply_rx)
        .await
        .unwrap()
        .is_err());
    assert!(timeout(Duration::from_secs(1), controls.recv())
        .await
        .unwrap()
        .is_none());
    timeout(Duration::from_secs(1), tx.closed()).await.unwrap();
    assert_eq!(network.network_control_ingress_sequence(), 0);
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            closed_drops: 1,
            ..Default::default()
        }
    );
    assert!(matches!(
        wire.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn no_control_opt_in_discards_without_sequence_reply_or_apdu_changes() {
    let (mut network, tx, mut wire) = fixture();
    let mut apdus = network.start_with_admission().await.unwrap();
    for id in 0..257 {
        tx.send(incoming(true, id)).await.unwrap();
    }
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut control = incoming(true, 257);
    control.reply_tx = Some(reply_tx);
    tx.send(control).await.unwrap();
    tx.send(incoming(false, 42)).await.unwrap();
    assert_apdu(&receive(&mut apdus).await, 42);
    assert!(reply_rx.await.is_err());
    assert_eq!(network.network_control_ingress_sequence(), 0);
    assert_eq!(
        apdus.counters().snapshot(),
        QueueAdmissionSnapshot {
            high_water: 1,
            ..Default::default()
        }
    );
    assert!(matches!(
        apdus.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        wire.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn stop_with_both_queues_full_is_bounded_and_preserves_drain_and_drop_accounting() {
    let (mut network, tx, _wire) = fixture();
    let mut controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    let mut apdus = network.start_with_admission().await.unwrap();
    let apdu_counters = apdus.counters();
    let control_counters = controls.counters();
    let (reply_tx, mut queued_reply) = oneshot::channel();
    let mut first = balanced_incoming(0);
    first.reply_tx = Some(reply_tx);
    tx.send(first).await.unwrap();
    for id in 1..256 {
        tx.send(balanced_incoming(id)).await.unwrap();
    }
    for id in 0..256 {
        tx.send(incoming(true, id)).await.unwrap();
    }
    // A control's reply sender is always released at the end of its ingress
    // iteration. Awaiting the overflow's closure is a barrier with both full.
    let (reply_tx, barrier) = oneshot::channel();
    let mut overflow = incoming(true, 256);
    overflow.reply_tx = Some(reply_tx);
    tx.send(overflow).await.unwrap();
    assert!(timeout(Duration::from_secs(1), barrier)
        .await
        .unwrap()
        .is_err());
    assert_eq!(apdu_counters.snapshot().current_depth, 256);
    assert_eq!(control_counters.snapshot().current_depth, 256);
    timeout(Duration::from_secs(1), network.stop())
        .await
        .unwrap()
        .unwrap();
    assert!(tx.is_closed());
    assert_eq!(
        queued_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    assert_eq!(apdu_counters.snapshot().current_depth, 256);

    assert_apdu(&receive(&mut apdus).await, 0);
    assert!(queued_reply.await.is_err());
    for id in 0..256 {
        assert_control(&receive(&mut controls).await, id, u64::from(id) + 1);
    }
    assert!(controls.recv().await.is_none());
    assert_eq!(control_counters.snapshot().current_depth, 0);
    assert_eq!(apdu_counters.snapshot().current_depth, 255);
    drop(apdus);
    assert_eq!(
        apdu_counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            ..Default::default()
        }
    );
    assert_eq!(
        control_counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 256,
            full_drops: 1,
            ..Default::default()
        }
    );
}

#[tokio::test(start_paused = true)]
async fn cancelled_receive_does_not_consume_or_change_counters() {
    let (mut network, tx, _wire) = fixture();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    assert!(timeout(Duration::from_secs(1), apdus.recv()).await.is_err());
    assert_eq!(counters.snapshot(), QueueAdmissionSnapshot::default());
    tx.send(incoming(false, 42)).await.unwrap();
    assert_apdu(&receive(&mut apdus).await, 42);
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            high_water: 1,
            ..Default::default()
        }
    );
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn legacy_receivers_also_drop_new_items_without_cross_queue_stalls() {
    let (mut network, tx, _wire) = fixture();
    let mut controls: mpsc::Receiver<ReceivedNetworkControl> =
        network.enable_network_control_receiver().unwrap();
    let mut apdus: mpsc::Receiver<ReceivedApdu> = network.start().await.unwrap();
    for id in 0..257 {
        tx.send(incoming(false, id)).await.unwrap();
    }
    tx.send(incoming(true, 42)).await.unwrap();
    let control = timeout(Duration::from_secs(1), controls.recv())
        .await
        .unwrap()
        .unwrap();
    assert_control(&control, 42, 1);
    assert_eq!(apdus.len(), 256);
    for id in 0..256 {
        assert_apdu(&apdus.try_recv().unwrap(), id);
    }
    for id in 0..257 {
        tx.send(incoming(true, id)).await.unwrap();
    }
    tx.send(incoming(false, 42)).await.unwrap();
    let apdu = timeout(Duration::from_secs(1), apdus.recv())
        .await
        .unwrap()
        .unwrap();
    assert_apdu(&apdu, 42);
    assert_eq!(controls.len(), 256);
    for id in 0..256 {
        assert_control(&controls.try_recv().unwrap(), id, u64::from(id) + 2);
    }
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn admission_opt_in_preserves_control_enable_guards() {
    let (mut network, _tx, _wire) = fixture();
    let _controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    assert_eq!(
        network
            .enable_network_control_receiver()
            .unwrap_err()
            .to_string(),
        "encoding error: network-control receiver is already enabled"
    );
    assert_eq!(
        network
            .enable_network_control_receiver_with_admission()
            .unwrap_err()
            .to_string(),
        "encoding error: network-control receiver is already enabled"
    );
    let _apdus = network.start_with_admission().await.unwrap();
    assert_eq!(
        network
            .enable_network_control_receiver_with_admission()
            .unwrap_err()
            .to_string(),
        "encoding error: network-control receiver must be enabled before NetworkLayer::start"
    );
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn closing_full_receivers_preserves_depth_until_drained_and_counts_closed_not_full() {
    let (mut network, tx, _wire) = fixture();
    let mut controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    let mut apdus = network.start_with_admission().await.unwrap();
    let apdu_counters = apdus.counters();
    let control_counters = controls.counters();
    for id in 0..256 {
        tx.send(incoming(true, id)).await.unwrap();
    }
    tx.send(incoming(false, 0)).await.unwrap();
    assert_apdu(&receive(&mut apdus).await, 0);
    controls.close();
    assert_eq!(control_counters.snapshot().current_depth, 256);
    tx.send(incoming(true, 256)).await.unwrap();
    tx.send(incoming(false, 0)).await.unwrap();
    assert_apdu(&receive(&mut apdus).await, 0);
    assert_eq!(control_counters.snapshot().closed_drops, 1);
    assert_eq!(control_counters.snapshot().full_drops, 0);
    assert_eq!(network.network_control_ingress_sequence(), 257);

    let (reply_tx, queued_reply) = oneshot::channel();
    let mut first = balanced_incoming(0);
    first.reply_tx = Some(reply_tx);
    tx.send(first).await.unwrap();
    for id in 1..256 {
        tx.send(balanced_incoming(id)).await.unwrap();
    }
    // Controls now use the discard path; closure of this reply is a barrier.
    let (reply_tx, barrier) = oneshot::channel();
    let mut control = incoming(true, 257);
    control.reply_tx = Some(reply_tx);
    tx.send(control).await.unwrap();
    assert!(timeout(Duration::from_secs(1), barrier)
        .await
        .unwrap()
        .is_err());
    apdus.close();
    assert_eq!(apdu_counters.snapshot().current_depth, 256);
    tx.send(incoming(false, 256)).await.unwrap();
    timeout(Duration::from_secs(1), tx.closed()).await.unwrap();
    assert_eq!(apdu_counters.snapshot().closed_drops, 1);
    assert_eq!(apdu_counters.snapshot().full_drops, 0);
    assert_eq!(apdu_counters.snapshot().current_depth, 256);
    assert_eq!(control_counters.snapshot().closed_drops, 1);
    assert_eq!(network.network_control_ingress_sequence(), 257);
    for id in 0..256 {
        assert_control(&receive(&mut controls).await, id, u64::from(id) + 1);
    }
    assert!(controls.recv().await.is_none());
    drop(apdus);
    assert!(queued_reply.await.is_err());
    assert_eq!(apdu_counters.snapshot().current_depth, 0);
    assert_eq!(apdu_counters.snapshot().high_water, 256);
    network.stop().await.unwrap();
}
