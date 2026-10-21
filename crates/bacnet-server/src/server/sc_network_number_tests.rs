use super::super::*;
use bacnet_endpoint_core::coordinator::CanonicalPeer;
#[path = "../../tests/support/sc_number_socket.rs"]
mod socket;
use socket::*;

#[tokio::test]
async fn sc_number_server_blocked_write_flood_ack_handler_and_canceled_stop() {
    let (transport, peer, observed) = fixture();
    let (server, ()) = bounded(async {
        tokio::join!(
            BACnetServer::start(
                ServerConfig {
                    device: bacnet_objects::DeviceIdentity::new(7, 0).unwrap(),
                    ..Default::default()
                },
                database(),
                transport
            ),
            peer.accept()
        )
    })
    .await;
    let mut server = server.unwrap();
    let (operation, acknowledged) = server
        .notification_transactions
        .reserve(
            CanonicalPeer::direct(&PEER),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        )
        .unwrap();
    peer.learn_and_query().await;
    count(&observed.numbers_started, 1).await;
    // The worker cannot drain its 256-entry input while physically sending.
    // Pace by actual network intake, not by sleeps or the SC origin quota.
    for sequence in 3..=260 {
        peer.send(false, &[1, 0x80, 0x12]).await;
        bounded(async {
            while server.test_network().network_control_ingress_sequence() < sequence {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }
    peer.send(
        false,
        &[
            1,
            0,
            0x20,
            operation.invoke_id(),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION.to_raw(),
        ],
    )
    .await;
    assert!(matches!(
        bounded(acknowledged).await.unwrap(),
        CovAckResult::Ack
    ));
    drop(operation);
    peer.send(false, &write_npdu()).await;
    handled(server.database()).await;
    assert_eq!(observed.numbers_completed.load(Ordering::SeqCst), 0);
    observed.hold_disconnect.store(true, Ordering::SeqCst);
    {
        let stop = server.stop();
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
    observed.disconnect_release.add_permits(1);
    bounded(server.stop()).await.unwrap();
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
async fn sc_number_server_live_release_and_bare_drop_join() {
    for bare_drop in [false, true] {
        let (transport, peer, observed) = fixture();
        let (server, ()) = bounded(async {
            tokio::join!(
                BACnetServer::start(
                    ServerConfig {
                        device: bacnet_objects::DeviceIdentity::new(7, 0).unwrap(),
                        ..Default::default()
                    },
                    database(),
                    transport
                ),
                peer.accept()
            )
        })
        .await;
        let mut server = server.unwrap();
        peer.learn_and_query().await;
        count(&observed.numbers_started, 1).await;
        if !bare_drop {
            observed.number_release.add_permits(1);
            peer.number().await;
            assert_eq!(observed.numbers_completed.load(Ordering::SeqCst), 1);
            bounded(server.stop()).await.unwrap();
        }
        drop(server);
        count(&observed.socket_dropped, 1).await;
        assert_eq!(observed.numbers_dropped.load(Ordering::SeqCst), 1);
        assert_eq!(
            observed.numbers_completed.load(Ordering::SeqCst),
            usize::from(!bare_drop)
        );
    }
}
