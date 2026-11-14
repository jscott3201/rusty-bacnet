//! The inner transport, the start modes and the send modes built on it.
use super::*;
use crate::loopback::LoopbackTransport;
use crate::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::time::timeout;

fn mstp_inner() -> (MstpTransport<LoopbackSerial>, LoopbackSerial) {
    let (serial, peer) = LoopbackSerial::pair();
    let config = MstpConfig {
        this_station: 4,
        max_master: 5,
        ..MstpConfig::default()
    };
    (MstpTransport::new(serial, config), peer)
}

/// An inner test transport that answers each rule with its own value.
fn answering() -> (TestTransport, TestTransportHandle) {
    let inner = TestTransport::builder()
        .local_mac(&[9])
        .receive_capacity(100)
        .egress_limit(90)
        .max_npdu(NpduLimits::both(80))
        .number_controls()
        .normal_bip(SocketAddrV4::new([10, 0, 0, 1].into(), 47808))
        .broadcast_mac(&[0xFF])
        .group_mac(&[0xE0])
        .on_bip_broadcast_endpoint(|| Some(SocketAddrV4::new([10, 0, 0, 255].into(), 47808)))
        .build();
    let handle = inner.handle();
    (inner, handle)
}

#[test]
fn any_transport_can_stand_behind_it_and_answers_what_the_test_does_not_set() {
    let (inner, _) = answering();
    let outer = TestTransport::builder().inner(inner).build();
    assert_eq!(outer.local_mac(), &[9]);
    assert_eq!(outer.local_receive_apdu_capacity(), 100);
    assert_eq!(outer.egress_apdu_limit(), 90);
    assert_eq!(outer.max_npdu(), NpduLimits::both(80));
    assert!(outer.supports_local_nonrouter_number_controls());
    assert_eq!(outer.bip_port().unwrap().endpoint.port(), 47808);
    assert!(outer.bip6_port().is_none());
    assert_eq!(
        outer.bip_broadcast_endpoint(),
        Some(SocketAddrV4::new([10, 0, 0, 255].into(), 47808))
    );
    assert!(outer.is_broadcast_mac(&[0xFF]));
    assert!(outer.is_group_destination(&[0xE0]));
    assert!(outer.group_destinations().contains(&[0xE0]));
    assert!(outer.local_broadcast_destinations().contains(&[0xFF]));

    // A real transport behind a test transport behind another.
    let (mstp, _peer) = mstp_inner();
    let reference = TransportPort::max_npdu(&mstp);
    let outer = TestTransport::builder()
        .inner(TestTransport::builder().inner(mstp).build())
        .build();
    assert_eq!(outer.local_mac(), &[4]);
    assert_eq!(outer.max_npdu(), reference);
    assert!(outer.is_broadcast_mac(&[0xFF]));
}

#[test]
fn what_the_test_sets_overrides_the_inner_transport_one_rule_at_a_time() {
    let (inner, _) = answering();
    let outer = TestTransport::builder()
        .inner(inner)
        .local_mac(&[3])
        .receive_capacity(10)
        .egress_limit(11)
        .max_npdu(NpduLimits::both(12))
        .build();
    assert_eq!(outer.local_mac(), &[3]);
    assert_eq!(outer.local_receive_apdu_capacity(), 10);
    assert_eq!(outer.egress_apdu_limit(), 11);
    assert_eq!(outer.max_npdu(), NpduLimits::both(12));
    // Only the setting named moved: the rest still comes from the inner one.
    assert!(outer.supports_local_nonrouter_number_controls());
    assert!(outer.is_broadcast_mac(&[0xFF]));

    // An own group MAC replaces the inner group rule and keeps its broadcast.
    let (inner, _) = answering();
    let outer = TestTransport::builder()
        .inner(inner)
        .group_mac(&[0xA0])
        .build();
    assert!(outer.is_broadcast_mac(&[0xFF]));
    assert!(outer.is_group_destination(&[0xFF]));
    assert!(outer.is_group_destination(&[0xA0]));
    assert!(!outer.is_group_destination(&[0xE0]));

    // An own broadcast MAC replaces the inner broadcast and group rules.
    let (inner, _) = answering();
    let outer = TestTransport::builder()
        .inner(inner)
        .broadcast_mac(&[0xEE])
        .build();
    assert!(outer.is_broadcast_mac(&[0xEE]));
    assert!(!outer.is_broadcast_mac(&[0xFF]));
    assert!(!outer.is_group_destination(&[0xE0]));
    assert!(outer.group_destinations().contains(&[0xEE]));

    // Own ports and Number controls win over the inner transport's.
    let (inner, _) = answering();
    let outer = TestTransport::builder()
        .inner(inner)
        .normal_bip(SocketAddrV4::new([10, 0, 0, 2].into(), 47809))
        .build();
    assert_eq!(outer.bip_port().unwrap().endpoint.port(), 47809);
    let (publish, changes) = watch::channel(NpduLimits::both(77));
    let outer = TestTransport::builder()
        .inner(TestTransport::builder().max_npdu_changes(changes).build())
        .build();
    assert_eq!(
        *outer.max_npdu_changes().unwrap().borrow(),
        NpduLimits::both(77),
        "the inner transport's publisher"
    );
    drop(publish);
}

