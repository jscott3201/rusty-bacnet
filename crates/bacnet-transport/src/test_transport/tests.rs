//! Contract of the shared test transport itself.
use super::*;
use crate::port::DataAttribute;
use bacnet_encoding::apdu::{Apdu, SimpleAck, encode_apdu};
use bacnet_encoding::npdu::Npdu;
use bacnet_encoding::npdu::encode_npdu;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::BytesMut;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicUsize, Ordering};
use std::time::Duration;

fn simple_ack_npdu(invoke_id: u8) -> Bytes {
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
    npdu.freeze()
}

fn attribute() -> DataAttribute {
    DataAttribute {
        option_type: 1,
        must_understand: false,
        data: vec![7],
    }
}

async fn panics(future: impl Future<Output = ()> + Send + 'static) -> bool {
    tokio::spawn(future).await.unwrap_err().is_panic()
}

#[tokio::test]
async fn identity_defaults_and_hooks() {
    let transport = TestTransport::new();
    assert_eq!(transport.local_mac(), &[1]);
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
    assert!(!transport.is_broadcast_mac(&[0xFF]));
    assert_eq!(transport.bip_broadcast_endpoint(), None);

    let endpoint = SocketAddrV4::new([127, 0, 0, 255].into(), 47808);
    let transport = TestTransport::builder()
        .local_mac(&BIP_LOCAL_MAC)
        .broadcast_mac(&[0xFF])
        .build();
    assert_eq!(transport.local_mac(), &BIP_LOCAL_MAC);
    assert!(transport.is_broadcast_mac(&[0xFF]));
    assert!(!transport.is_broadcast_mac(&[1]));

    let queries = Arc::new(AtomicUsize::new(0));
    let (macs, endpoints) = (Arc::clone(&queries), Arc::clone(&queries));
    let transport = TestTransport::builder()
        .broadcast_mac(&[0xFF])
        .on_is_broadcast_mac(move |mac| {
            macs.fetch_add(1, Ordering::SeqCst);
            mac == [0x42]
        })
        .on_bip_broadcast_endpoint(move || {
            endpoints.fetch_add(1, Ordering::SeqCst);
            Some(endpoint)
        })
        .build();
    assert!(transport.is_broadcast_mac(&[0x42]));
    assert!(
        !transport.is_broadcast_mac(&[0xFF]),
        "the hook replaces the list"
    );
    assert_eq!(transport.bip_broadcast_endpoint(), Some(endpoint));
    assert_eq!(queries.load(Ordering::SeqCst), 3);
}

