use super::*;
use bacnet_objects::traits::BACnetObject;
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicUsize};

struct ReadProbe {
    inner: LifeSafetyPointObject,
    fail: Arc<AtomicBool>,
    selected_reads: Arc<AtomicUsize>,
}
impl BACnetObject for ReadProbe {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        self.inner.property_list()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if matches!(
            property,
            PropertyIdentifier::SILENCED | PropertyIdentifier::OPERATION_EXPECTED
        ) {
            self.selected_reads.fetch_add(1, Ordering::Relaxed);
        }
        if property == PropertyIdentifier::OPERATION_EXPECTED && self.fail.load(Ordering::Acquire) {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::READ_ACCESS_DENIED.to_raw() as u32,
            });
        }
        self.inner.read_property(property, index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property(property, index, value, priority)
    }
    fn supports_cov(&self) -> bool {
        true
    }
    fn supports_cov_property(&self, property: PropertyIdentifier) -> bool {
        self.inner.supports_cov_property(property)
    }
}

async fn fire(fixture: &DispatchFixture, initial: bool, snapshots: &[CovSubscriptionSnapshot]) {
    if initial {
        BACnetServer::<RecordingTransport>::fire_initial_cov_notification_multiple(
            &fixture.db,
            &fixture.network,
            &fixture.cov_table,
            &fixture.cov_in_flight,
            &fixture.transactions,
            &fixture.comm_state,
            &fixture.config,
            snapshots,
        )
        .await;
    } else {
        BACnetServer::<RecordingTransport>::fire_life_safety_cov_notifications(
            &fixture.db,
            &fixture.network,
            &fixture.cov_table,
            &fixture.cov_in_flight,
            &fixture.transactions,
            &fixture.comm_state,
            &fixture.config,
            &point_oid(),
            &[PropertyIdentifier::STATUS_FLAGS],
        )
        .await;
    }
}

async fn mixed_case(initial: bool, confirmed: bool, retain_value: bool) {
    let fail = Arc::new(AtomicBool::new(false));
    let reads = Arc::new(AtomicUsize::new(0));
    let mut db = clocked_test_database();
    db.add(Box::new(ReadProbe {
        inner: LifeSafetyPointObject::new(1, "one").unwrap(),
        fail: fail.clone(),
        selected_reads: reads.clone(),
    }))
    .unwrap();
    // Initial Multiple can contain multiple objects; fanout selects one changed object.
    let second_oid = if initial {
        db.add(Box::new(ReadProbe {
            inner: LifeSafetyPointObject::new(2, "two").unwrap(),
            fail: fail.clone(),
            selected_reads: reads.clone(),
        }))
        .unwrap();
        ObjectIdentifier::new(ObjectType::LIFE_SAFETY_POINT, 2).unwrap()
    } else {
        point_oid()
    };
    let mut a = subscription(
        Some(PropertyIdentifier::SILENCED),
        CovNotificationKind::Multiple,
        73,
    );
    a.issue_confirmed_notifications = confirmed;
    a.expires_at = Some(Instant::now() + Duration::from_secs(100));
    let mut b = a.clone();
    b.monitored_object_identifier = second_oid;
    b.monitored_property = Some(PropertyIdentifier::OPERATION_EXPECTED);
    b.timestamped = true;
    let a_key = a.key().unwrap();
    let b_key = b.key().unwrap();
    let fixture = DispatchFixture::new(db, [a, b]).await;
    // Both selected reads succeed initially. The later failure is not a fixture
    // that was already invalid when accepted by the table.
    assert!(fixture
        .db
        .read()
        .await
        .get(&second_oid)
        .unwrap()
        .read_property(PropertyIdentifier::OPERATION_EXPECTED, None)
        .is_ok());
    let snapshots = {
        let table = fixture.cov_table.read().await;
        // Put the soon-stale timestamped candidate first, testing representative selection.
        vec![
            table.get_subscription(&b_key).unwrap().clone(),
            table.get_subscription(&a_key).unwrap().clone(),
        ]
    };
    fail.store(!retain_value, Ordering::Release);
    reads.store(0, Ordering::Relaxed);
    let db_guard = fixture.db.write().await;
    let mut work = Box::pin(fire(&fixture, initial, &snapshots));
    assert!(futures_util::poll!(work.as_mut()).is_pending());
    let mut table_guard = fixture.cov_table.write().await;
    drop(db_guard);
    assert!(futures_util::poll!(work.as_mut()).is_pending());
    assert!(
        reads.load(Ordering::Relaxed) >= 2,
        "the gate is after both property reads"
    );
    assert!(table_guard.unsubscribe(if retain_value { &b_key } else { &a_key }));
    drop(table_guard);
    work.await;
    if retain_value {
        tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.sent.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let apdus = fixture.take_apdus();
        assert_eq!(apdus.len(), 1);
        let payload = match &apdus[0] {
            Apdu::UnconfirmedRequest(request) => &request.service_request,
            Apdu::ConfirmedRequest(request) => {
                assert!(fixture.transactions.admit_terminal(
                    &snapshots[1].subscriber_mac,
                    None,
                    &Apdu::SimpleAck(SimpleAck {
                        invoke_id: request.invoke_id,
                        service_choice: request.service_choice
                    })
                ));
                &request.service_request
            }
            other => panic!("unexpected {other:?}"),
        };
        let notification = COVNotificationMultipleRequest::decode(payload).unwrap();
        assert!((99..=100).contains(&notification.time_remaining));
        assert!(
            notification.timestamp.is_none(),
            "stale timestamped candidate must not control the header"
        );
        assert_eq!(notification.list_of_cov_notifications.len(), 1);
        let item = &notification.list_of_cov_notifications[0];
        assert_eq!(item.monitored_object_identifier, point_oid());
        assert_eq!(
            item.list_of_values
                .iter()
                .map(|value| value.property_identifier)
                .collect::<Vec<_>>(),
            vec![
                PropertyIdentifier::SILENCED,
                PropertyIdentifier::STATUS_FLAGS
            ]
        );
        assert!(item
            .list_of_values
            .iter()
            .all(|value| value.time_of_change.is_none()));
    } else {
        assert!(fixture.take_apdus().is_empty(), "a live failed read cannot authorize the stale successful value or a companion-only payload");
        assert_eq!(fixture.transactions.active_count(), 0);
        let table = fixture.cov_table.read().await;
        assert_eq!(table.counters().snapshot().notifications_sent, 0);
        assert_eq!(table.counters().snapshot().notification_bytes_sent, 0);
        assert!(table
            .get_subscription(&b_key)
            .unwrap()
            .last_notified_value
            .is_none());
    }
    fixture.transactions.close();
    while fixture.transactions.join_next().await.is_some() {}
    assert_eq!(fixture.cov_in_flight.available_permits(), 255);
}

#[tokio::test]
async fn cov_lifetime_multiple_stale_success_and_current_failure_admit_nothing() {
    for initial in [false, true] {
        for confirmed in [false, true] {
            mixed_case(initial, confirmed, false).await;
        }
    }
}

#[tokio::test]
async fn cov_lifetime_multiple_retained_values_alone_own_lifetime_companions_and_timestamps() {
    for initial in [false, true] {
        for confirmed in [false, true] {
            mixed_case(initial, confirmed, true).await;
        }
    }
}
