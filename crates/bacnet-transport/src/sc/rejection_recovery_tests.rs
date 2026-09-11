//! Socket identity, admission, and fresh-only recovery after NAK expiry.

use super::*;

fn transport(client: GateSocket) -> ScTransport<GateSocket> {
    ScTransport::new(client, [1; 6])
        .with_device_uuid([1; 16])
        .with_connect_timeout_ms(40)
        .with_test_heartbeat_timing_ms(80, 240)
}

fn config(retries: u32) -> ScReconnectConfig {
    ScReconnectConfig {
        initial_delay_ms: 40,
        max_delay_ms: 40,
        max_retries: retries,
    }
}

fn snapshot(observed: &Observations) -> (usize, usize) {
    (
        observed.calls.lock().unwrap().len(),
        observed.receives.load(Ordering::SeqCst),
    )
}

fn assert_retired(observed: &Observations, before: (usize, usize)) {
    assert_eq!(snapshot(observed), before, "retired socket was used again");
    assert_eq!(observed.flushes.load(Ordering::SeqCst), 0);
    assert_eq!(observed.nak_completed.load(Ordering::SeqCst), 0);
    assert!(observed.buffered.lock().unwrap().is_some());
}

async fn expire(
    transport: &ScTransport<GateSocket>,
    hub: &LoopbackWebSocket,
    observed: &Observations,
) {
    observed.hold_nak.store(true, Ordering::SeqCst);
    hub.send(&rejection_wires()[2]).await.unwrap();
    wait_for_state(transport, ScConnectionState::Disconnected)
        .await
        .unwrap();
    assert_eq!(observed.nak_dropped.load(Ordering::SeqCst), 1);
}

async fn accept(hub: &LoopbackWebSocket, vmac: Vmac, npdu: u16, bvlc: u16) {
    let request = within(hub.recv()).await.unwrap();
    let msg = decode_sc_message(&request).unwrap();
    assert_eq!(msg.function, ScFunction::ConnectRequest);
    assert_eq!(&msg.payload[..6], &[1; 6]);
    assert_eq!(&msg.payload[6..22], &[1; 16]);
    let mut response = vec![7, 0, request[2], request[3]];
    response.extend_from_slice(&vmac);
    response.extend_from_slice(&[0x33; 16]);
    response.extend_from_slice(&bvlc.to_be_bytes());
    response.extend_from_slice(&npdu.to_be_bytes());
    hub.send(&response).await.unwrap();
}

#[tokio::test]
async fn rejection_deadline_no_factory_never_rehandshakes_or_flushes_retired_socket() {
    for retries in [0, 3] {
        let (client, hub, observed) = GateSocket::pair();
        let mut transport = transport(client).with_reconnect(config(retries));
        let _rx = started(&mut transport, &hub).await;
        expire(&transport, &hub, &observed).await;
        let before = snapshot(&observed);
        within(transport.recv_task.take().unwrap()).await.unwrap();
        assert!(transport.send_broadcast(&[1, 0]).await.is_err());
        transport.stop().await.unwrap();
        assert_retired(&observed, before);
    }
}

