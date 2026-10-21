use super::super::*;
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_transport::sc::ScTransport;
use bacnet_types::enums::ConfirmedServiceChoice as S;
#[path = "../../bacnet-server/tests/support/sc_number_socket.rs"]
mod socket;
use socket::*;
use std::future::Future;

#[tokio::test]
async fn sc_number_control_queue_full_closed_and_apdu_progress() {
    use bacnet_network::layer::{NetworkLayer, QueueAdmissionSnapshot};
    let (transport, peer, observed) = fixture();
    let mut network = NetworkLayer::new(transport);
    let mut controls = network
        .enable_network_control_receiver_with_admission()
        .unwrap();
    let counters = controls.counters();
    let (apdus, ()) = bounded(async { tokio::join!(network.start(), peer.accept()) }).await;
    let mut apdus = apdus.unwrap();
    for sequence in 1..=257 {
        peer.send(false, &[1, 0x80, 0x12]).await;
        // Controls are decoded as the APDU receiver is polled (#1713).
        bounded(async {
            while network.network_control_ingress_sequence() < sequence {
                assert!(apdus.try_recv().is_err());
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    peer.send(false, &[1, 0, 0x10, 8]).await;
    assert_eq!(
        bounded(apdus.recv()).await.unwrap().apdu.as_ref(),
        &[0x10, 8]
    );
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
    for _ in 0..256 {
        assert!(controls.try_recv().is_ok());
    }
    assert!(controls.try_recv().is_err());
    drop(controls);
    for _ in 0..3 {
        peer.send(false, &[1, 0x80, 0x12]).await;
        peer.send(false, &[1, 0, 0x10, 8]).await;
        assert_eq!(
            bounded(apdus.recv()).await.unwrap().apdu.as_ref(),
            &[0x10, 8]
        );
    }
    assert_eq!(network.network_control_ingress_sequence(), 258);
    assert_eq!(
        counters.snapshot(),
        QueueAdmissionSnapshot {
            current_depth: 0,
            high_water: 256,
            full_drops: 1,
            fairness_drops: 0,
            closed_drops: 1,
        }
    );
    bounded(network.stop()).await.unwrap();
    count(&observed.socket_dropped, 1).await;
}

async fn start() -> (
    EndpointSession<ScTransport<GateSocket>>,
    Peer,
    Arc<Observed>,
) {
    let (transport, peer, observed) = fixture();
    let mut session = EndpointSession::new(
        transport,
        SessionRole::Both,
        SessionConfig {
            queue_capacity: 2,
            ..Default::default()
        },
    )
    .unwrap()
    .with_database(database())
    .with_device_writes(Arc::new(|_| true));
    let (result, ()) = bounded(async { tokio::join!(session.start(), peer.accept()) }).await;
    result.unwrap();
    (session, peer, observed)
}
async fn ack(session: &EndpointSession<ScTransport<GateSocket>>, peer: &Peer) {
    let (operation, acknowledged) = session
        .notifications
        .as_ref()
        .unwrap()
        .reserve(
            CanonicalPeer::direct(&PEER),
            S::CONFIRMED_AUDIT_NOTIFICATION,
        )
        .unwrap();
    peer.send(
        false,
        &[
            1,
            0,
            0x20,
            operation.invoke_id(),
            S::CONFIRMED_AUDIT_NOTIFICATION.to_raw(),
        ],
    )
    .await;
    assert!(matches!(
        bounded(acknowledged).await.unwrap(),
        bacnet_server::server::CovAckResult::Ack
    ));
}

#[tokio::test]
async fn sc_number_endpoint_blocked_write_flood_ack_handler_queue_and_canceled_stop() {
    let (mut session, peer, observed) = start().await;
    peer.learn_and_query().await;
    count(&observed.numbers_started, 1).await;
    // Each terminal ACK is an ingress FIFO barrier and independently admitted
    // Audit transaction. Control saturation cannot block this live consumer.
    for _ in 0..258 {
        peer.send(false, &[1, 0x80, 0x12]).await;
        ack(&session, &peer).await;
    }
    let egress = session.egress.as_ref().unwrap().clone();
    let enqueue = || egress.send_network_number_is(vec![1, 0x80, 0x13, 0, 17, 0]);
    let mut queued1 = Box::pin(enqueue());
    let mut queued2 = Box::pin(enqueue());
    for queued in [&mut queued1, &mut queued2] {
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                queued.as_mut().poll(cx).is_pending()
            ))
            .await
        );
    }
    assert!(bounded(enqueue())
        .await
        .unwrap_err()
        .to_string()
        .contains("queue is full"));
    peer.send(false, &write_npdu()).await;
    handled(session.database.as_ref().unwrap()).await;
    assert_eq!(observed.numbers_completed.load(Ordering::SeqCst), 0);
    // Number sends are caller-owned, unlike detached ordinary APDU commands.
    // Cancel both queued waiters before the active worker is stopped.
    drop(queued1);
    drop(queued2);
    observed.hold_disconnect.store(true, Ordering::SeqCst);
    {
        let stop = session.stop();
        tokio::pin!(stop);
        bounded(async {
            tokio::select! { biased;
                _ = &mut stop => panic!("held transport stop completed"),
                _ = count(&observed.disconnect_started, 1) => {}
            }
        })
        .await;
    }
    assert_eq!(observed.numbers_dropped.load(Ordering::SeqCst), 1);
    assert!(!egress.is_open());
    assert!(bounded(enqueue()).await.is_err());
    observed.disconnect_release.add_permits(1);
    bounded(session.stop()).await.unwrap();
    count(&observed.socket_dropped, 1).await;
    assert_eq!(observed.numbers_started.load(Ordering::SeqCst), 1);
    assert_eq!(observed.numbers_completed.load(Ordering::SeqCst), 0);
    assert_eq!(
        tokio::runtime::Handle::current()
            .metrics()
            .num_alive_tasks(),
        0
    );
}

#[tokio::test]
async fn sc_number_endpoint_live_release_and_bare_drop_join() {
    for bare_drop in [false, true] {
        let (mut session, peer, observed) = start().await;
        peer.learn_and_query().await;
        count(&observed.numbers_started, 1).await;
        if !bare_drop {
            observed.number_release.add_permits(1);
            peer.number().await;
            assert_eq!(observed.numbers_completed.load(Ordering::SeqCst), 1);
            bounded(session.stop()).await.unwrap();
        }
        drop(session);
        count(&observed.socket_dropped, 1).await;
        assert_eq!(observed.numbers_dropped.load(Ordering::SeqCst), 1);
        assert_eq!(
            observed.numbers_completed.load(Ordering::SeqCst),
            usize::from(!bare_drop)
        );
    }
}