#[test]
fn a_network_port_lease_goes_to_whoever_reported_the_port() {
    let endpoint = SocketAddrV4::new([10, 0, 0, 1].into(), 47808);
    let lease = Arc::new(());

    // The inner transport reported the port: it holds the lease.
    let mut outer = TestTransport::builder()
        .inner(TestTransport::builder().normal_bip(endpoint).build())
        .build();
    outer
        .retain_network_port_lease_internal(Arc::clone(&lease))
        .unwrap();
    assert_eq!(Arc::strong_count(&lease), 2);
    drop(outer);
    assert_eq!(Arc::strong_count(&lease), 1);

    // The test transport reported it: it holds the lease itself.
    let mut outer = TestTransport::builder().normal_bip(endpoint).build();
    outer
        .retain_network_port_lease_internal(Arc::clone(&lease))
        .unwrap();
    assert_eq!(Arc::strong_count(&lease), 2);
    drop(outer);

    // No port anywhere: refused, and nobody keeps the lease.
    let mut outer = TestTransport::builder().inner(TestTransport::new()).build();
    assert!(
        outer
            .retain_network_port_lease_internal(Arc::clone(&lease))
            .is_err()
    );
    assert_eq!(Arc::strong_count(&lease), 1);
}

#[tokio::test]
async fn lifecycle_reaches_the_inner_transport_unless_replaced() {
    // Start, stop, abort and prepare all go on to the inner transport.
    let inner = TestTransport::new();
    let seen = inner.handle();
    let mut outer = TestTransport::builder().inner(inner).build();
    outer.prepare_for_routing();
    let _ = outer.start().await.unwrap();
    outer.stop().await.unwrap();
    outer.abort();
    assert_eq!(
        (seen.prepares(), seen.starts(), seen.stops(), seen.aborts()),
        (1, 1, 1, 1)
    );

    // A skipped prepare never reaches it.
    let inner = TestTransport::new();
    let seen = inner.handle();
    let mut outer = TestTransport::builder()
        .inner(inner)
        .skip_prepare_for_routing()
        .build();
    outer.prepare_for_routing();
    assert_eq!((outer.handle().prepares(), seen.prepares()), (1, 0));

    // An on_stop hook replaces the inner stop, and its error is returned.
    let inner = TestTransport::new();
    let seen = inner.handle();
    let mut outer = TestTransport::builder()
        .inner(inner)
        .on_stop(|| async { Err(Error::Encoding("replaced".into())) })
        .build();
    assert!(outer.stop().await.is_err());
    assert_eq!(seen.stops(), 0);

    // An inbound receiver or a fresh channel replaces the inner start.
    let inner = TestTransport::new();
    let seen = inner.handle();
    let (_tx, rx) = mpsc::channel(1);
    let mut outer = TestTransport::builder().inner(inner).inbound(rx).build();
    let _ = outer.start().await.unwrap();
    assert_eq!(seen.starts(), 0);
    let inner = TestTransport::new();
    let seen = inner.handle();
    let mut outer = TestTransport::builder()
        .inner(inner)
        .start(StartMode::Fresh(1))
        .build();
    let _ = outer.start().await.unwrap();
    assert_eq!(seen.starts(), 0);

    // The inner start is used otherwise, so its own failure comes back.
    let mut outer = TestTransport::builder()
        .inner(TestTransport::never_start())
        .build();
    let panicked = tokio::spawn(async move {
        let _ = outer.start().await;
    })
    .await;
    assert!(panicked.unwrap_err().is_panic());
}

