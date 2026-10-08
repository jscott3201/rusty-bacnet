use super::*;

pub(super) struct CallGate {
    pub record_before: bool,
    entered: Notify,
    release: Semaphore,
    fail: bool,
}
impl CallGate {
    fn new(record_before: bool, fail: bool) -> Arc<Self> {
        Arc::new(Self {
            record_before,
            entered: Notify::new(),
            release: Semaphore::new(0),
            fail,
        })
    }
    pub async fn wait(&self) -> Result<(), Error> {
        self.entered.notify_one();
        self.release.acquire().await.unwrap().forget();
        if self.fail {
            Err(Error::Encoding("per-call send failure".into()))
        } else {
            Ok(())
        }
    }
}
async fn command(f: &Fixture, value: f32) {
    f.db.write()
        .await
        .get_mut(&object())
        .unwrap()
        .write_property_from(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(value),
            Some(8),
            &crate::command_source::test_origin(),
        )
        .unwrap();
}
async fn observation(f: &Fixture, sub: &CovSubscriptionSnapshot) -> crate::cov::CovObservation {
    f.table
        .read()
        .await
        .get_subscription(sub.key())
        .unwrap()
        .last_notified_observation
        .clone()
        .unwrap()
}
fn recorded(f: &Fixture, kind: CovNotificationKind) -> Vec<f32> {
    f.sent
        .lock()
        .unwrap()
        .iter()
        .map(|frame| {
            let values = super::value_source::decoded(frame.clone(), kind);
            let bytes = &values
                .iter()
                .find(|v| v.0 == PropertyIdentifier::PRESENT_VALUE)
                .unwrap()
                .1;
            let (PropertyValue::Real(value), end) =
                bacnet_encoding::primitives::decode_application_value(bytes, 0).unwrap()
            else {
                panic!("real PV")
            };
            assert_eq!(end, bytes.len());
            value
        })
        .collect()
}
async fn delayed_older_success(kind: CovNotificationKind, ordinary: bool, record_before: bool) {
    let f = Fixture::new(false);
    let mut p = proposal(kind, false, PropertyIdentifier::PRESENT_VALUE);
    p.cov_increment = Some(5.0);
    if ordinary {
        p.monitored_property = None;
    }
    p.last_notified_observation = Some(
        crate::cov::CovObservation::new(
            crate::cov::CovSample::new(&PropertyValue::Real(0.0)).unwrap(),
            Some(&PropertyValue::BitString {
                unused_bits: 4,
                data: vec![0],
            }),
        )
        .unwrap(),
    );
    let sub = f.table.write().await.admit_for_test(p, 0).unwrap();
    command(&f, 10.0).await;
    let a = CallGate::new(record_before, false);
    f.gates.lock().unwrap().push_back(a.clone());
    let mut work = Box::pin(f.fire(false, &[]));
    assert!(futures_util::poll!(work.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_secs(2), a.entered.notified())
        .await
        .unwrap();
    command(&f, 20.0).await;
    f.fire(false, &[]).await;
    let newest = observation(&f, &sub).await;
    assert_eq!(newest.sample().value(), &PropertyValue::Real(20.0));
    assert!(
        f.table.read().await.is_current(&sub),
        "unchanged accepted generation"
    );
    a.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), work)
        .await
        .unwrap();
    assert_eq!(
        observation(&f, &sub).await,
        newest,
        "older successful completion must not replace newer observation"
    );
    command(&f, 14.0).await;
    f.fire(false, &[]).await;
    assert_eq!(
        recorded(&f, kind),
        if record_before {
            vec![10.0, 20.0, 14.0]
        } else {
            vec![20.0, 10.0, 14.0]
        }
    );
    f.finish(false).await;
}
#[tokio::test]
async fn cov_order_single_late_success_preserves_newer_observation() {
    delayed_older_success(CovNotificationKind::Single, false, true).await;
}
#[tokio::test]
async fn cov_order_multiple_late_success_preserves_newer_observation() {
    delayed_older_success(CovNotificationKind::Multiple, false, true).await;
}

