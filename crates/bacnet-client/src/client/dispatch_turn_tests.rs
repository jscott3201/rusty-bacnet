//! The dispatch loop's turn order (#1978 item 8): what each turn polls, and
//! that neither a cleanup nor the device purge waits out a flood of APDUs.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::task::{Context, Poll, Waker};

use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::{ObjectType, Segmentation};
use bacnet_types::primitives::ObjectIdentifier;

use super::dispatch_turn::{APDUS_BEFORE_CLEANUP, Turn, TurnOrder};
use super::lifecycle::{Controls, DEVICE_PURGE_INTERVAL};
use super::*;

/// Queues standing in for the loop's sources, counting each receiver's polls.
#[derive(Default)]
struct Sources {
    controls: RefCell<VecDeque<u32>>,
    apdus: RefCell<VecDeque<u32>>,
    cleanups: RefCell<VecDeque<u32>>,
    /// Controls the next APDU poll queues, as a receive that meets network
    /// messages on its way to an APDU does.
    queued_by_apdu_poll: RefCell<Vec<u32>>,
    apdu_polls: Cell<usize>,
    cleanup_polls: Cell<usize>,
}

impl Sources {
    fn with_apdus(apdus: impl IntoIterator<Item = u32>) -> Self {
        let sources = Self::default();
        sources.apdus.borrow_mut().extend(apdus);
        sources
    }

    fn turn(&self, order: &mut TurnOrder) -> Poll<Turn<u32, u32, u32>> {
        let mut cx = Context::from_waker(Waker::noop());
        order.poll_turn(
            &mut cx,
            || self.controls.borrow_mut().pop_front(),
            |_| {
                self.apdu_polls.set(self.apdu_polls.get() + 1);
                let queued = std::mem::take(&mut *self.queued_by_apdu_poll.borrow_mut());
                self.controls.borrow_mut().extend(queued);
                ready_or_pending(self.apdus.borrow_mut().pop_front())
            },
            |_| {
                self.cleanup_polls.set(self.cleanup_polls.get() + 1);
                ready_or_pending(self.cleanups.borrow_mut().pop_front())
            },
        )
    }
}

fn ready_or_pending(item: Option<u32>) -> Poll<Option<u32>> {
    item.map_or(Poll::Pending, |item| Poll::Ready(Some(item)))
}

#[test]
fn a_turn_that_finds_an_apdu_does_not_poll_the_cleanup_queue() {
    let sources = Sources::with_apdus([1]);
    let mut order = TurnOrder::default();
    assert_eq!(sources.turn(&mut order), Poll::Ready(Turn::Apdu(Some(1))));
    assert_eq!(sources.apdu_polls.get(), 1);
    assert_eq!(sources.cleanup_polls.get(), 0);
}

#[test]
fn a_turn_that_finds_nothing_polls_both_receivers_once() {
    let sources = Sources::default();
    let mut order = TurnOrder::default();
    assert_eq!(sources.turn(&mut order), Poll::Pending);
    assert_eq!(sources.apdu_polls.get(), 1);
    assert_eq!(sources.cleanup_polls.get(), 1);
}

#[test]
fn a_steady_stream_of_apdus_lets_a_cleanup_through_after_the_bound() {
    let sources = Sources::with_apdus(0..1_000);
    let mut order = TurnOrder::default();
    let mut turns = Vec::new();
    for round in 0..3 {
        sources.cleanups.borrow_mut().push_back(100 + round);
        loop {
            let Poll::Ready(turn) = sources.turn(&mut order) else {
                panic!("an APDU was always queued");
            };
            let done = matches!(turn, Turn::Cleanup(_));
            turns.push(turn);
            if done {
                break;
            }
            assert!(turns.len() < 1_000, "the cleanup waited out the APDUs");
        }
    }
    let run = APDUS_BEFORE_CLEANUP as usize;
    // Each cleanup goes after exactly `APDUS_BEFORE_CLEANUP` APDUs, and the
    // APDUs keep their order around it.
    let mut expected = Vec::new();
    for round in 0..3u32 {
        let first = round * APDUS_BEFORE_CLEANUP;
        expected.extend((first..first + APDUS_BEFORE_CLEANUP).map(|apdu| Turn::Apdu(Some(apdu))));
        expected.push(Turn::Cleanup(Some(100 + round)));
    }
    assert_eq!(turns, expected);
    // Each cleanup turn polled the cleanup queue first and the APDUs not at
    // all; the APDU turns between didn't poll it.
    assert_eq!(sources.cleanup_polls.get(), 3);
    assert_eq!(sources.apdu_polls.get(), 3 * run);
}

