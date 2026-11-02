use super::*;

#[tokio::test]
async fn source_read_delay_preflight_is_atomic_and_adapter_rejects_even_zero() {
    let (mut sink, _records) = network().await;
    for seconds in [0, 3600] {
        let mut db = database(false);
        db.get_mut(&selected())
            .unwrap()
            .audit_reporter_mut()
            .expect("an Audit Reporter")
            .configure(
                AuditLevel::AUDIT_ALL,
                AuditOperationFlags::from_bits(1).unwrap(),
                false,
                None,
                BACnetPriorityFilter::all(),
                Some(bacnet_objects::audit::AuditSendDelay::new(seconds).unwrap()),
            )
            .unwrap();
        let mut session = session(db, SessionRole::ClientOnly, &sink);
        assert!(session.start().await.is_err());
        assert_eq!(
            session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Ready as u8
        );
        let db = Arc::get_mut(session.database.as_mut().unwrap())
            .unwrap()
            .get_mut();
        let object = db.get_mut(&selected()).unwrap();
        assert_eq!(
            object
                .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
        object
            .audit_reporter_mut()
            .expect("an Audit Reporter")
            .configure(
                AuditLevel::AUDIT_ALL,
                AuditOperationFlags::from_bits(1).unwrap(),
                false,
                None,
                BACnetPriorityFilter::all(),
                None,
            )
            .unwrap();
        session.start().await.unwrap();
        {
            let mut db = session.database.as_ref().unwrap().write().await;
            let object = db.get_mut(&selected()).unwrap();
            assert!(
                object
                    .audit_reporter_mut()
                    .expect("an Audit Reporter")
                    .configure(
                        AuditLevel::NONE,
                        AuditOperationFlags::empty(),
                        true,
                        None,
                        BACnetPriorityFilter::empty(),
                        Some(bacnet_objects::audit::AuditSendDelay::new(0).unwrap())
                    )
                    .is_err()
            );
            assert_eq!(
                object
                    .read_property(PropertyIdentifier::AUDIT_LEVEL, None)
                    .unwrap(),
                PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw())
            );
        }
        session.stop().await.unwrap();
    }
    sink.stop().await.unwrap();
}
