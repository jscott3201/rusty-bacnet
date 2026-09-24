use super::batching::delayed;
use super::*;
use bacnet_objects::audit::AuditReporterObject;

#[tokio::test(start_paused = true)]
async fn delayed_target_audit_late_older_drop_replaces_earliest_and_counts_every_local_loss() {
    let mut r = delayed(10);
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::WRITE);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    r.set_auditable_operations(flags).unwrap();
    let mut f = server(r).await;
    let held = (0..64)
        .map(|_| {
            f.server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect::<Vec<_>>();
    for _ in 0..65 {
        assert!(matches!(
            write_value(&f.server, None).await,
            Apdu::SimpleAck(_)
        ));
    }
    settle().await;
    assert_eq!(
        f.server
            .target_audit
            .as_ref()
            .unwrap()
            .batches
            .resources()
            .0,
        64
    );
    assert!(f.transport.sent.lock().unwrap().is_empty());
    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(
        f.server
            .target_audit
            .as_ref()
            .unwrap()
            .batches
            .resources()
            .0,
        0
    );
    assert_eq!(f.server.notification_transactions.audit_resources().1, 65);
    drop(held);
    settle().await;
    let batches = notifications(&f.transport.sent);
    assert_eq!(batches.len(), 1);
    let summary = &batches[0].notifications[0];
    assert_eq!(summary.operation, AuditOperation::AUDITING_FAILURE);
    assert_eq!(
        summary.target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(0))
    );
    assert_eq!(summary.current_value, Some(vec![0x21, 65]));
    assert_eq!(
        f.writes.load(Ordering::Acquire),
        65,
        "queue pressure cannot reject ordinary mutation"
    );
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn delayed_target_audit_apdu_split_preserves_order_and_complete_limit() {
    let mut f = server(delayed(1)).await;
    for _ in 0..40 {
        write_value(&f.server, None).await;
    }
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    let batches = notifications(&f.transport.sent);
    assert!(batches.len() > 1);
    let records = batches
        .into_iter()
        .flat_map(|batch| batch.notifications)
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 40);
    for (index, record) in records.iter().enumerate() {
        assert_eq!(
            record.target_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(index as u16))
        );
    }
    for frame in f.transport.sent.lock().unwrap().iter() {
        assert!(
            decode_npdu(frame.clone()).unwrap().payload.len()
                <= f.server.config.max_apdu_length as usize
        );
    }
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn delayed_target_audit_last_reporter_context_failure_rolls_back_whole_device_then_recovers()
{
    let mut first = AuditReporterObject::new(1, "inactive first").unwrap();
    first.set_monitored_objects(Some(vec![])).unwrap();
    let mut last = AuditReporterObject::new(2, "active last").unwrap();
    last.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::WRITE);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    last.set_auditable_operations(flags).unwrap();
    last.set_maximum_send_delay(Some(
        bacnet_objects::audit::AuditSendDelay::new(3600).unwrap(),
    ))
    .unwrap();
    let mut f = try_servers(
        vec![first, last],
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
        vec![
            DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap(),
            DeviceBinding::local(oid(ObjectType::DEVICE, 21), NEW_LOGGER).unwrap(),
        ],
    )
    .await
    .unwrap();
    let statuses = f
        .server
        .target_audit
        .as_ref()
        .unwrap()
        .association
        .reporters()
        .iter()
        .map(|(_, status)| Arc::clone(status))
        .collect::<Vec<_>>();
    for index in 0..8 {
        write_value(&f.server, None).await;
        if index < 7 {
            f.server
                .database()
                .write()
                .await
                .get_mut(&oid(ObjectType::AUDIT_REPORTER, 2))
                .unwrap()
                .write_property(
                    PropertyIdentifier::DESCRIPTION,
                    None,
                    PropertyValue::CharacterString(format!("epoch{index}")),
                    None,
                )
                .unwrap();
            settle().await;
        }
    }
    let resources = statuses
        .iter()
        .map(|status| {
            f.server
                .notification_transactions
                .audit_failure_queue(status)
                .unwrap()
                .context_resources()
        })
        .collect::<Vec<_>>();
    assert_eq!(resources[1].0, 8);
    let epochs = statuses
        .iter()
        .map(|status| status.configuration_epoch())
        .collect::<Vec<_>>();
    let before = f
        .server
        .database()
        .read()
        .await
        .get(&oid(ObjectType::DEVICE, 10))
        .unwrap()
        .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
        .unwrap();
    let mut raw = BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(
        &mut raw,
        &BACnetRecipient::Device(oid(ObjectType::DEVICE, 21)),
    );
    let count = f.transport.sent.lock().unwrap().len();
    let denied = dispatch(
        &f.server,
        ConfirmedServiceChoice::WRITE_PROPERTY,
        wp(
            oid(ObjectType::DEVICE, 10),
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            raw.to_vec(),
            None,
        ),
    )
    .await;
    assert!(
        matches!(denied,Apdu::Error(ref error) if error.error_class==ErrorClass::SERVICES && error.error_code==ErrorCode::SERVICE_REQUEST_DENIED)
    );
    assert_eq!(
        statuses
            .iter()
            .map(|status| status.configuration_epoch())
            .collect::<Vec<_>>(),
        epochs
    );
    assert_eq!(
        statuses
            .iter()
            .map(|status| f
                .server
                .notification_transactions
                .audit_failure_queue(status)
                .unwrap()
                .context_resources())
            .collect::<Vec<_>>(),
        resources
    );
    assert_eq!(
        f.server
            .database()
            .read()
            .await
            .get(&oid(ObjectType::DEVICE, 10))
            .unwrap()
            .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
            .unwrap(),
        before
    );
    assert_eq!(f.transport.sent.lock().unwrap().len(), count);
    // A command needs no new context and frees the retained history without changing its routing.
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::AUDIT_REPORTER, 2),
                PropertyIdentifier::SEND_NOW,
                vec![0x11],
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::DEVICE, 10),
                PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                raw.to_vec(),
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    settle().await;
    let pair = notifications(&f.transport.sent);
    let last = &pair[pair.len() - 2..];
    assert_eq!(last[0].notifications, last[1].notifications);
    assert_eq!(
        last[0].notifications[0].target_timestamp,
        Some(BACnetTimeStamp::SequenceNumber(16)),
        "denied whole-Device prepare consumes no sequence"
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn delayed_target_audit_oversize_record_is_known_local_loss_and_does_not_block_small_suffix()
{
    let mut f = try_servers_config(
        vec![delayed(1)],
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
        vec![DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap()],
        true,
        50,
        CaptureTransport::default(),
    )
    .await
    .unwrap();
    f.server
        .db
        .write()
        .await
        .get_mut(&oid(ObjectType::BINARY_VALUE, 1))
        .unwrap()
        .write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("a".repeat(27)),
            None,
        )
        .unwrap();
    let mut value = BytesMut::new();
    bacnet_encoding::primitives::encode_app_character_string(&mut value, &"b".repeat(27)).unwrap();
    assert!(matches!(
        dispatch(
            &f.server,
            ConfirmedServiceChoice::WRITE_PROPERTY,
            wp(
                oid(ObjectType::BINARY_VALUE, 1),
                PropertyIdentifier::DESCRIPTION,
                value.to_vec(),
                None
            )
        )
        .await,
        Apdu::SimpleAck(_)
    ));
    write_value(&f.server, None).await;
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    let records = notifications(&f.transport.sent)
        .into_iter()
        .flat_map(|r| r.notifications)
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0]
            .target_property
            .as_ref()
            .unwrap()
            .property_identifier,
        PropertyIdentifier::PRESENT_VALUE
    );
    let status = &f
        .server
        .target_audit
        .as_ref()
        .unwrap()
        .association
        .reporters()[0]
        .1;
    assert_eq!(
        f.server
            .notification_transactions
            .audit_failure_queue(status)
            .unwrap()
            .context_resources()
            .2,
        1,
        "filter-off loss stays an internal diagnostic"
    );
    f.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn delayed_target_audit_full_ordinary_queue_keeps_mandatory_changes_immediate() {
    let mut f = server(delayed(3600)).await;
    for _ in 0..64 {
        write_value(&f.server, None).await;
    }
    assert_eq!(
        f.server
            .target_audit
            .as_ref()
            .unwrap()
            .batches
            .resources()
            .0,
        64
    );
    let target = oid(ObjectType::AUDIT_REPORTER, 1);
    f.server
        .write_local(
            &target,
            PropertyIdentifier::MAXIMUM_SEND_DELAY,
            None,
            PropertyValue::Unsigned(3599),
            None,
        )
        .await
        .unwrap();
    settle().await;
    let records = notifications(&f.transport.sent);
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].notifications[0]
            .target_property
            .as_ref()
            .unwrap()
            .property_identifier,
        PropertyIdentifier::MAXIMUM_SEND_DELAY
    );
    assert_eq!(
        f.server
            .target_audit
            .as_ref()
            .unwrap()
            .batches
            .resources()
            .0,
        64
    );
    super::batching::command(&f, true).await;
    settle().await;
    assert_eq!(
        f.server
            .target_audit
            .as_ref()
            .unwrap()
            .batches
            .resources()
            .0,
        0
    );
    f.server.stop().await.unwrap();
}