#[test]
fn a_cleanup_first_turn_with_no_cleanup_takes_the_apdu_and_polls_the_queue_once() {
    let sources = Sources::with_apdus(0..APDUS_BEFORE_CLEANUP + 2);
    let mut order = TurnOrder::default();
    for apdu in 0..APDUS_BEFORE_CLEANUP {
        assert_eq!(
            sources.turn(&mut order),
            Poll::Ready(Turn::Apdu(Some(apdu)))
        );
    }
    assert_eq!(sources.cleanup_polls.get(), 0);
    assert_eq!(
        sources.turn(&mut order),
        Poll::Ready(Turn::Apdu(Some(APDUS_BEFORE_CLEANUP)))
    );
    assert_eq!(sources.cleanup_polls.get(), 1);
    // The empty poll ended the run, so the next APDU turn doesn't poll the
    // queue first again.
    assert_eq!(
        sources.turn(&mut order),
        Poll::Ready(Turn::Apdu(Some(APDUS_BEFORE_CLEANUP + 1)))
    );
    assert_eq!(sources.cleanup_polls.get(), 1);
    // A turn with nothing polls the queue once more.
    assert_eq!(sources.turn(&mut order), Poll::Pending);
    assert_eq!(sources.cleanup_polls.get(), 2);
}

#[test]
fn a_turn_without_an_apdu_ends_the_run() {
    let sources = Sources::with_apdus(0..APDUS_BEFORE_CLEANUP - 1);
    let mut order = TurnOrder::default();
    while let Poll::Ready(Turn::Apdu(_)) = sources.turn(&mut order) {}
    assert_eq!(sources.cleanup_polls.get(), 1);
    // A new run of APDUs starts counting from none.
    sources.apdus.borrow_mut().extend(0..APDUS_BEFORE_CLEANUP);
    for _ in 0..APDUS_BEFORE_CLEANUP {
        assert!(matches!(
            sources.turn(&mut order),
            Poll::Ready(Turn::Apdu(_))
        ));
    }
    assert_eq!(sources.cleanup_polls.get(), 1);
}

#[test]
fn controls_come_first_and_those_the_apdu_poll_queues_go_in_its_turn() {
    let sources = Sources::with_apdus([10]);
    sources.controls.borrow_mut().push_back(1);
    let mut order = TurnOrder::default();
    assert_eq!(sources.turn(&mut order), Poll::Ready(Turn::Control(1)));
    assert_eq!(sources.apdu_polls.get(), 0);
    assert_eq!(sources.turn(&mut order), Poll::Ready(Turn::Apdu(Some(10))));

    // A poll that meets only network messages queues them and finds no
    // APDU; the same turn takes the first, after polling the cleanup queue.
    sources.queued_by_apdu_poll.borrow_mut().extend([2, 3]);
    assert_eq!(sources.turn(&mut order), Poll::Ready(Turn::Control(2)));
    assert_eq!(sources.cleanup_polls.get(), 1);
    // The next turn takes the other before it polls anything.
    assert_eq!(sources.turn(&mut order), Poll::Ready(Turn::Control(3)));
    assert_eq!(sources.apdu_polls.get(), 2);
    assert_eq!(sources.turn(&mut order), Poll::Pending);
}

#[test]
fn a_closed_receiver_ends_the_turn_with_none() {
    let mut order = TurnOrder::default();
    let mut cx = Context::from_waker(Waker::noop());
    let turn: Poll<Turn<u32, u32, u32>> = order.poll_turn(
        &mut cx,
        || None,
        |_| Poll::Ready(None),
        |_| panic!("an APDU turn doesn't poll cleanups"),
    );
    assert_eq!(turn, Poll::Ready(Turn::Apdu(None)));
    let turn: Poll<Turn<u32, u32, u32>> =
        order.poll_turn(&mut cx, || None, |_| Poll::Pending, |_| Poll::Ready(None));
    assert_eq!(turn, Poll::Ready(Turn::Cleanup(None)));
}

const CLIENT_MAC: [u8; 1] = [0x01];
const PEER_MAC: [u8; 1] = [0x02];

/// A SimpleACK for an invoke ID the client has no transaction for: the
/// dispatch loop decodes it, finds nothing to complete and drops it, and it
/// announces nothing.
fn stray_simple_ack(invoke_id: u8) -> Vec<u8> {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu.to_vec()
}

/// Keep the client's receive queue full: each round sends more frames than
/// the queue holds (the loopback drops the rest), then yields. The dispatch
/// task takes fewer than that before Tokio's budget makes it yield, so it
/// always finds another APDU waiting.
fn flood(peer: LoopbackTransport, invoke_id: u8) -> tokio::task::JoinHandle<()> {
    let frame = stray_simple_ack(invoke_id);
    tokio::spawn(async move {
        loop {
            for _ in 0..300 {
                peer.send_unicast(&frame, &CLIENT_MAC).await.unwrap();
            }
            tokio::task::yield_now().await;
        }
    })
}