/// Group MACs widen `is_group_destination` and the owned rule, not
/// `is_broadcast_mac`; the owned rule never calls the broadcast hook.
#[test]
fn group_macs_are_group_destinations_only() {
    let transport = TestTransport::builder()
        .broadcast_mac(&[0xFF])
        .group_mac(&[0xE0])
        .build();
    let owned = transport.group_destinations();
    for (mac, broadcast, group) in [
        ([0xFF], true, true),
        ([0xE0], false, true),
        ([1], false, false),
    ] {
        assert_eq!(transport.is_broadcast_mac(&mac), broadcast, "{mac:?}");
        assert_eq!(transport.is_group_destination(&mac), group, "{mac:?}");
        assert_eq!(owned.contains(&mac), group, "{mac:?}");
    }

    let queries = Arc::new(AtomicUsize::new(0));
    let hook = Arc::clone(&queries);
    let transport = TestTransport::builder()
        .on_is_broadcast_mac(move |_| {
            hook.fetch_add(1, Ordering::SeqCst);
            false
        })
        .group_mac(&[0xE0])
        .build();
    assert!(transport.group_destinations().contains(&[0xE0]));
    assert_eq!(queries.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn start_modes() {
    let started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&started);
    let mut transport = TestTransport::builder()
        .on_start(move || flag.store(true, Ordering::SeqCst))
        .build();
    assert!(transport.start().await.unwrap().recv().await.is_none());
    assert!(started.load(Ordering::SeqCst));
    assert_eq!(transport.handle().starts(), 1);

    let (mut transport, tx) = TestTransport::inbound(1);
    let mut rx = transport.start().await.unwrap();
    tx.send(ReceivedNpdu::unverified(
        simple_ack_npdu(1),
        MacAddr::from_slice(&[9]),
        false,
        Vec::new(),
        None,
    ))
    .await
    .unwrap();
    assert_eq!(rx.recv().await.unwrap().source_mac.as_slice(), &[9]);
    assert!(
        transport.start().await.is_err(),
        "the receiver is taken once"
    );

    let mut transport = TestTransport::never_start();
    let handle = transport.handle();
    assert!(
        panics(async move {
            let _ = transport.start().await;
        })
        .await
    );
    assert_eq!(
        handle.starts(),
        1,
        "a start is counted before its mode runs"
    );
}

#[tokio::test]
async fn records_both_kinds_and_decodes() {
    let transport = TestTransport::new();
    let sent = transport.sent();
    let npdu = simple_ack_npdu(3);
    transport.send_unicast(&npdu, &[5]).await.unwrap();
    transport
        .send_unicast_with_data_attributes(&npdu, &[6], &[attribute()])
        .await
        .unwrap();
    transport.send_broadcast(&npdu).await.unwrap();
    transport
        .send_broadcast_with_data_attributes(&npdu, &[attribute()])
        .await
        .unwrap();
    assert_eq!(sent.len(), 4);
    assert_eq!(sent.unicasts().len(), 2);
    assert_eq!(sent.broadcasts().len(), 2);
    let frame = sent.frame(1);
    assert_eq!((frame.mac.as_slice(), frame.broadcast), (&[6][..], false));
    assert!(sent.frame(2).broadcast && sent.frame(2).mac.is_empty());
    assert_eq!(sent.npdus(), vec![npdu.clone(); 4]);
    assert!(matches!(sent.frame(0).apdu(), Apdu::SimpleAck(ref ack) if ack.invoke_id == 3));
    assert!(sent.frames()[3].decode_npdu().destination.is_none());
    assert_eq!(sent.take().len(), 4);
    assert!(sent.is_empty());
    sent.push(frame);
    sent.clear();
    assert!(sent.lock().is_empty());
}

#[tokio::test]
async fn send_modes() {
    let transport = TestTransport::builder()
        .unicast(SendMode::Ignore)
        .broadcast(SendMode::Record)
        .build();
    transport.send_unicast(&[1], &[2]).await.unwrap();
    transport.send_broadcast(&[1]).await.unwrap();
    assert_eq!(transport.sent().broadcasts().len(), 1);
    assert_eq!(transport.sent().len(), 1);
    let transport = Arc::new(
        TestTransport::builder()
            .broadcast(SendMode::Panic("must stay silent"))
            .build(),
    );
    assert!(panics(async move { transport.send_broadcast(&[1]).await.unwrap() }).await);
}

#[tokio::test]
async fn built_in_controls_fail_and_hold() {
    let transport = Arc::new(TestTransport::new());
    let handle = transport.handle();
    handle.fail_next_send();
    assert!(transport.send_unicast(&[1], &[2]).await.is_err());
    assert!(transport.send_unicast(&[1], &[2]).await.is_ok());
    assert_eq!(handle.sent().len(), 2, "failed sends are still recorded");

    handle.block_next_send();
    let held = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.send_unicast(&[1], &[2]).await }
    });
    // Let the spawned send reach its hold before waiting, so the wait must be
    // satisfied by a send that was already held.
    tokio::task::yield_now().await;
    assert_eq!(handle.sent().len(), 3, "the spawned send ran and is held");
    assert!(!held.is_finished());
    handle.wait_blocked().await;
    assert!(!held.is_finished());
    transport.send_unicast(&[1], &[2]).await.unwrap();
    handle.release_sends(1);
    held.await.unwrap().unwrap();
}

#[tokio::test]
async fn held_send_fails_after_release_and_is_recorded() {
    let transport = Arc::new(TestTransport::new());
    let handle = transport.handle();
    handle.block_next_send();
    handle.fail_next_send();
    let held = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.send_unicast(&[1], &[2]).await }
    });
    handle.wait_blocked().await;
    assert_eq!(handle.sent().len(), 1, "a held send is already recorded");
    assert!(!held.is_finished(), "the send is held, not failed early");
    handle.release_sends(1);
    assert!(
        held.await.unwrap().is_err(),
        "the release is followed by the failure"
    );
    assert_eq!(handle.sent().len(), 1);
    transport
        .send_unicast(&[1], &[2])
        .await
        .expect("the failure and the hold were one-shot");
}

