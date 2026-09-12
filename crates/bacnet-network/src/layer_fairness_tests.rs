use super::*;
use bacnet_transport::port::ReceivedNpdu;
use tokio::time::{timeout, Duration};

// Same owned-NPDU fake-transport pattern as layer_admission_tests: the real
// dispatch task receives payload, provenance, attributes and the reply sender.
struct IngressTransport(Option<mpsc::Receiver<ReceivedNpdu>>);

impl TransportPort for IngressTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        Ok(self.0.take().expect("transport already started"))
    }

    async fn stop(&mut self) -> Result<(), Error> {
        Ok(())
    }

    async fn send_unicast(&self, _npdu: &[u8], _mac: &[u8]) -> Result<(), Error> {
        panic!("admission must not send wire replies or rejections")
    }

    async fn send_broadcast(&self, _npdu: &[u8]) -> Result<(), Error> {
        panic!("admission must not send wire replies or rejections")
    }

    fn local_mac(&self) -> &[u8] {
        &[1]
    }
}

fn fixture() -> (NetworkLayer<IngressTransport>, mpsc::Sender<ReceivedNpdu>) {
    let (tx, rx) = mpsc::channel(1024);
    (NetworkLayer::new(IngressTransport(Some(rx))), tx)
}

fn incoming(source: &[u8], id: u16) -> ReceivedNpdu {
    let npdu = Npdu {
        expecting_reply: true,
        // Deliberately identical routed source: fairness must use the immediate
        // transport MAC, not the NPDU SNET/SADR or payload.
        source: Some(NpduAddress {
            network: 200,
            mac_address: MacAddr::from_slice(&[0x55]),
        }),
        payload: Bytes::copy_from_slice(&id.to_be_bytes()),
        ..Npdu::default()
    };
    let mut bytes = BytesMut::new();
    encode_npdu(&mut bytes, &npdu).unwrap();
    ReceivedNpdu {
        npdu: bytes.freeze(),
        source_mac: MacAddr::from_slice(source),
        link_layer_group: true,
        data_attributes: vec![DataAttribute {
            option_type: 31,
            must_understand: false,
            data: vec![0x12, 0x34],
        }],
        reply_tx: None,
    }
}

fn control() -> ReceivedNpdu {
    let mut arrival = incoming(&[2], 0);
    // Independent Router-Busy network-message vector.
    arrival.npdu = Bytes::from_static(&[1, 0x80, 4]);
    arrival
}

async fn barrier(tx: &mpsc::Sender<ReceivedNpdu>) {
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut arrival = control();
    arrival.reply_tx = Some(reply_tx);
    tx.send(arrival).await.unwrap();
    // The control reply is released after all earlier ingress has dispatched,
    // without consuming any APDU or relying on scheduling sleeps.
    assert!(timeout(Duration::from_secs(1), reply_rx)
        .await
        .expect("dispatch stalled")
        .is_err());
}

fn assert_apdu(apdu: &ReceivedApdu, source: &[u8], id: u16) {
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
    assert_eq!(apdu.data_attributes, incoming(source, id).data_attributes);
}

fn assert_sources(counters: &QueueAdmissionCounters, entries: usize, depth: usize) {
    let state = counters.0.lock().unwrap();
    assert_eq!(state.snapshot.current_depth, depth);
    assert_eq!(state.queued_by_source.len(), entries);
    state.assert_source_depth();
}

