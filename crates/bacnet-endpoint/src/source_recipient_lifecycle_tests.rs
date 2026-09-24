use super::*;

fn released_projection(db: &mut ObjectDatabase) {
    let object = db.get_mut(&selected()).unwrap();
    assert_eq!(
        object
            .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
            .unwrap(),
        PropertyValue::Boolean(false)
    );
    assert!(object.is_deleteable());
    object
        .configure_audit_reporter_internal(
            AuditLevel::NONE,
            AuditOperationFlags::empty(),
            false,
            Some(vec![]),
            BACnetPriorityFilter::empty(),
            None,
        )
        .unwrap();
}
fn protected(db: &mut ObjectDatabase) {
    assert!(db.remove(&oid(ObjectType::DEVICE, 123)).is_err());
    assert!(db.remove(&selected()).is_err());
    assert!(db
        .add(Box::new(
            AuditReporterObject::new(1, "replacement").unwrap()
        ))
        .is_err());
    let mut extra = crate::DeviceIdentity::new(456, 42)
        .unwrap()
        .build_database()
        .unwrap();
    assert!(db
        .add(
            extra
                .remove(&oid(ObjectType::DEVICE, 456))
                .unwrap()
                .unwrap()
        )
        .is_err());
}

#[tokio::test]
async fn source_recipient_canceled_stop_keeps_sealed_membership_until_retry_barrier() {
    let (mut sink, _records) = network().await;
    let mut session = session(database(false), SessionRole::Both, &sink);
    session.start().await.unwrap();
    let db = session.database.as_ref().unwrap().clone();
    let client = session.cloned_client_handle().unwrap();
    let server = session.cloned_server_handle().unwrap();
    let token = session.shared.token.clone();
    let mut guard = db.write().await;
    protected(&mut guard);
    assert!(timeout(Duration::from_millis(20), session.stop())
        .await
        .is_err());
    assert!(!token.is_open());
    assert!(!server.is_session_alive());
    protected(&mut guard);
    assert!(guard
        .get_mut(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .write_property(
            PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT,
            None,
            PropertyValue::ApplicationData(value(&direct(&sink))),
            None
        )
        .is_err());
    drop(guard);
    timeout(WAIT, session.stop()).await.unwrap().unwrap();
    let mut guard = db.write().await;
    assert!(!guard
        .get(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .property_list()
        .contains(&PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT));
    released_projection(&mut guard);
    assert!(guard.remove(&selected()).unwrap().is_some());
    assert!(guard
        .remove(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .is_some());
    assert!(client
        .read_property(
            sink.local_mac(),
            target(),
            PropertyIdentifier::PRESENT_VALUE,
            None
        )
        .await
        .is_err());
    drop(guard);
    assert!(session.stop().await.is_err());
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_recipient_drop_does_not_pin_membership_for_dormant_external_read() {
    let (mut sink, _records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let db = session.database.as_ref().unwrap().clone();
    let client = session.cloned_client_handle().unwrap();
    let mut guard = db.write().await;
    let peer_mac = sink.local_mac().to_vec();
    let read = client.read_property(&peer_mac, target(), PropertyIdentifier::PRESENT_VALUE, None);
    tokio::pin!(read);
    assert!(timeout(Duration::from_millis(10), &mut read).await.is_err());
    protected(&mut guard);
    let owner = Arc::downgrade(&session.source_recipient.as_ref().unwrap().owner);
    drop(session);
    // An aborted dispatch frame retains membership until the executor drops it.
    timeout(WAIT, async {
        while owner.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // Keep the external future unpolled: it must retain neither a sink nor a lease.
    released_projection(&mut guard);
    assert!(guard.remove(&selected()).unwrap().is_some());
    assert!(guard
        .remove(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .is_some());
    drop(guard);
    assert!(timeout(WAIT, &mut read).await.unwrap().is_err());
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn source_recipient_protected_topology_preserves_pending_failure_then_releases() {
    let (mut peer, mut requests) = network().await;
    let (mut sink, mut records) = network().await;
    let mut session = session(
        failures::failure_database(false),
        SessionRole::ClientOnly,
        &sink,
    );
    session.start().await.unwrap();
    let permits: Vec<_> = (0..64)
        .map(|_| {
            session
                .notifications
                .as_ref()
                .unwrap()
                .try_admit_audit()
                .unwrap()
        })
        .collect();
    failures::complete_read(&session, &peer, &mut requests).await;
    {
        let mut db = session.database.as_ref().unwrap().write().await;
        protected(&mut db);
    }
    drop(permits);
    let (summary, _) = notification(&receive(&mut records).await, false);
    assert_eq!(summary.operation, AuditOperation::AUDITING_FAILURE);
    assert_eq!(summary.current_value, Some(vec![0x21, 1]));
    let db = session.database.as_ref().unwrap().clone();
    session.stop().await.unwrap();
    assert!(db.write().await.remove(&selected()).unwrap().is_some());
    peer.stop().await.unwrap();
    sink.stop().await.unwrap();
}

#[tokio::test]
async fn public_recipient_write_rechecks_sealed_owner_after_database_wait() {
    let (mut sink, mut records) = network().await;
    let mut session = session(database(false), SessionRole::ClientOnly, &sink);
    session.start().await.unwrap();
    let db = session.database.as_ref().unwrap().clone();
    let guard = db.write().await;
    let before = guard
        .get(&oid(ObjectType::DEVICE, 123))
        .unwrap()
        .read_property(PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT, None)
        .unwrap();
    {
        let write = session.write_audit_recipient(Some(direct(&sink)));
        tokio::pin!(write);
        tokio::select! {
            biased;
            result = &mut write => panic!("write escaped held database: {result:?}"),
            () = std::future::ready(()) => {}
        }
        session.source_recipient.as_ref().unwrap().seal();
        drop(guard);
        assert!(write.await.is_err());
    }
    assert_eq!(current(&session).await, before);
    assert!(records.try_recv().is_err());
    session.stop().await.unwrap();
    sink.stop().await.unwrap();
}