#[tokio::test]
async fn cov_order_ordinary_and_reverse_fake_recording_order() {
    delayed_older_success(CovNotificationKind::Single, true, true).await;
    for (kind, ordinary) in [
        (CovNotificationKind::Single, true),
        (CovNotificationKind::Single, false),
        (CovNotificationKind::Multiple, false),
    ] {
        delayed_older_success(kind, ordinary, false).await;
    }
}
#[tokio::test]
async fn cov_order_newer_failure_or_cancellation_does_not_block_older_success() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        for cancel in [false, true] {
            let f = Fixture::new(false);
            let mut p = proposal(kind, false, PropertyIdentifier::PRESENT_VALUE);
            p.last_notified_observation = None;
            let sub = f.table.write().await.admit_for_test(p, 0).unwrap();
            command(&f, 10.0).await;
            let a = CallGate::new(true, false);
            f.gates.lock().unwrap().push_back(a.clone());
            let mut older = Box::pin(f.fire(false, &[]));
            assert!(futures_util::poll!(older.as_mut()).is_pending());
            tokio::time::timeout(Duration::from_secs(2), a.entered.notified())
                .await
                .unwrap();
            command(&f, 20.0).await;
            let b = CallGate::new(true, true);
            f.gates.lock().unwrap().push_back(b.clone());
            let mut newer = Box::pin(f.fire(false, &[]));
            assert!(futures_util::poll!(newer.as_mut()).is_pending());
            tokio::time::timeout(Duration::from_secs(2), b.entered.notified())
                .await
                .unwrap();
            if cancel {
                drop(newer);
            } else {
                b.release.add_permits(1);
                newer.await;
            }
            assert!(f
                .table
                .read()
                .await
                .get_subscription(sub.key())
                .unwrap()
                .last_notified_observation
                .is_none());
            a.release.add_permits(1);
            older.await;
            assert_eq!(
                observation(&f, &sub).await.sample().value(),
                &PropertyValue::Real(10.0)
            );
            command(&f, 30.0).await;
            f.fire(false, &[]).await;
            assert_eq!(
                observation(&f, &sub).await.sample().value(),
                &PropertyValue::Real(30.0)
            );
            f.finish(false).await;
        }
    }
}
#[tokio::test]
async fn cov_order_specialized_whole_tuple_is_never_partially_replaced() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let f = Fixture::new(false);
        let mut p = proposal(kind, false, PropertyIdentifier::VALUE_SOURCE);
        p.last_notified_observation = None;
        let sub = f.table.write().await.admit_for_test(p, 0).unwrap();
        command(&f, 10.0).await;
        let a = CallGate::new(true, false);
        f.gates.lock().unwrap().push_back(a.clone());
        let mut older = Box::pin(f.fire(false, &[]));
        assert!(futures_util::poll!(older.as_mut()).is_pending());
        tokio::time::timeout(Duration::from_secs(2), a.entered.notified())
            .await
            .unwrap();
        {
            let mut db = f.db.write().await;
            let obj = db.get_mut(&object()).unwrap();
            let origin = bacnet_objects::command_source::CommandOrigin::Local {
                owner_device: ObjectIdentifier::new(ObjectType::DEVICE, 2).unwrap(),
                initiating_object: None,
            };
            obj.write_property_from(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Real(20.0),
                Some(4),
                &origin,
            )
            .unwrap();
            obj.write_property(
                PropertyIdentifier::OUT_OF_SERVICE,
                None,
                PropertyValue::Boolean(true),
                None,
            )
            .unwrap();
        }
        f.fire(false, &[]).await;
        let newest = observation(&f, &sub).await;
        assert_eq!(
            newest.source_companions().unwrap().0.value(),
            &PropertyValue::Real(20.0)
        );
        assert_eq!(
            newest
                .source_companions()
                .unwrap()
                .1
                .as_ref()
                .unwrap()
                .value(),
            &PropertyValue::Unsigned(4)
        );
        assert_eq!(newest.status_flags(), Some(1));
        a.release.add_permits(1);
        older.await;
        assert_eq!(observation(&f, &sub).await, newest);
        let reports = f.sent.lock().unwrap().clone();
        for frame in &reports {
            assert_eq!(super::value_source::decoded(frame.clone(), kind).len(), 5);
        }
        let sent_source = super::value_source::decoded(reports[1].clone(), kind)
            .into_iter()
            .find(|v| v.0 == PropertyIdentifier::VALUE_SOURCE)
            .unwrap()
            .1;
        assert_eq!(
            newest.sample().value(),
            &PropertyValue::ApplicationData(sent_source)
        );
        f.finish(false).await;
    }
}

