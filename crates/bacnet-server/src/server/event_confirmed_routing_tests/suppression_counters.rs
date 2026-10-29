//! A confirmed EventNotification that is never acknowledged moves one
//! undelivered-notification counter once for its recipient (#1142).

use super::*;
use crate::server::EventNotificationCounters;
use crate::server::notification_transactions::canonical_direct_peer;

const TARGET: [u8; 6] = [192, 168, 1, 50, 0xBA, 0xC0];

/// One confirmed recipient on the local network, so its notification is a
/// plain unicast to [`TARGET`].
async fn confirmed_local_harness(retry_timeout_ms: u64) -> Harness {
    Harness::new(
        vec![destination_for(address_recipient(0, &TARGET), true)],
        retry_timeout_ms,
    )
    .await
}

/// The invoke ID of the one confirmed notification sent to [`TARGET`].
fn sent_invoke_id(harness: &Harness) -> u8 {
    let unicasts = harness.unicast_frames();
    assert_eq!(unicasts.len(), 1);
    assert_eq!(unicasts[0].0.as_slice(), &TARGET);
    decode_confirmed(&unicasts[0].1).1.invoke_id
}

/// Wait for the one spawned notification worker to finish on its own.
async fn worker_finished(harness: &Harness) {
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        harness.notification_transactions.join_next(),
    )
    .await
    .expect("the notification worker finishes");
    assert!(matches!(result, Some(Ok(()))), "{result:?}");
}

fn counters(harness: &Harness) -> EventNotificationCounters {
    harness.suppressions.snapshot()
}

#[tokio::test]
async fn acknowledged_confirmed_notification_moves_no_counter() {
    let harness = confirmed_local_harness(60_000).await;
    harness.distribute().await;
    let invoke_id = sent_invoke_id(&harness);
    assert!(self_ack_local(&harness, &TARGET, invoke_id).await);
    worker_finished(&harness).await;
    assert_eq!(counters(&harness), EventNotificationCounters::default());
}

#[tokio::test]
async fn error_and_reject_each_count_one_rejection() {
    for reply in ["error", "reject"] {
        let harness = confirmed_local_harness(60_000).await;
        harness.distribute().await;
        let invoke_id = sent_invoke_id(&harness);
        let apdu = if reply == "error" {
            Apdu::Error(ErrorPdu {
                invoke_id,
                service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                error_class: ErrorClass::SERVICES,
                error_code: ErrorCode::SERVICE_REQUEST_DENIED,
                error_data: Bytes::new(),
            })
        } else {
            Apdu::Reject(RejectPdu {
                invoke_id,
                reject_reason: RejectReason::OTHER,
            })
        };
        assert!(
            harness.dispatch_terminal(&TARGET, None, apdu).await,
            "{reply} completes the transaction"
        );
        worker_finished(&harness).await;
        assert_eq!(
            counters(&harness),
            EventNotificationCounters {
                confirmed_rejected: 1,
                ..Default::default()
            },
            "{reply}"
        );
        assert_eq!(harness.unicast_frames().len(), 1, "{reply} is not retried");
    }
}

#[tokio::test(start_paused = true)]
async fn unanswered_confirmed_notification_counts_once_after_its_last_retry() {
    const TIMEOUT: Duration = Duration::from_millis(100);
    let harness = confirmed_local_harness(TIMEOUT.as_millis() as u64).await;
    harness.distribute().await;
    for retry in 1..=DEFAULT_APDU_RETRIES {
        tokio::time::advance(TIMEOUT).await;
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
        assert_eq!(harness.unicast_frames().len(), usize::from(retry) + 1);
        assert_eq!(
            counters(&harness),
            EventNotificationCounters::default(),
            "retry {retry} is not yet a failure"
        );
    }
    tokio::time::advance(TIMEOUT).await;
    worker_finished(&harness).await;
    assert_eq!(
        counters(&harness),
        EventNotificationCounters {
            confirmed_unanswered: 1,
            ..Default::default()
        }
    );
    assert_eq!(
        harness.unicast_frames().len(),
        usize::from(DEFAULT_APDU_RETRIES) + 1
    );
}

#[tokio::test]
async fn confirmed_notification_without_a_free_invoke_id_counts_once() {
    let harness = confirmed_local_harness(60_000).await;
    // Lease every device-wide invoke ID to another peer.
    let held: Vec<_> = (0..256)
        .map(|_| {
            harness
                .notification_transactions
                .reserve(
                    canonical_direct_peer(&[1]),
                    ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
                )
                .expect("a free invoke ID")
        })
        .collect();
    harness.distribute().await;
    assert!(harness.unicast_frames().is_empty());
    assert_eq!(
        counters(&harness),
        EventNotificationCounters {
            confirmed_no_invoke_id: 1,
            ..Default::default()
        }
    );
    drop(held);
}

#[tokio::test]
async fn confirmed_notification_refused_while_stopping_is_not_counted() {
    let harness = confirmed_local_harness(60_000).await;
    harness.notification_transactions.close();
    harness.distribute().await;
    assert!(harness.unicast_frames().is_empty());
    assert_eq!(counters(&harness), EventNotificationCounters::default());
}

/// A confirmed destination skipped at route resolution counts there, before
/// any invoke ID is reserved, never as a confirmed transaction failure (#1160).
#[tokio::test]
async fn confirmed_route_skips_reserve_no_invoke_id() {
    let unknown = ObjectIdentifier::new(ObjectType::DEVICE, 42).unwrap();
    let harness = Harness::new(
        vec![
            destination_for(address_recipient(0, &[]), true),
            destination_for(address_recipient(1000, &[]), true),
            destination_for(BACnetRecipient::Device(unknown), true),
            destination_for(address_recipient(65535, &TARGET), true),
        ],
        60_000,
    )
    .await;
    harness.distribute().await;
    // The unbound Device recipient is looked for first (#1368): one Who-Is,
    // and no notification goes anywhere.
    let broadcasts = harness.sent.broadcasts();
    assert!(broadcasts.len() == 1 && is_who_is(&broadcasts[0]));
    assert!(harness.unicast_frames().is_empty());
    assert_eq!(harness.notification_transactions.active_count(), 0);
    // Its device stays silent, so its notification is skipped too.
    harness
        .device_bindings
        .write()
        .await
        .probes
        .run_out_for_test();
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    assert_eq!(harness.sent.broadcasts().len(), 1);
    assert_eq!(harness.notification_transactions.active_count(), 0);
    assert_eq!(
        counters(&harness),
        EventNotificationCounters {
            device_recipient_unbound: 1,
            recipient_unroutable: 1,
            confirmed_broadcast_recipient: 2,
            ..Default::default()
        }
    );
}