#[test]
fn the_handle_reads_the_inner_transports_limits() {
    let (inner, inner_handle) = answering();
    let outer = TestTransport::builder().inner(inner).build();
    let handle = outer.handle();
    assert_eq!(handle.inner_limits(), None, "nothing asked yet");
    let _ = outer.max_npdu();
    assert_eq!(
        handle.inner_limits(),
        Some(InnerLimits {
            receive_capacity: 100,
            egress_limit: 90,
            max_npdu: NpduLimits::both(80),
        })
    );
    inner_handle.set_receive_capacity(40);
    inner_handle.set_egress_limit(41);
    inner_handle.set_max_npdu(NpduLimits::both(42));
    assert_eq!(outer.local_receive_apdu_capacity(), 40);
    assert_eq!(
        handle.inner_limits(),
        Some(InnerLimits {
            receive_capacity: 40,
            egress_limit: 41,
            max_npdu: NpduLimits::both(42),
        })
    );
    assert_eq!(TestTransport::new().handle().inner_limits(), None);
}

#[tokio::test]
async fn a_mac_can_be_reported_only_until_the_start() {
    let mut transport = TestTransport::builder()
        .local_mac_until_started(&[7])
        .local_mac(&[1])
        .build();
    assert_eq!(transport.local_mac(), &[7]);
    let _ = transport.start().await.unwrap();
    assert_eq!(transport.local_mac(), &[1]);

    // After the start the inner transport's MAC shows, as on a link that
    // learns its address then. A failed start leaves the placeholder.
    let (mstp, _peer) = mstp_inner();
    let mut transport = TestTransport::builder()
        .inner(mstp)
        .local_mac_until_started(&[7])
        .start_gate(|| async { Err(Error::Encoding("no".into())) })
        .build();
    assert!(transport.start().await.is_err());
    assert_eq!(transport.local_mac(), &[7]);
    let (mstp, _peer) = mstp_inner();
    let mut transport = TestTransport::builder()
        .inner(mstp)
        .local_mac_until_started(&[7])
        .build();
    assert_eq!(transport.local_mac(), &[7]);
    let _ = transport.start().await.unwrap();
    assert_eq!(transport.local_mac(), &[4]);
}

#[tokio::test]
async fn a_fresh_channel_each_start_closes_when_the_link_stops() {
    let mut transport = TestTransport::builder().start(StartMode::Fresh(4)).build();
    let handle = transport.handle();
    assert!(handle.inbound().is_none(), "nothing before a start");

    let mut first = transport.start().await.unwrap();
    let tx = handle.inbound().unwrap();
    tx.send(ReceivedNpdu::unverified(
        Bytes::from_static(&[1]),
        MacAddr::from_slice(&[2]),
        false,
        Vec::new(),
        None,
    ))
    .await
    .unwrap();
    assert_eq!(&first.recv().await.unwrap().npdu[..], &[1]);

    // Stopping drops the handle's sender; the receiver closes once the test
    // drops its own clone.
    transport.stop().await.unwrap();
    assert!(handle.inbound().is_none());
    drop(tx);
    assert!(first.recv().await.is_none());

    // A second start makes a new channel; abort closes it too.
    let mut second = transport.start().await.unwrap();
    assert!(handle.inbound().is_some());
    transport.abort();
    assert!(handle.inbound().is_none());
    assert!(second.recv().await.is_none());
    assert_eq!(handle.starts(), 2);
}