#[tokio::test(start_paused = true)]
async fn single_source_flood_caps_at_sixteen_and_another_source_progresses() {
    let (mut network, tx) = fixture();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    let (reply_tx, mut accepted_reply) = oneshot::channel();
    let mut first = incoming(&[2], 0);
    first.reply_tx = Some(reply_tx);
    tx.send(first).await.unwrap();
    for id in 1..256 {
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut arrival = incoming(&[2], id);
        arrival.reply_tx = Some(reply_tx);
        tx.send(arrival).await.unwrap();
        if id >= 16 {
            assert!(timeout(Duration::from_secs(1), reply_rx)
                .await
                .expect("fairness drop retained reply sender")
                .is_err());
        }
    }
    for id in 0..16 {
        tx.send(incoming(&[3], id)).await.unwrap();
    }
    barrier(&tx).await;
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 32,
            high_water: 32,
            fairness_drops: 240,
            ..Default::default()
        }
    );
    assert_sources(&counters, 2, 32);
    assert_eq!(
        accepted_reply.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    );
    for source in [2, 3] {
        for id in 0..16 {
            let apdu = apdus.try_recv().unwrap();
            assert_apdu(&apdu, &[source], id);
            if source == 2 && id == 0 {
                apdu.reply_tx
                    .unwrap()
                    .send(Bytes::from_static(b"reply"))
                    .unwrap();
            }
        }
    }
    assert_eq!(accepted_reply.await.unwrap(), Bytes::from_static(b"reply"));
    assert_eq!(
        apdus.try_recv().unwrap_err(),
        mpsc::error::TryRecvError::Empty
    );
    assert_sources(&counters, 0, 0);
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn every_recv_and_try_recv_releases_one_slot_even_if_consumer_retains_items() {
    let (mut network, tx) = fixture();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    for id in 0..16 {
        tx.send(incoming(&[2], id)).await.unwrap();
    }
    barrier(&tx).await;
    let retained = [apdus.recv().await.unwrap(), apdus.try_recv().unwrap()];
    assert_sources(&counters, 1, 14);
    for id in 16..19 {
        tx.send(incoming(&[2], id)).await.unwrap();
    }
    barrier(&tx).await;
    assert_eq!(counters.snapshot().fairness_drops, 1);
    assert_sources(&counters, 1, 16);
    for id in 2..18 {
        let apdu = if id % 2 == 0 {
            apdus.recv().await.unwrap()
        } else {
            apdus.try_recv().unwrap()
        };
        assert_apdu(&apdu, &[2], id);
        let remaining = usize::from(17 - id);
        assert_sources(&counters, usize::from(remaining != 0), remaining);
    }
    // Empty and cancelled receives neither release a second slot nor leak keys.
    assert_eq!(
        apdus.try_recv().unwrap_err(),
        mpsc::error::TryRecvError::Empty
    );
    assert!(timeout(Duration::from_secs(1), apdus.recv()).await.is_err());
    for id in 19..35 {
        tx.send(incoming(&[2], id)).await.unwrap();
    }
    barrier(&tx).await;
    assert_sources(&counters, 1, 16);
    assert_eq!(counters.snapshot().fairness_drops, 1);
    assert_eq!(counters.snapshot().high_water, 16);
    assert_apdu(&retained[0], &[2], 0);
    assert_apdu(&retained[1], &[2], 1);
    network.stop().await.unwrap();
    for id in 19..35 {
        assert_apdu(&apdus.recv().await.unwrap(), &[2], id);
    }
    assert!(apdus.recv().await.is_none());
    assert_sources(&counters, 0, 0);
}

