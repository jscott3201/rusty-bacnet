use super::*;
use std::sync::atomic::Ordering;

#[tokio::test]
async fn client_number_cancelled_stop_retains_task_joins_and_transport_cleanup() {
    let (mut client, inbound, mut outbound, gates) = harness(true).await;
    inject(&inbound, &number(77, 1), true).await;
    inject(&inbound, QUERY, false).await;
    bounded(gates.entered.acquire()).await.unwrap().forget();
    inject(&inbound, QUERY, false).await;
    gates.hold_stop.store(true, Ordering::SeqCst);
    {
        let stop = client.stop();
        tokio::pin!(stop);
        assert!(futures_util::poll!(&mut stop).is_pending());
    }
    assert!(
        client.network_number_task.is_some(),
        "canceled waiter retains Number join"
    );
    assert!(
        client.dispatch_task.is_some(),
        "canceled waiter retains dispatch join"
    );
    {
        let stop = client.stop();
        tokio::pin!(stop);
        bounded(async {
            tokio::select! {
                _ = &mut stop => panic!("transport stop must remain held"),
                permit = gates.stop_entered.acquire() => permit.unwrap().forget(),
            }
        })
        .await;
    }
    // Joining Number and dispatch released both network Arcs before transport stop.
    assert!(client.network_number_task.is_none());
    assert!(client.dispatch_task.is_none());
    bounded(gates.dropped.acquire()).await.unwrap().forget();
    gates.stop_release.add_permits(1);
    bounded(client.stop()).await.unwrap();
    bounded(inbound.closed()).await;
    gates.release.add_permits(2);
    drop(client);
    bounded(gates.transport_dropped.acquire())
        .await
        .unwrap()
        .forget();
    assert!(
        bounded(outbound.recv()).await.is_none(),
        "pending and queued Number sends canceled"
    );
}

#[tokio::test]
async fn client_number_drop_aborts_pending_and_queued_replies() {
    let (client, inbound, mut outbound, gates) = harness(true).await;
    inject(&inbound, &number(77, 1), true).await;
    inject(&inbound, QUERY, false).await;
    bounded(gates.entered.acquire()).await.unwrap().forget();
    inject(&inbound, QUERY, false).await;
    drop(client);
    bounded(gates.dropped.acquire()).await.unwrap().forget();
    bounded(gates.transport_dropped.acquire())
        .await
        .unwrap()
        .forget();
    bounded(inbound.closed()).await;
    gates.release.add_permits(2);
    assert!(bounded(outbound.recv()).await.is_none());
}

#[tokio::test]
async fn client_number_worker_queue_is_bounded_and_closes_on_abort() {
    use super::super::network_number;
    use bacnet_network::network_number::NumberControl;
    let (mut client, _, mut outbound, gates) = harness(true).await;
    let owner = network_number::Owner {
        discovery: std::sync::Arc::clone(&client.device_table),
        device_tx: client.device_tx.clone(),
        collision_tx: client.device_collision_tx.clone(),
        link_returns: None,
    };
    let (tx, task) = network_number::spawn(&client.network, owner);
    tx.try_send(NumberControl::NumberIs {
        number: 77,
        flag: 1,
    })
    .unwrap();
    tx.try_send(NumberControl::WhatIs { broadcast: false })
        .unwrap();
    bounded(gates.entered.acquire()).await.unwrap().forget();
    // The reply follows the announcement, so the layer already holds it.
    assert_eq!(client.network.local_network_number().get(), Some(77));
    for _ in 0..network_number::CAPACITY {
        tx.try_send(NumberControl::WhatIs { broadcast: false })
            .unwrap();
    }
    assert!(matches!(
        tx.try_send(NumberControl::WhatIs { broadcast: false }),
        Err(tokio::sync::mpsc::error::TrySendError::Full(_))
    ));
    task.abort();
    let _ = task.await;
    assert!(matches!(
        tx.try_send(NumberControl::WhatIs { broadcast: false }),
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_))
    ));
    client.stop().await.unwrap();
    drop(client);
    assert!(bounded(outbound.recv()).await.is_none());
}
