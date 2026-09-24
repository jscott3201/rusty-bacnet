use super::batching::delayed;
use super::*;

#[tokio::test(start_paused = true)]
async fn delayed_target_audit_historical_aba_summaries_use_captured_routes_without_current_health_authority(
) {
    let mut reporter = delayed(10);
    let mut flags = AuditOperationFlags::empty();
    flags.insert(AuditOperation::WRITE);
    flags.insert(AuditOperation::AUDITING_FAILURE);
    reporter.set_auditable_operations(flags).unwrap();
    let mut f = try_server(
        reporter,
        &[10],
        Some(BACnetRecipient::Device(oid(ObjectType::DEVICE, 20))),
        vec![
            DeviceBinding::local(oid(ObjectType::DEVICE, 20), LOGGER).unwrap(),
            DeviceBinding::local(oid(ObjectType::DEVICE, 21), NEW_LOGGER).unwrap(),
        ],
    )
    .await
    .unwrap();
    write_value(&f.server, None).await;
    for instance in [21, 20] {
        let mut value = BytesMut::new();
        bacnet_encoding::constructed::encode_recipient(
            &mut value,
            &BACnetRecipient::Device(oid(ObjectType::DEVICE, instance)),
        );
        assert!(matches!(
            dispatch(
                &f.server,
                ConfirmedServiceChoice::WRITE_PROPERTY,
                wp(
                    oid(ObjectType::DEVICE, 10),
                    PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
                    value.to_vec(),
                    None
                )
            )
            .await,
            Apdu::SimpleAck(_)
        ));
        settle().await;
        write_value(&f.server, None).await;
    }
    assert_eq!(
        notifications(&f.transport.sent).len(),
        4,
        "only mandatory old/new pairs send immediately"
    );
    f.transport.sent.lock().unwrap().clear();
    f.transport.destinations.lock().unwrap().clear();
    let held = (0..64)
        .map(|_| {
            f.server
                .notification_transactions
                .try_admit_audit()
                .unwrap()
        })
        .collect::<Vec<_>>();
    tokio::time::advance(Duration::from_secs(10)).await;
    settle().await;
    assert_eq!(f.server.notification_transactions.audit_resources().1, 3);
    f.transport.block.store(true, Ordering::Release);
    drop(held);
    settle().await;
    assert_eq!(notifications(&f.transport.sent).len(), 1);
    assert_eq!(health(&f.server).await, Reliability::COMMUNICATION_FAILURE);
    f.transport.block.store(false, Ordering::Release);
    f.transport.unblock.notify_waiters();
    settle().await;
    let summaries = notifications(&f.transport.sent)
        .into_iter()
        .flat_map(|r| r.notifications)
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 3);
    assert_eq!(
        *f.transport.destinations.lock().unwrap(),
        vec![LOGGER.to_vec(), NEW_LOGGER.to_vec(), LOGGER.to_vec()]
    );
    for (summary, sequence) in summaries.iter().zip([0, 2, 4]) {
        assert_eq!(summary.operation, AuditOperation::AUDITING_FAILURE);
        assert_eq!(summary.current_value, Some(vec![0x21, 1]));
        assert_eq!(
            summary.target_timestamp,
            Some(BACnetTimeStamp::SequenceNumber(sequence))
        );
    }
    assert_eq!(health(&f.server).await, Reliability::NO_FAULT_DETECTED);
    f.server.stop().await.unwrap();
}
#[tokio::test]
async fn delayed_target_audit_active_presence_and_off_runtime_silent_commands_preserve_owner() {
    let mut reporter = delayed(0);
    reporter.set_audit_level(AuditLevel::NONE).unwrap();
    let mut f = server(reporter).await;
    let database = Arc::clone(&f.server.db);
    std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let mut db = database.blocking_write();
        let object = db.get_mut(&oid(ObjectType::AUDIT_REPORTER, 1)).unwrap();
        for value in [true, true, false] {
            object
                .write_property(
                    PropertyIdentifier::SEND_NOW,
                    None,
                    PropertyValue::Boolean(value),
                    None,
                )
                .unwrap();
        }
        object
            .write_property(
                PropertyIdentifier::MAXIMUM_SEND_DELAY,
                None,
                PropertyValue::Unsigned(1),
                None,
            )
            .unwrap();
        assert!(object
            .configure_audit_reporter_internal(
                AuditLevel::NONE,
                AuditOperationFlags::empty(),
                false,
                None,
                BACnetPriorityFilter::all(),
                None
            )
            .is_err());
        assert_eq!(
            object
                .read_property(PropertyIdentifier::MAXIMUM_SEND_DELAY, None)
                .unwrap(),
            PropertyValue::Unsigned(1)
        );
        assert_eq!(
            object
                .read_property(PropertyIdentifier::SEND_NOW, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
    })
    .join()
    .unwrap();
    settle().await;
    assert!(f.transport.sent.lock().unwrap().is_empty());
    assert!(f.server.notification_transactions.delivery_workers_idle());
    f.server.stop().await.unwrap();
}