struct Dispatch {
    counters: Arc<crate::cov::AtomicCovCounters>,
    tracker: Arc<crate::cov::CovInFlightTracker>,
}
impl Dispatch {
    async fn new(f: &Fixture) -> Self {
        let table = f.table.read().await;
        Self {
            counters: table.counters().clone(),
            tracker: table.in_flight_tracker().clone(),
        }
    }
    async fn send(&self, f: &Fixture, subs: &[CovSubscriptionSnapshot]) {
        let mut budget = super::super::cov_notifications::EventBudget::new(&f.config.cov_policy);
        BACnetServer::<TestTransport>::fire_cov_notifications_for_subscriptions(
            &crate::server::cov_notify_context::CovFanoutHandles {
                ctx: &crate::server::cov_notify_context::CovNotifyContext {
                    db: &f.db,
                    network: &f.network,
                    cov_table: &f.table,
                    cov_in_flight: &f.permits,
                    notification_transactions: &f.transactions,
                    comm_state: &f.comm,
                    config: &f.config,
                },
                in_flight_tracker: &self.tracker,
                counters: &self.counters,
            },
            &object(),
            subs,
            None,
            false,
            &mut budget,
        )
        .await;
    }
}
#[tokio::test]
async fn cov_order_ordinary_ticket_precedes_earlier_property_send_await() {
    let f = Fixture::new(false);
    let mut p = proposal(
        CovNotificationKind::Single,
        false,
        PropertyIdentifier::PRESENT_VALUE,
    );
    p.monitored_property = None;
    let ordinary = f.table.write().await.admit_for_test(p, 0).unwrap();
    let property = f
        .table
        .write()
        .await
        .admit_for_test(
            proposal(
                CovNotificationKind::Single,
                false,
                PropertyIdentifier::RELINQUISH_DEFAULT,
            ),
            0,
        )
        .unwrap();
    command(&f, 10.0).await;
    let dispatch = Dispatch::new(&f).await;
    let a = CallGate::new(true, false);
    f.gates.lock().unwrap().push_back(a.clone());
    let subs = [property, ordinary.clone()];
    let mut older = Box::pin(dispatch.send(&f, &subs));
    assert!(futures_util::poll!(older.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_secs(2), a.entered.notified())
        .await
        .unwrap();
    command(&f, 20.0).await;
    dispatch.send(&f, std::slice::from_ref(&ordinary)).await;
    let newest = observation(&f, &ordinary).await;
    assert_eq!(newest.sample().value(), &PropertyValue::Real(20.0));
    a.release.add_permits(1);
    older.await;
    assert_eq!(observation(&f, &ordinary).await, newest);
    f.finish(false).await;
}
#[tokio::test]
async fn cov_order_single_capture_precedes_final_table_await() {
    for property in [
        PropertyIdentifier::PRESENT_VALUE,
        PropertyIdentifier::VALUE_SOURCE,
    ] {
        let f = Fixture::new(false);
        let mut p = proposal(CovNotificationKind::Single, false, property);
        p.last_notified_observation = None;
        let sub = f.table.write().await.admit_for_test(p, 0).unwrap();
        let subs = [sub.clone()];
        let dispatch = Dispatch::new(&f).await;
        command(&f, 10.0).await;
        let guard = f.table.write().await;
        let mut older = Box::pin(dispatch.send(&f, &subs));
        assert!(futures_util::poll!(older.as_mut()).is_pending());
        assert!(f.sent.lock().unwrap().is_empty());
        // The older DB read guard has been released; its complete preparation
        // must already own the earlier ticket while the table check is blocked.
        command(&f, 20.0).await;
        let mut newer = Box::pin(dispatch.send(&f, &subs));
        assert!(futures_util::poll!(newer.as_mut()).is_pending());
        drop(guard);
        // Poll each table reader through to its own held send, avoiding a
        // queued read permit blocking the other completion's write guard.
        let b = CallGate::new(true, false);
        f.gates.lock().unwrap().push_back(b.clone());
        assert!(futures_util::poll!(newer.as_mut()).is_pending());
        tokio::time::timeout(Duration::from_secs(2), b.entered.notified())
            .await
            .unwrap();
        let a = CallGate::new(true, false);
        f.gates.lock().unwrap().push_back(a.clone());
        assert!(futures_util::poll!(older.as_mut()).is_pending());
        tokio::time::timeout(Duration::from_secs(2), a.entered.notified())
            .await
            .unwrap();
        b.release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(2), newer)
            .await
            .unwrap();
        let newest = observation(&f, &sub).await;
        a.release.add_permits(1);
        tokio::time::timeout(Duration::from_secs(2), older)
            .await
            .unwrap();
        assert_eq!(observation(&f, &sub).await, newest);
        assert_eq!(recorded(&f, CovNotificationKind::Single), [20.0, 10.0]);
        f.finish(false).await;
    }
}

#[tokio::test]
async fn cov_order_multiple_companion_never_borrows_selected_progress() {
    let f = Fixture::new(false);
    let kind = CovNotificationKind::Multiple;
    f.db.write()
        .await
        .get_mut(&object())
        .unwrap()
        .write_property(
            PropertyIdentifier::COV_INCREMENT,
            None,
            PropertyValue::Real(5.0),
            None,
        )
        .unwrap();
    let mut p = proposal(kind, false, PropertyIdentifier::PRESENT_VALUE);
    p.last_notified_observation = None;
    p.cov_increment = Some(5.0);
    let pv = f.table.write().await.admit_for_test(p.clone(), 0).unwrap();
    p.monitored_property = Some(PropertyIdentifier::VALUE_SOURCE);
    let source = f.table.write().await.admit_for_test(p, 0).unwrap();
    command(&f, 10.0).await;
    let a = CallGate::new(true, false);
    f.gates.lock().unwrap().push_back(a.clone());
    let mut older = Box::pin(f.fire(false, &[]));
    assert!(futures_util::poll!(older.as_mut()).is_pending());
    tokio::time::timeout(Duration::from_secs(2), a.entered.notified())
        .await
        .unwrap();
    command(&f, 20.0).await;
    // A later report includes PV as an implicit source companion only.
    f.fire(true, std::slice::from_ref(&source)).await;
    let newest_source = observation(&f, &source).await;
    assert!(f
        .table
        .read()
        .await
        .get_subscription(pv.key())
        .unwrap()
        .last_notified_observation
        .is_none());
    a.release.add_permits(1);
    older.await;
    assert_eq!(observation(&f, &source).await, newest_source);
    assert_eq!(
        observation(&f, &pv).await.sample().value(),
        &PropertyValue::Real(10.0)
    );
    assert_eq!(recorded(&f, kind), [10.0, 20.0]);
    f.finish(false).await;
}
