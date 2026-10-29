//! Contract of the shared test transport itself.
use super::*;
use bacnet_encoding::apdu::{SimpleAck, encode_apdu};
use bacnet_encoding::npdu::encode_npdu;
use bacnet_transport::port::DataAttribute;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::BytesMut;
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
                        Err(Error::Encoding("hook refused".into()))
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
        Err(Error::Encoding(m)) if m == "hook refused"
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
    let mut transport = builder
        .on_send(move |frame| {
            let recorded = log.len();
            async move {
                assert_eq!(recorded, 1, "the frame is logged before the hook");
                if frame.mac.as_slice() == [9] {
                    return Err(Error::Encoding("hook refused".into()));
                }
                Ok(())
            }
        })
        .on_stop(move || {
            stop_count.fetch_add(1, Ordering::SeqCst);
            async { Err(Error::Encoding("cleanup failed".into())) }
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
