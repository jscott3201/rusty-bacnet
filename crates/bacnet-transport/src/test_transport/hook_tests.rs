//! The start, stop, lease and port hooks, with and without an inner transport.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[tokio::test]
async fn before_stop_holds_the_stop_and_the_inner_stop_still_runs() {
    let inner = TestTransport::new();
    let seen = inner.handle();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let hold = Arc::clone(&gate);
    let mut outer = TestTransport::builder()
        .inner(inner)
        .before_stop(move || {
            let hold = Arc::clone(&hold);
            async move {
                hold.acquire().await.unwrap().forget();
                Ok(())
            }
        })
        .build();
    {
        let stop = outer.stop();
        tokio::pin!(stop);
        tokio::select! {
            biased;
            _ = &mut stop => panic!("the stop was not held before the inner stop"),
            () = std::future::ready(()) => {}
        }
        assert_eq!(seen.stops(), 0);
        gate.add_permits(1);
        stop.await.unwrap();
    }
    assert_eq!(seen.stops(), 1, "the inner stop ran after the hook");

    // A failing hook ends the stop with the inner transport untouched.
    let inner = TestTransport::new();
    let seen = inner.handle();
    let mut outer = TestTransport::builder()
        .inner(inner)
        .before_stop(|| async { Err(Error::Encoding("refused".into())) })
        .build();
    assert!(outer.stop().await.is_err());
    assert_eq!((outer.handle().stops(), seen.stops()), (1, 0));
}

#[tokio::test]
async fn stop_releases_the_lease_the_transport_holds() {
    let endpoint = SocketAddrV4::new([10, 0, 0, 1].into(), 47808);
    for failing in [false, true] {
        let lease = Arc::new(());
        let mut builder = TestTransport::builder().normal_bip(endpoint);
        if failing {
            builder = builder.on_stop(|| async { Err(Error::Encoding("stop failed".into())) });
        }
        let mut transport = builder.build();
        transport
            .retain_network_port_lease_internal(Arc::clone(&lease))
            .unwrap();
        assert_eq!(Arc::strong_count(&lease), 2);
        assert_eq!(transport.stop().await.is_err(), failing);
        assert_eq!(Arc::strong_count(&lease), 1, "failing: {failing}");
    }
}

#[tokio::test]
async fn replace_starts_the_inner_transport_but_hands_out_the_tests_receiver() {
    let inner = TestTransport::new();
    let seen = inner.handle();
    let (tx, rx) = mpsc::channel(1);
    let mut outer = TestTransport::builder()
        .inner(inner)
        .replace_inbound(rx)
        .build();
    let mut received = outer.start().await.unwrap();
    assert_eq!(seen.starts(), 1, "the inner transport was started");
    tx.send(ReceivedNpdu::unverified(
        Bytes::from_static(&[7]),
        MacAddr::from_slice(&[2]),
        false,
        Vec::new(),
        None,
    ))
    .await
    .unwrap();
    assert_eq!(&received.recv().await.unwrap().npdu[..], &[7]);
    assert!(outer.start().await.is_err(), "the receiver is taken once");
    assert_eq!(seen.starts(), 2);

    // An inner start that fails fails the start; without one every start fails.
    let (_tx, rx) = mpsc::channel(1);
    let mut bare = TestTransport::builder().replace_inbound(rx).build();
    assert!(bare.start().await.is_err());
}

#[tokio::test]
async fn after_stop_runs_after_a_successful_stop_only() {
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (log_inner, log_hook) = (Arc::clone(&order), Arc::clone(&order));
    let inner = TestTransport::builder()
        .on_stop(move || {
            log_inner.lock().unwrap().push("inner");
            async { Ok(()) }
        })
        .build();
    let mut outer = TestTransport::builder()
        .inner(inner)
        .after_stop(move || {
            log_hook.lock().unwrap().push("after");
            async { Ok(()) }
        })
        .build();
    outer.stop().await.unwrap();
    assert_eq!(*order.lock().unwrap(), ["inner", "after"]);

    // A failing stop skips the hook; a failing hook fails the stop.
    let ran = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&ran);
    let mut failing = TestTransport::builder()
        .on_stop(|| async { Err(Error::Encoding("stop failed".into())) })
        .after_stop(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .build();
    assert!(failing.stop().await.is_err());
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    let mut refusing = TestTransport::builder()
        .after_stop(|| async { Err(Error::Encoding("after failed".into())) })
        .build();
    assert!(matches!(refusing.stop().await, Err(Error::Encoding(m)) if m == "after failed"));
}