/// Arm a hold and a failure for the next recorded send.
fn arm_controls(handle: &TestTransportHandle) {
    handle.block_next_send();
    handle.fail_next_send();
}

#[tokio::test]
async fn ignore_and_hook_errors_skip_built_in_controls() {
    // An ignored unicast leaves the armed controls for the next recorded send.
    let transport = Arc::new(TestTransport::builder().unicast(SendMode::Ignore).build());
    let handle = transport.handle();
    arm_controls(&handle);
    transport
        .send_unicast(&[1], &[2])
        .await
        .expect("an ignored send neither holds nor fails");
    assert!(handle.sent().is_empty());
    let held = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.send_broadcast(&[1]).await }
    });
    handle.wait_blocked().await;
    handle.release_sends(1);
    assert!(
        held.await.unwrap().is_err(),
        "the controls were still armed"
    );

    // A hook error ends the send before the controls, which stay armed.
    let refuse = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&refuse);
    let transport = Arc::new(
        TestTransport::builder()
            .on_send(move |_| {
                let refuse = flag.load(Ordering::SeqCst);
                async move {
                    if refuse {
                        Err(Error::Transport(std::io::Error::other("hook refused")))
                    } else {
                        Ok(())
                    }
                }
            })
            .build(),
    );
    let handle = transport.handle();
    arm_controls(&handle);
    assert!(matches!(
        transport.send_unicast(&[1], &[2]).await,
        Err(Error::Transport(m)) if m.to_string() == "hook refused"
    ));
    assert_eq!(handle.sent().len(), 1, "the refused send is recorded");
    refuse.store(false, Ordering::SeqCst);
    let held = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.send_unicast(&[1], &[2]).await }
    });
    handle.wait_blocked().await;
    handle.release_sends(1);
    assert!(
        held.await.unwrap().is_err(),
        "the controls were still armed"
    );
}