#[tokio::test]
async fn rejection_deadline_without_reconnect_does_not_dial_or_consume_failover() {
    let (client, hub, observed) = GateSocket::pair();
    let (failover, _failover_hub, failover_observed) = GateSocket::pair();
    let dials = Arc::new(AtomicUsize::new(0));
    let count = dials.clone();
    let mut transport = transport(client)
        .with_failover(failover)
        .with_connector(move || {
            count.fetch_add(1, Ordering::SeqCst);
            async { Err(Error::Encoding("unexpected dial".into())) }
        });
    let _rx = started(&mut transport, &hub).await;
    expire(&transport, &hub, &observed).await;
    within(transport.recv_task.take().unwrap()).await.unwrap();
    assert_eq!(dials.load(Ordering::SeqCst), 0);
    assert_eq!(snapshot(&failover_observed), (0, 0));
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn rejection_deadline_fresh_redial_validates_probe_identity_limits_and_publication() {
    fresh_redial_validates_probe(2).await;
}

#[tokio::test]
async fn empty_npdu_rejection_deadline_fresh_redial_validates_probe_identity_limits_and_publication(
) {
    fresh_redial_validates_probe(3).await;
}

async fn fresh_redial_validates_probe(wire_index: usize) {
    let (client, hub, observed) = GateSocket::pair();
    let (tx, mut dials) = mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let dial_count = count.clone();
    let mut transport = transport(client)
        .with_reconnect(config(2))
        .with_connector(move || {
            dial_count.fetch_add(1, Ordering::SeqCst);
            let (client, hub, observed) = GateSocket::pair();
            tx.send((hub, observed)).unwrap();
            async { Ok(client) }
        });
    let mut rx = started(&mut transport, &hub).await;
    observed.hold_nak.store(true, Ordering::SeqCst);
    hub.send(&rejection_wires()[wire_index]).await.unwrap();
    wait_for_state(&transport, ScConnectionState::Disconnected)
        .await
        .unwrap();
    assert_eq!(observed.nak_dropped.load(Ordering::SeqCst), 1);
    let old = snapshot(&observed);
    let (failed, failed_observed) = within(dials.recv()).await.unwrap();
    // Mismatched identity must not publish a fresh but unaccepted socket.
    let request = within(failed.recv()).await.unwrap();
    let mut wrong = vec![7, 0, request[2], request[3].wrapping_add(1)];
    wrong.extend_from_slice(&[0x66; 6]);
    wrong.extend_from_slice(&[0x55; 16]);
    wrong.extend_from_slice(&[0, 32, 0, 40]);
    failed.send(&wrong).await.unwrap();
    assert!(transport.send_broadcast(&[1, 0]).await.is_err());
    let (fresh, fresh_observed) = within(dials.recv()).await.unwrap();
    assert_ne!(
        transport.connection().unwrap().lock().await.hub_vmac,
        Some([0x66; 6])
    );
    {
        let conn = transport.connection().unwrap().lock().await;
        // Existing retry reset clears peer identity; the failed probe must not
        // commit its UUID or advertised 32/40 limits into that fresh state.
        assert_eq!(conn.hub_device_uuid, None);
        assert_eq!(conn.hub_max_bvlc_length, 1476);
        assert_eq!(conn.hub_max_apdu_length, 1476);
    }
    accept(&fresh, [0x11; 6], 32, 40).await;
    wait_for_state(&transport, ScConnectionState::Connected)
        .await
        .unwrap();
    assert_eq!(transport.max_apdu_length(), 28);
    assert_eq!(
        transport.connection().unwrap().lock().await.hub_vmac,
        Some([0x11; 6])
    );
    transport
        .send_unicast(&[1, 0, 0x30], &[0x44; 6])
        .await
        .unwrap();
    let sent = decode_sc_message(&recv_function(&fresh, 1).await).unwrap();
    assert_eq!(sent.destination_vmac, Some([0x44; 6]));
    assert_eq!(sent.payload.as_ref(), &[1, 0, 0x30]);
    assert!(transport.send_unicast(&[0; 33], &[0x44; 6]).await.is_err());
    let mut incoming = vec![1, 8, 0x66, 0x77];
    incoming.extend_from_slice(&[0x22; 6]);
    incoming.extend_from_slice(&[1, 0, 0x30]);
    fresh.send(&incoming).await.unwrap();
    assert_eq!(
        within(rx.recv()).await.unwrap().npdu.as_ref(),
        &[1, 0, 0x30]
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(
        failed_observed
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v[0] == 1)
            .count(),
        0
    );
    assert_eq!(
        fresh_observed
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|v| v[0] == 1)
            .count(),
        1
    );
    transport.stop().await.unwrap();
    assert_retired(&observed, old);
}

#[tokio::test]
async fn unknown_function_rejection_deadline_fresh_redial_validates_probe_identity_limits_and_publication(
) {
    fresh_redial_validates_probe(4).await;
}

