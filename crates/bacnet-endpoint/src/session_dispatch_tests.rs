use super::*;
use bacnet_encoding::apdu::{encode_apdu, SimpleAck};
use bacnet_endpoint_core::coordinator::CanonicalPeer;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::BytesMut;

fn envelope(invoke_id: u8) -> ReceivedApdu {
    let mut bytes = BytesMut::new();
    encode_apdu(
        &mut bytes,
        &Apdu::SimpleAck(SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        }),
    )
    .unwrap();
    ReceivedApdu {
        apdu: bytes.freeze(),
        source_mac: [2].into_iter().collect(),
        source_network: None,
        ingress_network: None,
        link_layer_group: false,
        is_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

fn fixture() -> (
    DispatchParts,
    mpsc::Sender<ReceivedApdu>,
    mpsc::Sender<ReceivedApdu>,
    mpsc::Sender<PolicyOutcome>,
) {
    let (inbound_tx, inbound) = mpsc::channel(8);
    let (terminal_tx, terminal) = mpsc::channel(8);
    let (policy_tx, policy) = mpsc::channel(8);
    let coordinator = Arc::new(OutboundTransactionCoordinator::new());
    let notifications = NotificationTransactions::with_coordinator(Arc::clone(&coordinator));
    let shared = Arc::new(SessionShared {
        token: crate::roles::SessionToken::new(Arc::clone(&coordinator)),
        counters: Mutex::new(PolicyCounters {
            ingress_policy: 0,
            no_server_role: 0,
            no_client_role: 0,
            unclaimed_terminal: 0,
            responder_declined: 0,
        }),
    });
    (
        DispatchParts {
            inbound,
            terminal,
            policy,
            requester: None,
            responder: None,
            notifications: Some(notifications),
            coordinator,
            shared,
        },
        inbound_tx,
        terminal_tx,
        policy_tx,
    )
}

// Current-thread execution: after the signal, the worker returns without another
// await, so its completed join is ready before this test resumes. No sleep/race.
async fn ready_worker(owner: &NotificationTransactions) {
    let (done, finished) = oneshot::channel();
    owner.spawn(async move {
        let _ = done.send(());
    });
    finished.await.unwrap();
}

#[tokio::test]
async fn replenished_worker_completions_cannot_starve_valid_terminal_or_ingress() {
    let (mut parts, inbound, terminal, policy) = fixture();
    let owner = Arc::clone(parts.notifications.as_ref().unwrap());
    let (operation, mut acknowledged) = owner
        .reserve(
            CanonicalPeer::direct(&[2]),
            ConfirmedServiceChoice::CONFIRMED_AUDIT_NOTIFICATION,
        )
        .unwrap();
    terminal.try_send(envelope(operation.invoke_id())).unwrap();
    inbound.try_send(envelope(99)).unwrap();
    policy
        .try_send(PolicyOutcome {
            reason: PolicyReason::MalformedApdu,
            received: envelope(99),
        })
        .unwrap();
    let mut next = 0;
    let mut seen = [false; 4];
    for _ in 0..4 {
        ready_worker(&owner).await; // completion is ready on every selection
        match next_event(&mut parts, &mut next).await {
            DispatchEvent::Worker(_) => seen[0] = true,
            DispatchEvent::Inbound(received) => {
                seen[1] = true;
                handle_inbound(&mut parts, received).await;
            }
            DispatchEvent::Terminal(received) => {
                seen[2] = true;
                handle_terminal(&mut parts, received).await;
            }
            DispatchEvent::Policy(outcome) => {
                seen[3] = true;
                handle_ingress_policy(&parts.shared, outcome).await;
            }
            DispatchEvent::Closed => panic!("open streams"),
        }
    }
    assert_eq!(
        seen, [true; 4],
        "each ready stream must progress within four selections"
    );
    assert!(
        acknowledged.try_recv().is_ok(),
        "valid terminal must release its exact lease"
    );
    assert_eq!(parts.coordinator.active_count().unwrap(), 0);
    let counters = parts.shared.counters.lock().await;
    assert_eq!(counters.no_server_role, 1);
    assert_eq!(counters.ingress_policy, 1);
    drop(counters);
    drop(operation);
    owner.close();
    while let Some(result) = owner.join_next().await {
        NotificationTransactions::observe(Some(result));
    }
}

#[tokio::test]
async fn replenished_ingress_cannot_starve_ready_worker_reaping() {
    let (mut parts, inbound, terminal, policy) = fixture();
    let owner = Arc::clone(parts.notifications.as_ref().unwrap());
    let mut next = 1; // start at ingress, not the completion slot
    for _ in 0..16 {
        ready_worker(&owner).await;
        let mut reaped = false;
        for _ in 0..4 {
            if parts.inbound.is_empty() {
                inbound.try_send(envelope(99)).unwrap();
            }
            if parts.terminal.is_empty() {
                terminal.try_send(envelope(99)).unwrap();
            }
            if parts.policy.is_empty() {
                policy
                    .try_send(PolicyOutcome {
                        reason: PolicyReason::MalformedApdu,
                        received: envelope(99),
                    })
                    .unwrap();
            }
            if matches!(
                next_event(&mut parts, &mut next).await,
                DispatchEvent::Worker(_)
            ) {
                reaped = true;
                break;
            }
        }
        assert!(
            reaped,
            "sustained ready ingress must not delay a join beyond four selections"
        );
    }
    owner.close();
    assert!(
        owner.join_next().await.is_none(),
        "all completed joins were reaped while running"
    );
}

#[tokio::test]
async fn cancellation_precedes_all_ready_dispatch_streams() {
    let (parts, inbound, terminal, policy) = fixture();
    let owner = Arc::clone(parts.notifications.as_ref().unwrap());
    let shared = Arc::clone(&parts.shared);
    ready_worker(&owner).await;
    inbound.try_send(envelope(99)).unwrap();
    terminal.try_send(envelope(99)).unwrap();
    policy
        .try_send(PolicyOutcome {
            reason: PolicyReason::MalformedApdu,
            received: envelope(99),
        })
        .unwrap();
    let (cancel, cancelled) = oneshot::channel();
    cancel.send(()).unwrap();
    assert!(matches!(
        dispatch_loop(parts, cancelled).await,
        SessionExit::Cancelled
    ));
    let counters = shared.counters.lock().await;
    assert_eq!(counters.no_server_role, 0);
    assert_eq!(counters.ingress_policy, 0);
    assert_eq!(counters.unclaimed_terminal, 0);
    owner.close();
    assert!(owner.join_next().await.is_some());
    assert!(owner.join_next().await.is_none());
}

#[tokio::test]
async fn closed_streams_drain_buffered_events_before_exit() {
    let (mut parts, inbound, terminal, policy) = fixture();
    parts.notifications.as_ref().unwrap().close();
    inbound.try_send(envelope(99)).unwrap();
    terminal.try_send(envelope(99)).unwrap();
    policy
        .try_send(PolicyOutcome {
            reason: PolicyReason::MalformedApdu,
            received: envelope(99),
        })
        .unwrap();
    drop((inbound, terminal, policy));
    let mut next = 0;
    assert!(matches!(
        next_event(&mut parts, &mut next).await,
        DispatchEvent::Inbound(_)
    ));
    assert!(matches!(
        next_event(&mut parts, &mut next).await,
        DispatchEvent::Terminal(_)
    ));
    assert!(matches!(
        next_event(&mut parts, &mut next).await,
        DispatchEvent::Policy(_)
    ));
    assert!(matches!(
        next_event(&mut parts, &mut next).await,
        DispatchEvent::Closed
    ));
}