#[tokio::test]
async fn hooks_state_and_lifecycle() {
    let stops = Arc::new(AtomicUsize::new(0));
    let dropped = Arc::new(AtomicBool::new(false));
    let (stop_count, drop_flag) = (Arc::clone(&stops), Arc::clone(&dropped));
    let builder = TestTransport::builder();
    let log = builder.sent();
    let transport = builder
        .on_send(move |frame| {
            let recorded = log.len();
            async move {
                assert_eq!(recorded, 1, "the frame is logged before the hook");
                if frame.mac.as_slice() == [9] {
                    return Err(Error::Transport(std::io::Error::other("hook refused")));
                }
                Ok(())
            }
        })
        .on_stop(move || {
            stop_count.fetch_add(1, Ordering::SeqCst);
            async { Err(Error::Transport(std::io::Error::other("cleanup failed"))) }
        })
        .on_drop(move || drop_flag.store(true, Ordering::SeqCst))
        .state(Arc::new(41_u32))
        .build();
    assert!(transport.send_unicast(&[1], &[9]).await.is_err());
    assert_eq!(*transport.state::<u32>(), 41);
    assert!(transport.stop().await.is_err());
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    drop(transport);
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn wait_for_len_wakes_on_push() {
    let transport = TestTransport::new();
    let sent = transport.sent();
    let waiter = tokio::spawn(async move { sent.wait_for_len(2).await });
    transport.send_unicast(&[1], &[2]).await.unwrap();
    tokio::task::yield_now().await;
    assert!(!waiter.is_finished());
    transport.send_broadcast(&[1]).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn data_attributes_are_recorded_with_the_frame() {
    let transport = TestTransport::new();
    let sent = transport.sent();
    transport.send_unicast(&[1], &[2]).await.unwrap();
    transport
        .send_unicast_with_data_attributes(&[1], &[2], &[attribute()])
        .await
        .unwrap();
    transport
        .send_broadcast_with_data_attributes(&[1], &[attribute()])
        .await
        .unwrap();
    assert!(sent.frame(0).attributes.is_empty());
    assert_eq!(sent.frame(1).attributes, vec![attribute()]);
    assert_eq!(sent.frame(2).attributes, vec![attribute()]);
}

#[tokio::test]
async fn limits_follow_settings_hooks_and_handle() {
    let transport = TestTransport::new();
    assert_eq!(transport.local_receive_apdu_capacity(), 1476);
    assert_eq!(transport.egress_apdu_limit(), 1476);
    assert_eq!(transport.max_npdu(), NpduLimits::default());
    assert!(transport.max_npdu_changes().is_none());

    let (publish, changes) = watch::channel(NpduLimits::both(500));
    let transport = TestTransport::builder()
        .receive_capacity(480)
        .egress_limit(400)
        .max_npdu(NpduLimits::both(600))
        .max_npdu_changes(changes)
        .build();
    assert_eq!(transport.local_receive_apdu_capacity(), 480);
    assert_eq!(transport.egress_apdu_limit(), 400);
    assert_eq!(transport.max_npdu(), NpduLimits::both(600));
    let handle = transport.handle();
    handle.set_receive_capacity(100);
    handle.set_egress_limit(90);
    handle.set_max_npdu(NpduLimits::both(80));
    assert_eq!(transport.local_receive_apdu_capacity(), 100);
    assert_eq!(transport.egress_apdu_limit(), 90);
    assert_eq!(transport.max_npdu(), NpduLimits::both(80));
    assert_eq!(
        *transport.max_npdu_changes().unwrap().borrow(),
        NpduLimits::both(500)
    );
    drop(publish);

    let own = Arc::new(AtomicU16::new(70));
    let hooked = Arc::clone(&own);
    let transport = TestTransport::builder()
        .on_max_npdu(move || NpduLimits::both(hooked.load(Ordering::SeqCst)))
        .build();
    assert_eq!(transport.max_npdu(), NpduLimits::both(70));
    own.store(71, Ordering::SeqCst);
    assert_eq!(transport.max_npdu(), NpduLimits::both(71));
}

/// The link's returns are the test's to publish, its own receiver or else
/// the inner transport's; with neither, the transport reports none.
#[tokio::test]
async fn link_returns_are_the_tests_or_the_inner_transports() {
    assert!(TestTransport::new().link_returns().is_none());

    let (back, returns) = watch::channel(0u64);
    let transport = TestTransport::builder().link_returns(returns).build();
    let mut seen = transport.link_returns().unwrap();
    back.send_modify(|count| *count += 1);
    seen.changed().await.unwrap();
    assert_eq!(*seen.borrow(), 1);

    let (_inner_back, inner_returns) = watch::channel(5u64);
    let outer = TestTransport::builder()
        .inner(TestTransport::builder().link_returns(inner_returns).build())
        .build();
    assert_eq!(*outer.link_returns().unwrap().borrow(), 5);
}

#[tokio::test]
async fn lifecycle_counters_and_started_report() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let aborted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&aborted);
    let mut transport = TestTransport::builder()
        .local_mac(&[7])
        .receive_capacity(480)
        .egress_limit(400)
        .on_started(move |started| {
            log.lock().unwrap().push((
                started.local_mac.to_vec(),
                started.receive_capacity,
                started.egress_limit,
            ));
        })
        .on_abort(move || flag.store(true, Ordering::SeqCst))
        .build();
    let handle = transport.handle();
    transport.prepare_for_routing();
    transport.start().await.unwrap();
    transport.stop().await.unwrap();
    transport.abort();
    assert_eq!(
        (
            handle.prepares(),
            handle.starts(),
            handle.stops(),
            handle.aborts()
        ),
        (1, 1, 1, 1)
    );
    assert!(aborted.load(Ordering::SeqCst));
    assert_eq!(*seen.lock().unwrap(), vec![(vec![7], 480, 400)]);

    // A failed start reports nothing.
    let seen = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&seen);
    let mut transport = TestTransport::builder()
        .start_gate(|| async { Err(Error::Transport(std::io::Error::other("no start"))) })
        .on_started(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
        })
        .build();
    assert!(transport.start().await.is_err());
    assert_eq!(seen.load(Ordering::SeqCst), 0);
}

