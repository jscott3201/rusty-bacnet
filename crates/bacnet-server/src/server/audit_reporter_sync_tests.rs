use super::*;

#[tokio::test]
async fn target_reporter_off_runtime_silent_change_commits_without_worker() {
    let mut target = reporter();
    target.set_audit_level(AuditLevel::NONE).unwrap();
    target
        .set_auditable_operations(AuditOperationFlags::empty())
        .unwrap();
    let mut fixture = server(target).await;
    let database = Arc::clone(fixture.server.database());
    let reporter_id = oid(ObjectType::AUDIT_REPORTER, 1);
    let outcome = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let mut database = database.blocking_write();
        let reporter = database.get_mut(&reporter_id).unwrap();
        let mut operations = AuditOperationFlags::empty();
        operations.insert(AuditOperation::WRITE);
        reporter.configure_audit_reporter_internal(
            AuditLevel::NONE,
            operations,
            false,
            None,
            BACnetPriorityFilter::all(),
        )?;
        reporter.write_property(
            PropertyIdentifier::DESCRIPTION,
            None,
            PropertyValue::CharacterString("synchronous silent change".into()),
            None,
        )?;
        assert_eq!(
            reporter.read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None)?,
            PropertyValue::BitString {
                unused_bits: operations.to_bacnet().0,
                data: operations.to_bacnet().1
            },
        );
        assert_eq!(
            reporter.read_property(PropertyIdentifier::DESCRIPTION, None)?,
            PropertyValue::CharacterString("synchronous silent change".into()),
        );
        // A notification-producing change still rejects before committing when
        // no runtime is available; the silent path must not weaken this guard.
        assert!(reporter
            .configure_audit_reporter_internal(
                AuditLevel::AUDIT_ALL,
                operations,
                false,
                None,
                BACnetPriorityFilter::all(),
            )
            .is_err());
        assert_eq!(
            reporter.read_property(PropertyIdentifier::AUDIT_LEVEL, None)?,
            PropertyValue::Enumerated(AuditLevel::NONE.to_raw())
        );
        Ok::<_, Error>(())
    })
    .join()
    .unwrap();
    assert!(outcome.is_ok(), "silent active change failed: {outcome:?}");
    settle().await;
    assert!(fixture.transport.sent.lock().unwrap().is_empty());
    assert!(fixture.server.notification_transactions.workers_empty());
    assert_eq!(
        fixture.server.notification_transactions.audit_resources(),
        (false, 0, 64)
    );
    let owner = fixture
        .server
        .notification_transactions
        .audit_owner_lease()
        .unwrap();
    fixture
        .server
        .notification_transactions
        .seal_audit_owner(&owner);
    for closed in [false, true] {
        if closed {
            fixture.server.notification_transactions.close();
        }
        let database = Arc::clone(fixture.server.database());
        std::thread::spawn(move || {
            let mut database = database.blocking_write();
            let reporter = database.get_mut(&reporter_id).unwrap();
            assert!(reporter
                .write_property(
                    PropertyIdentifier::DESCRIPTION,
                    None,
                    PropertyValue::CharacterString("must not commit".into()),
                    None,
                )
                .is_err());
            assert_eq!(
                reporter
                    .read_property(PropertyIdentifier::DESCRIPTION, None)
                    .unwrap(),
                PropertyValue::CharacterString("synchronous silent change".into())
            );
        })
        .join()
        .unwrap();
    }
    drop(owner);
    fixture.server.stop().await.unwrap();
}
