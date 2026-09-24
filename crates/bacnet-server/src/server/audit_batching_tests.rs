//! Executed target delayed-notification controls and wire batches.
use super::*;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_services::audit::AuditNotificationRequest;

#[tokio::test]
async fn delayed_target_audit_wire_maximum_send_delay_is_executed() {
    let mut f = server(delayed(0)).await;
    let response = dispatch(
        &f.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::AUDIT_REPORTER, 1),
            PropertyIdentifier::MAXIMUM_SEND_DELAY,
            vec![0x21, 1],
            None,
        ),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    f.server.stop().await.unwrap();
}

#[tokio::test]
async fn delayed_target_audit_wire_send_now_is_executed() {
    let mut f = server(delayed(0)).await;
    let response = dispatch(
        &f.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::AUDIT_REPORTER, 1),
            PropertyIdentifier::SEND_NOW,
            vec![0x11],
            None,
        ),
    )
    .await;
    assert!(matches!(response, Apdu::SimpleAck(_)), "{response:?}");
    f.server.stop().await.unwrap();
}

pub(super) fn delayed(seconds: u32) -> AuditReporterObject {
    let mut value = reporter();
    value
        .set_maximum_send_delay(Some(
            bacnet_objects::audit::AuditSendDelay::new(seconds).unwrap(),
        ))
        .unwrap();
    value
}
async fn delay_write(f: &Fixture, seconds: u64) {
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_unsigned(&mut value, seconds);
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::AUDIT_REPORTER, 1),
                PropertyIdentifier::MAXIMUM_SEND_DELAY,
                value.to_vec(),
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
}
pub(super) async fn command(f: &Fixture, value: bool) {
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::AUDIT_REPORTER, 1),
                PropertyIdentifier::SEND_NOW,
                vec![if value { 0x11 } else { 0x10 }],
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
}
async fn readback(f: &Fixture) -> bool {
    f.server
        .database()
        .read()
        .await
        .get(&oid(ObjectType::AUDIT_REPORTER, 1))
        .unwrap()
        .read_property(PropertyIdentifier::SEND_NOW, None)
        .unwrap()
        == PropertyValue::Boolean(true)
}
fn ordinary(f: &Fixture) -> Vec<AuditNotificationRequest> {
    notifications(&f.transport.sent)
        .into_iter()
        .filter(|request| {
            request
                .notifications
                .iter()
                .all(|n| n.target_object == Some(oid(ObjectType::BINARY_VALUE, 1)))
        })
        .collect()
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_first_deadline_batches_preserve_timestamps_and_increase_never_postpones(
) {
    let mut f = server(delayed(10)).await;
    write_value(&f.server, None).await;
    tokio::time::advance(Duration::from_secs(9)).await;
    write_value(&f.server, None).await;
    delay_write(&f, 30).await;
    settle().await;
    assert!(ordinary(&f).is_empty());
    tokio::time::advance(Duration::from_millis(999)).await;
    settle().await;
    assert!(ordinary(&f).is_empty());
    tokio::time::advance(Duration::from_millis(1)).await;
    settle().await;
    let records = ordinary(&f);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].notifications.len(), 2);
    assert_ne!(
        records[0].notifications[0].target_timestamp,
        records[0].notifications[1].target_timestamp
    );
    f.server.stop().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_decrease_accelerates_and_new_context_does_not_mix() {
    let mut f = server(delayed(30)).await;
    write_value(&f.server, None).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    delay_write(&f, 2).await;
    write_value(&f.server, None).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    let records = ordinary(&f);
    assert_eq!(
        records.len(),
        2,
        "configuration generations must not share one APDU"
    );
    assert!(records.iter().all(|batch| batch.notifications.len() == 1));
    f.server.stop().await.unwrap();
}
#[tokio::test(start_paused = true)]
async fn delayed_target_audit_command_fences_false_continuation_and_empty_repeated_true() {
    let mut f = server(delayed(30)).await;
    f.transport.block.store(true, Ordering::Release);
    write_value(&f.server, None).await;
    command(&f, true).await;
    settle().await;
    assert!(readback(&f).await);
    command(&f, false).await;
    assert!(!readback(&f).await);
    write_value(&f.server, None).await;
    command(&f, true).await;
    assert!(readback(&f).await);
    f.transport.block.store(false, Ordering::Release);
    f.transport.unblock.notify_waiters();
    settle().await;
    assert!(!readback(&f).await);
    assert_eq!(
        ordinary(&f)
            .iter()
            .map(|r| r.notifications.len())
            .sum::<usize>(),
        2
    );
    command(&f, true).await;
    command(&f, true).await;
    settle().await;
    assert!(!readback(&f).await);
    let commands = notifications(&f.transport.sent)
        .into_iter()
        .flat_map(|r| r.notifications)
        .filter(|n| {
            n.target_property
                .as_ref()
                .is_some_and(|p| p.property_identifier == PropertyIdentifier::SEND_NOW)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        commands.len(),
        5,
        "one external record per command, no internal reset WRITE"
    );
    assert!(commands.iter().all(|n| n.invoke_id.is_some()
        && n.source_device
            == bacnet_types::constructed::BACnetRecipient::Address(
                bacnet_types::constructed::BACnetAddress {
                    network_number: 0,
                    mac_address: SOURCE.to_vec().into()
                }
            )));
    f.server.stop().await.unwrap();
}