/// From a stop's return to the next start every send fails with the
/// stopped error, in every mode, records nothing and runs no hook, as on a
/// real link (#2617); the stop works through a shared reference.
#[tokio::test]
async fn sends_fail_between_a_stop_and_the_next_start() {
    let hooked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&hooked);
    let mut transport = TestTransport::builder()
        .start(StartMode::Fresh(4))
        .broadcast(SendMode::Ignore)
        .on_send(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .build();
    let handle = transport.handle();
    transport.start().await.unwrap();
    transport.send_unicast(&[1, 0], &[2]).await.unwrap();
    let transport = Arc::new(transport);
    Arc::clone(&transport).stop().await.unwrap();
    let stopped =
        |sent: Result<(), Error>| matches!(sent, Err(Error::State(Lifecycle::NotRunning, _)));
    assert!(stopped(transport.send_unicast(&[1, 0], &[2]).await));
    assert!(
        stopped(transport.send_broadcast(&[1, 0]).await),
        "Ignore too"
    );
    assert!(stopped(
        transport
            .send_unicast_with_data_attributes(&[1, 0], &[2], &[attribute()])
            .await
    ));
    assert_eq!(handle.sent().len(), 1, "nothing recorded after the stop");
    assert_eq!(hooked.load(Ordering::SeqCst), 1, "no hook after the stop");
    let mut transport = Arc::into_inner(transport).unwrap();
    transport.start().await.unwrap();
    transport.send_unicast(&[1, 0], &[2]).await.unwrap();
    assert_eq!(handle.sent().len(), 2, "a start opens the link again");
}

/// An abort through a shared reference fails every later send, as a stop
/// does, and closes the `Fresh` channel; a start then waits for a stop
/// (#2617).
#[tokio::test]
async fn sends_fail_after_an_abort_until_the_next_start() {
    let mut transport = TestTransport::builder().start(StartMode::Fresh(4)).build();
    let handle = transport.handle();
    let mut received = transport.start().await.unwrap();
    let transport = Arc::new(transport);
    transport.abort();
    let sent = transport.send_unicast(&[1, 0], &[2]).await;
    assert!(
        matches!(&sent, Err(Error::State(Lifecycle::NotRunning, _))),
        "{sent:?}"
    );
    assert!(received.recv().await.is_none());
    assert!(handle.sent().is_empty());
    assert_eq!((handle.stops(), handle.aborts()), (0, 1));
    let mut transport = Arc::into_inner(transport).unwrap();
    assert!(
        matches!(
            transport.start().await,
            Err(Error::State(Lifecycle::Stopping, _))
        ),
        "a start waits for a stop"
    );
    transport.stop().await.unwrap();
    transport.start().await.unwrap();
    transport.send_unicast(&[1, 0], &[2]).await.unwrap();
    assert_eq!(handle.sent().len(), 1, "a start opens the link again");
}

/// Two stops at once run the stop hook once, and both return once it has
/// finished; a stop after it runs nothing until a start, and one that
/// failed is run again. Every call is counted (#2617).
#[tokio::test(start_paused = true)]
async fn stops_at_once_run_the_hooks_once() {
    let runs = Arc::new(AtomicUsize::new(0));
    let fail = Arc::new(AtomicBool::new(false));
    let release = Arc::new(tokio::sync::Notify::new());
    let (count, failing, gate) = (Arc::clone(&runs), Arc::clone(&fail), Arc::clone(&release));
    let transport = TestTransport::builder()
        .on_stop(move || {
            count.fetch_add(1, Ordering::SeqCst);
            let (failing, gate) = (Arc::clone(&failing), Arc::clone(&gate));
            async move {
                gate.notified().await;
                match failing.load(Ordering::SeqCst) {
                    true => Err(Error::Transport(std::io::Error::other("stop refused"))),
                    false => Ok(()),
                }
            }
        })
        .build();
    let handle = transport.handle();
    let transport = Arc::new(transport);
    fail.store(true, Ordering::SeqCst);
    let first = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.stop().await }
    });
    tokio::time::sleep(Duration::from_millis(1)).await;
    let second = tokio::spawn({
        let transport = Arc::clone(&transport);
        async move { transport.stop().await }
    });
    tokio::time::sleep(Duration::from_millis(1)).await;
    assert_eq!(runs.load(Ordering::SeqCst), 1, "the second waits");
    // The first fails: the second runs the hook itself, and succeeds.
    release.notify_one();
    assert!(first.await.unwrap().is_err());
    fail.store(false, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(1)).await;
    release.notify_one();
    second.await.unwrap().unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    transport.stop().await.unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 2, "nothing runs after a stop");
    assert_eq!(handle.stops(), 3, "every call counts");
}