#[tokio::test]
async fn fail_mode_records_the_frame_and_refuses_the_send() {
    let hooked = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&hooked);
    let transport = TestTransport::builder()
        .unicast(SendMode::Fail("no route"))
        .on_send(move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .build();
    let handle = transport.handle();
    handle.fail_next_send();
    let error = transport.send_unicast(&[1], &[2]).await.unwrap_err();
    assert!(
        matches!(&error, Error::Encoding(m) if m == "no route"),
        "{error}"
    );
    assert_eq!(handle.sent().len(), 1, "the refused frame is recorded");
    assert_eq!(hooked.load(Ordering::SeqCst), 0, "the hook is skipped");
    // The broadcast kind is unaffected, and still takes the armed failure.
    assert!(transport.send_broadcast(&[1]).await.is_err());
    assert_eq!(hooked.load(Ordering::SeqCst), 1);
    transport.send_broadcast(&[1]).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn an_inner_transport_supplies_identity_and_takes_prepare() {
    let (mstp, _peer) = mstp_inner();
    let reference = mstp_inner().0;
    let mut transport = TestTransport::builder().inner(mstp).build();
    assert_eq!(transport.local_mac(), &[4]);
    assert!(transport.is_broadcast_mac(&[0xFF]));
    assert!(transport.group_destinations().contains(&[0xFF]));
    assert!(transport.local_broadcast_destinations().contains(&[0xFF]));
    assert_eq!(transport.max_npdu(), TransportPort::max_npdu(&reference));
    transport.prepare_for_routing();
    assert_ne!(
        transport.max_npdu(),
        TransportPort::max_npdu(&reference),
        "the inner transport was prepared"
    );

    // A skipped prepare leaves the inner transport as it was.
    let (mstp, _peer) = mstp_inner();
    let mut transport = TestTransport::builder()
        .inner(mstp)
        .skip_prepare_for_routing()
        .build();
    let before = transport.max_npdu();
    transport.prepare_for_routing();
    assert_eq!(transport.max_npdu(), before);
}

#[tokio::test]
async fn forward_mode_sends_to_the_inner_transport_after_the_pipeline() {
    let (a, mut b) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut from_a = b.start().await.unwrap();
    let mut transport = TestTransport::builder()
        .inner(a)
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Record)
        .build();
    let _ = transport.start().await.unwrap();
    let handle = transport.handle();

    transport.send_unicast(&[0xAA], &[2]).await.unwrap();
    let frame = from_a.recv().await.unwrap();
    assert_eq!(&frame.npdu[..], &[0xAA]);
    assert_eq!(frame.source_mac.as_slice(), &[1]);
    assert_eq!(handle.sent().len(), 1, "a forwarded send is logged first");

    // A broadcast in Record mode stops here.
    transport.send_broadcast(&[0xBB]).await.unwrap();
    assert!(from_a.try_recv().is_err());
    assert_eq!(handle.sent().broadcasts().len(), 1);

    // A refused send is logged, and the controls run before the forward.
    handle.fail_next_send();
    assert!(transport.send_unicast(&[0xCC], &[2]).await.is_err());
    assert!(from_a.try_recv().is_err());
    assert_eq!(handle.sent().len(), 3);

    // The inner transport's own refusal comes back.
    drop((b, from_a));
    assert!(transport.send_unicast(&[0xDD], &[2]).await.is_err());
}

#[tokio::test]
async fn forward_mode_without_an_inner_transport_fails() {
    let transport = TestTransport::builder()
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward)
        .build();
    assert!(transport.send_unicast(&[1], &[2]).await.is_err());
    assert!(transport.send_broadcast(&[1]).await.is_err());
    assert_eq!(transport.sent().len(), 2);
}

#[tokio::test]
async fn an_inner_transport_starts_once_and_its_frames_come_through() {
    let (a, mut b) = LoopbackTransport::pair(vec![1], vec![2]);
    let mut transport = TestTransport::builder().inner(a).build();
    let mut received = transport.start().await.unwrap();
    assert!(
        transport.start().await.is_err(),
        "the inner transport started once"
    );
    let _ = b.start().await.unwrap();
    b.send_unicast(&[5], &[1]).await.unwrap();
    let frame = timeout(Duration::from_secs(2), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&frame.npdu[..], &[5]);
    transport.stop().await.unwrap();
}