#[tokio::test]
async fn a_cleanup_is_processed_while_apdus_keep_arriving() {
    let (client_transport, peer) = LoopbackTransport::pair(&CLIENT_MAC[..], &PEER_MAC[..]);
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mac = MacAddr::from_slice(&PEER_MAC);
    let (invoke_id, registration) = client
        .tsm
        .lock()
        .register_coordinated_transaction_with_progress(
            mac.clone(),
            CanonicalPeer::direct(&PEER_MAC),
            ConfirmedServiceChoice::WRITE_PROPERTY,
            false,
        )
        .unwrap();
    // Stray acknowledgments: none of them completes this transaction.
    let flood = flood(peer, invoke_id.wrapping_add(128));
    tokio::task::yield_now().await;

    client.enqueue_transaction_cleanup(&mac, invoke_id, &registration.owner, true, None);
    timeout(
        Duration::from_secs(5),
        client.segmented_cleanup.wait_processed(),
    )
    .await
    .expect("the cleanup waited out the flood of APDUs");
    assert_eq!(client.tsm.lock().pending_count(), 0);

    flood.abort();
    client.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn the_device_purge_runs_while_apdus_keep_arriving() {
    let (client_transport, peer) = LoopbackTransport::pair(&CLIENT_MAC[..], &PEER_MAC[..]);
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let mut events = client.device_events();
    // Let the purge task consume its interval's immediate tick.
    tokio::task::yield_now().await;
    client.device_table.lock().upsert(DiscoveredDevice {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2001).unwrap(),
        mac_address: MacAddr::from_slice(&[192, 168, 1, 42, 0xBA, 0xC0]),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        max_segments_accepted: None,
        vendor_id: 42,
        last_seen: crate::discovery::now(),
        source_network: None,
        source_address: None,
    });
    let flood = flood(peer, 0);
    tokio::task::yield_now().await;

    // The runtime is never idle now, so paused time moves only by hand. The
    // flood announces no device events, so the Lost one is the first.
    // Past the row's age limit, to the purge tick after it.
    tokio::time::advance(DEVICE_PURGE_INTERVAL * 3).await;
    let mut turns = 0;
    let event = loop {
        match events.try_recv() {
            Ok(event) => break event,
            Err(broadcast::error::TryRecvError::Empty) if turns < 10_000 => {
                turns += 1;
                tokio::task::yield_now().await;
            }
            other => panic!("no Lost event while the APDUs kept arriving: {other:?}"),
        }
    };
    assert_eq!(event.kind, DeviceEventKind::Lost);
    assert_eq!(event.device.object_identifier.instance_number(), 2001);

    flood.abort();
    client.stop().await.unwrap();
}

#[tokio::test]
async fn stopping_or_dropping_the_client_ends_the_device_purge() {
    let (client_transport, _peer) = LoopbackTransport::pair(&CLIENT_MAC[..], &PEER_MAC[..]);
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let purge = client
        .device_purge_task
        .as_ref()
        .expect("the client starts a purge task")
        .abort_handle();
    client.stop().await.unwrap();
    assert!(purge.is_finished(), "stop() joins the purge task");
    assert!(client.device_purge_task.is_none());

    let (client_transport, _peer) = LoopbackTransport::pair(&CLIENT_MAC[..], &PEER_MAC[..]);
    let client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let purge = client
        .device_purge_task
        .as_ref()
        .expect("the client starts a purge task")
        .abort_handle();
    drop(client);
    timeout(Duration::from_secs(1), async {
        while !purge.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the client aborts the purge task");
}

fn control(ingress_sequence: u64) -> ReceivedNetworkControl {
    ReceivedNetworkControl {
        npdu: Npdu::default(),
        source_mac: MacAddr::from_slice(&PEER_MAC),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        ingress_sequence,
    }
}

#[test]
fn controls_are_read_only_once_the_ingress_sequence_moves_or_at_its_maximum() {
    let (tx, rx) = mpsc::channel(4);
    let mut controls = Controls::new(rx, 5);
    // Queued without the sequence moving, as no real layer does: not read.
    tx.try_send(control(5)).unwrap();
    assert!(controls.take(5).is_none());
    assert_eq!(controls.take(6).map(|c| c.ingress_sequence), Some(5));
    assert!(controls.take(6).is_none(), "the queue is empty at 6 now");
    tx.try_send(control(6)).unwrap();
    assert!(controls.take(6).is_none());

    // At its maximum the sequence stops moving, so the queue is read every time.
    let (tx, rx) = mpsc::channel(4);
    let mut controls = Controls::new(rx, u64::MAX);
    tx.try_send(control(u64::MAX)).unwrap();
    tx.try_send(control(u64::MAX)).unwrap();
    assert!(controls.take(u64::MAX).is_some());
    assert!(controls.take(u64::MAX).is_some());
    assert!(controls.take(u64::MAX).is_none());
    tx.try_send(control(u64::MAX)).unwrap();
    assert!(controls.take(u64::MAX).is_some());

    // A closed queue is read no more.
    drop(tx);
    assert!(controls.take(u64::MAX).is_none());
    assert!(!controls.open);
}

#[tokio::test]
async fn the_device_purge_ends_with_the_dispatch_loop() {
    let (client_transport, peer) = LoopbackTransport::pair(&CLIENT_MAC[..], &PEER_MAC[..]);
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    let dispatch = client.dispatch_task.as_ref().unwrap().abort_handle();
    let purge = client.device_purge_task.as_ref().unwrap().abort_handle();
    // With the peer gone the receive queue closes, and the loop ends.
    drop(peer);
    timeout(Duration::from_secs(1), async {
        while !(dispatch.is_finished() && purge.is_finished()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the purge task outlived the dispatch loop");
    client.stop().await.unwrap();
}