#[tokio::test(start_paused = true)]
async fn sixteen_sources_compose_to_capacity_and_fairness_precedes_full() {
    let (mut network, tx) = fixture();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    for source in 0..16 {
        for id in 0..16 {
            tx.send(incoming(&[source], id)).await.unwrap();
        }
    }
    tx.send(incoming(&[0], 16)).await.unwrap(); // Over quota AND globally full.
    tx.send(incoming(&[16], 0)).await.unwrap(); // Below quota, globally full.
    barrier(&tx).await;
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            fairness_drops: 1,
            full_drops: 1,
            closed_drops: 0,
        }
    );
    assert_sources(&counters, 16, 256);
    for source in 0..16 {
        for id in 0..16 {
            assert_apdu(&apdus.try_recv().unwrap(), &[source], id);
        }
    }
    assert_sources(&counters, 0, 0);
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn unique_source_churn_cannot_grow_map_beyond_queued_items() {
    let (mut network, tx) = fixture();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    for round in 0..3u16 {
        for id in 0..512u16 {
            tx.send(incoming(&(round * 512 + id).to_be_bytes(), id))
                .await
                .unwrap();
        }
        barrier(&tx).await;
        assert_sources(&counters, 256, 256);
        assert_eq!(counters.snapshot().full_drops, u64::from(round + 1) * 256);
        assert_eq!(counters.snapshot().fairness_drops, 0);
        for id in 0..256u16 {
            assert_apdu(
                &apdus.try_recv().unwrap(),
                &(round * 512 + id).to_be_bytes(),
                id,
            );
            assert_sources(&counters, usize::from(255 - id), usize::from(255 - id));
        }
        assert_sources(&counters, 0, 0);
    }
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn close_and_drop_preserve_closed_precedence_and_release_all_source_entries() {
    for close_then_drain in [true, false] {
        let (mut network, tx) = fixture();
        let mut apdus = network.start_with_admission().await.unwrap();
        let counters = apdus.counters();
        let (reply_tx, mut queued_reply) = oneshot::channel();
        let mut first = incoming(&[2], 0);
        first.reply_tx = Some(reply_tx);
        tx.send(first).await.unwrap();
        for id in 1..16 {
            tx.send(incoming(&[2], id)).await.unwrap();
        }
        barrier(&tx).await;
        let mut apdus = if close_then_drain {
            apdus.close();
            assert_sources(&counters, 1, 16);
            assert_eq!(
                queued_reply.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            );
            Some(apdus)
        } else {
            drop(apdus);
            assert_sources(&counters, 0, 0);
            None
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        let mut arrival = incoming(&[2], 16);
        arrival.reply_tx = Some(reply_tx);
        tx.send(arrival).await.unwrap();
        assert!(timeout(Duration::from_secs(1), reply_rx)
            .await
            .unwrap()
            .is_err());
        timeout(Duration::from_secs(1), tx.closed()).await.unwrap();
        assert_eq!(counters.snapshot().closed_drops, 1);
        assert_eq!(counters.snapshot().fairness_drops, 0);
        assert_eq!(counters.snapshot().full_drops, 0);
        if let Some(apdus) = apdus.as_mut() {
            for id in 0..16 {
                assert_apdu(&apdus.recv().await.unwrap(), &[2], id);
            }
            assert!(apdus.recv().await.is_none());
        }
        assert!(queued_reply.await.is_err());
        assert_sources(&counters, 0, 0);
        network.stop().await.unwrap();
    }
}

#[tokio::test(start_paused = true)]
async fn legacy_apdus_and_tracked_controls_have_no_per_source_cap() {
    let (mut network, tx) = fixture();
    let mut controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    let mut apdus = network.start().await.unwrap();
    for id in 0..257 {
        tx.send(incoming(&[2], id)).await.unwrap();
        tx.send(control()).await.unwrap();
    }
    barrier(&tx).await;
    assert_eq!(apdus.len(), 256);
    assert_eq!(
        controls.counters().snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 256,
            high_water: 256,
            full_drops: 2, // Includes the final control barrier.
            ..Default::default()
        }
    );
    assert!(controls
        .counters
        .0
        .lock()
        .unwrap()
        .queued_by_source
        .is_empty());
    for id in 0..256 {
        assert_apdu(&apdus.try_recv().unwrap(), &[2], id);
        let control = controls.try_recv().unwrap();
        assert_eq!(control.source_mac.as_slice(), &[2]);
        assert_eq!(control.ingress_sequence, u64::from(id) + 1);
    }
    assert_eq!(
        apdus.try_recv().unwrap_err(),
        mpsc::error::TryRecvError::Empty
    );
    network.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn mac_keys_use_complete_byte_value_not_allocation_or_prefix() {
    fn requires_hash_eq<T: std::hash::Hash + Eq>() {}
    requires_hash_eq::<MacAddr>();
    let (mut network, tx) = fixture();
    let apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    for id in 0..16 {
        tx.send(incoming(&[2], id)).await.unwrap();
    }
    let mut spilled = MacAddr::with_capacity(32);
    spilled.push(2);
    assert!(spilled.spilled());
    let mut arrival = incoming(&[2], 16);
    arrival.source_mac = spilled;
    tx.send(arrival).await.unwrap();
    for source in [&[2, 0][..], &[2; 18][..]] {
        for id in 0..17 {
            tx.send(incoming(source, id)).await.unwrap();
        }
    }
    barrier(&tx).await;
    assert_sources(&counters, 3, 48);
    assert_eq!(counters.snapshot().fairness_drops, 3);
    assert_eq!(counters.snapshot().full_drops, 0);
    network.stop().await.unwrap();
    assert_sources(&counters, 3, 48); // Stop retains queued items and keys.
    drop(apdus);
    assert_sources(&counters, 0, 0);
}

#[tokio::test(start_paused = true)]
async fn fairness_drop_counter_saturates_without_altering_depth_or_other_drops() {
    let (mut network, tx) = fixture();
    let apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    for id in 0..16 {
        tx.send(incoming(&[2], id)).await.unwrap();
    }
    barrier(&tx).await;
    counters.0.lock().unwrap().snapshot.fairness_drops = u64::MAX - 1;
    for id in 16..19 {
        tx.send(incoming(&[2], id)).await.unwrap();
    }
    barrier(&tx).await;
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 16,
            high_water: 16,
            fairness_drops: u64::MAX,
            ..Default::default()
        }
    );
    assert_sources(&counters, 1, 16);
    network.stop().await.unwrap();
}

#[tokio::test]
async fn concurrent_dispatch_and_dequeues_keep_source_and_depth_accounting_in_step() {
    let (mut network, tx) = fixture();
    let mut apdus = network.start_with_admission().await.unwrap();
    let counters = apdus.counters();
    let receiver_counters = counters.clone();
    // The crate enables Tokio's current-thread runtime, not rt-multi-thread.
    // Moving the receiver to a second OS thread still races its accounting with
    // real NetworkLayer dispatch, without broadening the dependency features.
    let consumer = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let mut seen = std::collections::HashSet::new();
                for _ in 0..256 {
                    let apdu = timeout(Duration::from_secs(5), apdus.recv())
                        .await
                        .unwrap()
                        .unwrap();
                    let id = u16::from_be_bytes(apdu.apdu.as_ref().try_into().unwrap());
                    let source = apdu.source_mac[0];
                    assert!(source < 16 && id < 16);
                    assert!(seen.insert((source, id)));
                    assert_apdu(&apdu, &[source], id);
                    receiver_counters.0.lock().unwrap().assert_source_depth();
                }
                assert_sources(&receiver_counters, 0, 0);
            });
    });
    // Sixteen items per source fit even if dispatch outruns the consumer;
    // if they race, source-key insertions and removals must stay in step.
    for id in 0..16 {
        for source in 0..16 {
            tx.send(incoming(&[source], id)).await.unwrap();
        }
    }
    barrier(&tx).await;
    consumer.join().unwrap();
    assert_sources(&counters, 0, 0);
    assert_eq!(counters.snapshot().fairness_drops, 0);
    assert_eq!(counters.snapshot().full_drops, 0);
    assert_eq!(counters.snapshot().closed_drops, 0);
    network.stop().await.unwrap();
}