#[tokio::test]
async fn rejection_deadline_unused_failover_works_but_poisoned_primary_never_restores() {
    let (client, hub, observed) = GateSocket::pair();
    let (failover, failover_hub, _failover_observed) = GateSocket::pair();
    let mut transport = transport(client)
        .with_reconnect(config(1))
        .with_failover(failover);
    let _rx = started(&mut transport, &hub).await;
    expire(&transport, &hub, &observed).await;
    let old = snapshot(&observed);
    accept(&failover_hub, [0x20; 6], 1476, 1476).await;
    wait_for_state(&transport, ScConnectionState::Connected)
        .await
        .unwrap();
    // More than several primary-restore intervals; accepted heartbeats keep the
    // healthy failover alive without permitting an old-primary Connect-Request.
    for id in 1..=8 {
        failover_hub.send(&[0x0A, 0, 0, id]).await.unwrap();
        assert_eq!(recv_function(&failover_hub, 0x0B).await, [0x0B, 0, 0, id]);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    assert_eq!(
        transport.connection().unwrap().lock().await.hub_vmac,
        Some([0x20; 6])
    );
    transport.send_broadcast(&[1, 0]).await.unwrap();
    assert_eq!(
        decode_sc_message(&recv_function(&failover_hub, 1).await)
            .unwrap()
            .destination_vmac,
        Some(BROADCAST_VMAC)
    );
    transport.stop().await.unwrap();
    assert_retired(&observed, old);
}

#[tokio::test]
async fn rejection_deadline_fresh_primary_restore_and_outstanding_disconnect_cleanup() {
    let (client, hub, observed) = GateSocket::pair();
    let (failover, failover_hub, failover_observed) = GateSocket::pair();
    let (tx, mut dials) = mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let dial_count = count.clone();
    let mut transport = transport(client)
        .with_reconnect(config(1))
        .with_failover(failover)
        .with_connector(move || {
            let attempt = dial_count.fetch_add(1, Ordering::SeqCst);
            let (client, hub, observed) = GateSocket::pair();
            if attempt == 1 {
                tx.send((hub, observed)).unwrap();
            }
            async move {
                if attempt == 1 {
                    Ok(client)
                } else {
                    Err(Error::Encoding("scripted dial failure".into()))
                }
            }
        });
    // Give the existing best-effort restore disconnect a long enough timeout
    // to overlap the next NAK budget; it still must not delay publication.
    transport.connect_timeout_ms = 1000;
    let _rx = started(&mut transport, &hub).await;
    expire(&transport, &hub, &observed).await;
    let old = snapshot(&observed);
    failover_observed
        .hold_disconnect
        .store(true, Ordering::SeqCst);
    accept(&failover_hub, [0x20; 6], 1476, 1476).await;
    let (fresh, fresh_observed) = within(dials.recv()).await.unwrap();
    accept(&fresh, [0x11; 6], 32, 40).await;
    within(async {
        while transport.connection().unwrap().lock().await.hub_vmac != Some([0x11; 6]) {
            tokio::task::yield_now().await;
        }
    })
    .await;
    wait_count(&failover_observed.disconnect_started, 1).await;
    transport.send_broadcast(&[1, 0]).await.unwrap();
    recv_function(&fresh, 1).await;
    // A second retirement cancels and joins the previous restore-disconnect
    // future; releasing its gate later must not resume transport-owned I/O.
    expire(&transport, &fresh, &fresh_observed).await;
    wait_count(&failover_observed.disconnect_dropped, 1).await;
    let fresh_before = snapshot(&fresh_observed);
    within(transport.recv_task.take().unwrap()).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert!(transport.restore_disconnect_task.lock().unwrap().is_none());
    failover_observed.release.notify_waiters();
    assert_eq!(
        failover_observed.disconnect_started.load(Ordering::SeqCst),
        1
    );
    transport.stop().await.unwrap();
    assert_retired(&observed, old);
    assert_retired(&fresh_observed, fresh_before);
}

#[tokio::test]
async fn rejection_deadline_active_failover_expiry_does_not_reuse_or_change_recovery_order() {
    let (client, hub, observed) = GateSocket::pair();
    drop(hub); // Initial primary handshake fails, untouched failover is allowed.
    let (failover, failover_hub, failover_observed) = GateSocket::pair();
    let mut transport =
        transport(client)
            .with_failover(failover)
            .with_reconnect(ScReconnectConfig {
                initial_delay_ms: 500,
                max_delay_ms: 500,
                max_retries: 1,
            });
    let (rx, ()) = tokio::join!(
        transport.start(),
        accept(&failover_hub, [0x20; 6], 1476, 1476)
    );
    let _rx = rx.unwrap();
    let primary_before = snapshot(&observed);
    expire(&transport, &failover_hub, &failover_observed).await;
    let failover_before = snapshot(&failover_observed);
    within(transport.recv_task.take().unwrap()).await.unwrap();
    assert_eq!(
        *transport.connection_state_changes().borrow(),
        ScConnectionState::Disconnected
    );
    transport.stop().await.unwrap();
    assert_eq!(
        snapshot(&observed),
        primary_before,
        "active failover exhaustion must not add a primary fallback"
    );
    assert_retired(&failover_observed, failover_before);
}

#[tokio::test]
async fn rejection_deadline_active_failover_redials_only_fresh_failover() {
    let (client, hub, observed) = GateSocket::pair();
    let (tx, mut dials) = mpsc::unbounded_channel();
    let count = Arc::new(AtomicUsize::new(0));
    let dial_count = count.clone();
    let mut transport = transport(client)
        .with_reconnect(config(1))
        .with_failover_connector(move || {
            dial_count.fetch_add(1, Ordering::SeqCst);
            let (client, hub, observed) = GateSocket::pair();
            tx.send((hub, observed)).unwrap();
            async { Ok(client) }
        });
    let _rx = started(&mut transport, &hub).await;
    expire(&transport, &hub, &observed).await;
    let primary_before = snapshot(&observed);
    let (first, first_observed) = within(dials.recv()).await.unwrap();
    accept(&first, [0x20; 6], 1476, 1476).await;
    wait_for_state(&transport, ScConnectionState::Connected)
        .await
        .unwrap();
    expire(&transport, &first, &first_observed).await;
    let failover_before = snapshot(&first_observed);
    let (fresh, _fresh_observed) = within(dials.recv()).await.unwrap();
    accept(&fresh, [0x21; 6], 1476, 1476).await;
    wait_for_state(&transport, ScConnectionState::Connected)
        .await
        .unwrap();
    transport.send_broadcast(&[1, 0]).await.unwrap();
    assert_eq!(
        decode_sc_message(&recv_function(&fresh, 1).await)
            .unwrap()
            .payload
            .as_ref(),
        &[1, 0]
    );
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(
        transport.connection().unwrap().lock().await.hub_vmac,
        Some([0x21; 6])
    );
    transport.stop().await.unwrap();
    assert_retired(&observed, primary_before);
    assert_retired(&first_observed, failover_before);
}

#[tokio::test]
async fn rejection_deadline_already_admitted_public_send_can_finish_but_new_send_cannot() {
    let (client, hub, observed) = GateSocket::pair();
    let mut transport = transport(client);
    let _rx = started(&mut transport, &hub).await;
    observed.hold_application.store(true, Ordering::SeqCst);
    let (sent, ()) = tokio::join!(transport.send_broadcast(&[1, 0, 0x30]), async {
        wait_count(&observed.application_started, 1).await;
        expire(&transport, &hub, &observed).await;
        let before = snapshot(&observed);
        assert!(transport.send_broadcast(&[1, 0, 0x31]).await.is_err());
        assert_eq!(snapshot(&observed), before);
        observed.release.notify_waiters();
    });
    sent.unwrap();
    assert_eq!(
        decode_sc_message(&recv_function(&hub, 1).await)
            .unwrap()
            .payload
            .as_ref(),
        &[1, 0, 0x30]
    );
    assert_eq!(observed.application_dropped.load(Ordering::SeqCst), 1);
    assert_eq!(observed.nak_completed.load(Ordering::SeqCst), 0);
    transport.stop().await.unwrap();
}

#[tokio::test]
async fn rejection_deadline_drop_terminates_pending_nak_without_claiming_socket_close() {
    let (client, hub, observed) = GateSocket::pair();
    let mut transport = transport(client);
    let _rx = started(&mut transport, &hub).await;
    let retained = transport.ws_shared.as_ref().unwrap().lock().await.clone();
    observed.hold_nak.store(true, Ordering::SeqCst);
    hub.send(&rejection_wires()[0]).await.unwrap();
    wait_count(&observed.nak_started, 1).await;
    let task = transport.recv_task.as_ref().unwrap().abort_handle();
    drop(transport);
    within(async {
        while !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert_eq!(observed.nak_dropped.load(Ordering::SeqCst), 1);
    assert!(Arc::strong_count(&retained) >= 1);
    assert_eq!(observed.flushes.load(Ordering::SeqCst), 0);
}