#[tokio::test]
async fn bip_port_and_lease_hooks_and_a_start_held_after_the_link_is_up() {
    // The port hook filters the inner port by the test's state.
    let present = Arc::new(AtomicBool::new(true));
    let flag = Arc::clone(&present);
    let endpoint = SocketAddrV4::new([10, 0, 0, 1].into(), 47808);
    let witnessed = Arc::new(std::sync::Mutex::new(std::sync::Weak::new()));
    let witness = Arc::clone(&witnessed);
    let mut outer = TestTransport::builder()
        .inner(TestTransport::builder().normal_bip(endpoint).build())
        .on_bip_port(move |port| flag.load(Ordering::SeqCst).then_some(port).flatten())
        .on_retain_lease(move |lease| *witness.lock().unwrap() = Arc::downgrade(lease))
        .build();
    assert!(outer.bip_port().is_some());
    present.store(false, Ordering::SeqCst);
    assert!(
        outer.bip_port().is_none(),
        "the hook dropped the inner port"
    );

    // The lease is witnessed, then held by the inner transport, which keeps it.
    let lease = Arc::new(());
    outer
        .retain_network_port_lease_internal(Arc::clone(&lease))
        .unwrap();
    assert_eq!(witnessed.lock().unwrap().strong_count(), 2);
    drop(lease);
    assert_eq!(witnessed.lock().unwrap().strong_count(), 1);
    drop(outer);
    assert_eq!(witnessed.lock().unwrap().strong_count(), 0);

    // after_start runs once the inner link is up and can hold or fail the start.
    let inner = TestTransport::new();
    let seen = inner.handle();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let hold = Arc::clone(&gate);
    let mut held = TestTransport::builder()
        .inner(inner)
        .after_start(move || {
            let hold = Arc::clone(&hold);
            async move {
                hold.acquire().await.unwrap().forget();
                Ok(())
            }
        })
        .build();
    {
        let start = held.start();
        tokio::pin!(start);
        tokio::select! {
            biased;
            _ = &mut start => panic!("the start was not held"),
            () = std::future::ready(()) => {}
        }
        assert_eq!(seen.starts(), 1, "the inner transport is already started");
        gate.add_permits(1);
        let _ = start.await.unwrap();
    }
    let mut failing = TestTransport::builder()
        .after_start(|| async { Err(Error::Encoding("held start failed".into())) })
        .build();
    assert!(failing.start().await.is_err());
}

#[tokio::test]
async fn intercept_decides_what_reaches_the_inner_transport() {
    for mode in [SendMode::Forward, SendMode::ForwardFirst] {
        let inner = TestTransport::new();
        let seen = inner.handle();
        let outer = TestTransport::builder()
            .inner(inner)
            .unicast(mode)
            .intercept(|frame| async move { frame.mac.as_slice() != [9] })
            .build();
        outer.send_unicast(&[1], &[9]).await.unwrap();
        assert_eq!(seen.sent().len(), 0, "{mode:?}: the dropped frame");
        outer.send_unicast(&[2], &[8]).await.unwrap();
        assert_eq!(seen.sent().len(), 1, "{mode:?}");
        assert_eq!(seen.sent().frame(0).npdu.as_ref(), &[2]);
        assert_eq!(outer.sent().len(), 2, "both are recorded here");
    }
}

#[tokio::test]
async fn relay_passes_the_inner_frames_through_the_hook_and_takes_injected_ones() {
    let (a, mut b) = crate::loopback::LoopbackTransport::pair(vec![1], vec![2]);
    let _ = b.start().await.unwrap();
    let mut outer = TestTransport::builder()
        .inner(a)
        .start(StartMode::Relay(8))
        .on_inbound(|frame| async move { (frame.npdu[0] != 0).then_some(frame) })
        .build();
    let handle = outer.handle();
    assert!(handle.inbound().is_none());
    let mut received = outer.start().await.unwrap();
    b.send_unicast(&[0], &[1]).await.unwrap();
    b.send_unicast(&[5], &[1]).await.unwrap();
    assert_eq!(
        &received.recv().await.unwrap().npdu[..],
        &[5],
        "the 0 frame was swallowed"
    );

    // A test can inject a frame beside the link's own.
    handle
        .inbound()
        .unwrap()
        .send(ReceivedNpdu::unverified(
            Bytes::from_static(&[7]),
            MacAddr::from_slice(&[3]),
            false,
            Vec::new(),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(&received.recv().await.unwrap().npdu[..], &[7]);

    // Stopping drops the handle's sender; the relay ends with the link.
    outer.stop().await.unwrap();
    assert!(handle.inbound().is_none());

    // Without an inner transport a relay start fails.
    let mut bare = TestTransport::builder().start(StartMode::Relay(1)).build();
    assert!(bare.start().await.is_err());
}

#[tokio::test]
async fn started_reaches_the_concrete_inner_transport_and_sends_see_its_current_limits() {
    let (a, _b) = crate::loopback::LoopbackTransport::pair(vec![1], vec![2]);
    let macs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (seen, other) = (Arc::clone(&macs), Arc::clone(&macs));
    let inner = TestTransport::builder().receive_capacity(100).build();
    let inner_handle = inner.handle();
    let builder = TestTransport::builder()
        .inner(a)
        .on_started(move |started| {
            seen.lock().unwrap().push(
                started
                    .inner::<crate::loopback::LoopbackTransport>()
                    .map(|link| link.local_mac().to_vec()),
            );
            assert!(started.inner::<TestTransport>().is_none(), "another type");
        });
    let mut outer = builder.build();
    let _ = outer.start().await.unwrap();
    assert_eq!(*other.lock().unwrap(), vec![Some(vec![1])]);

    // A send's hook sees the inner transport's limits as they are now.
    let sampled = Arc::new(std::sync::Mutex::new(Vec::new()));
    let log = Arc::clone(&sampled);
    let builder = TestTransport::builder()
        .inner(inner)
        .unicast(SendMode::Forward);
    let handle = builder.handle();
    let outer = builder
        .on_send(move |_| {
            log.lock()
                .unwrap()
                .push(handle.inner_limits().map(|limits| limits.receive_capacity));
            async { Ok(()) }
        })
        .build();
    outer.send_unicast(&[1], &[2]).await.unwrap();
    inner_handle.set_receive_capacity(40);
    outer.send_unicast(&[1], &[2]).await.unwrap();
    assert_eq!(*sampled.lock().unwrap(), vec![Some(100), Some(40)]);
}
