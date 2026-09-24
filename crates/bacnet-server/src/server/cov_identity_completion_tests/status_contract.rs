use super::status_flags::values;
use super::*;
use bacnet_objects::traits::BACnetObject;
use std::borrow::Cow;

struct State {
    present: bool,
    flags: PropertyValue,
    flags_fail: bool,
    selected_fail: bool,
    selected: PropertyValue,
    reads: usize,
    changing: bool,
    increment: Option<f32>,
}
struct Probe(Arc<StdMutex<State>>);
fn flags(bits: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![bits << 4],
    }
}
impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        object()
    }
    fn object_name(&self) -> &str {
        "flags probe"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let mut p = vec![
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::RELINQUISH_DEFAULT,
        ];
        if self.0.lock().unwrap().present {
            p.push(PropertyIdentifier::STATUS_FLAGS);
        }
        Cow::Owned(p)
    }
    fn supports_cov(&self) -> bool {
        true
    }
    fn cov_increment(&self) -> Option<f32> {
        self.0.lock().unwrap().increment
    }
    fn read_property(&self, p: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        let mut s = self.0.lock().unwrap();
        if p == PropertyIdentifier::STATUS_FLAGS {
            s.reads += 1;
            if s.flags_fail {
                return Err(Error::Encoding("transient flags failure".into()));
            }
            if s.changing {
                s.selected = PropertyValue::Real(s.reads as f32);
                s.flags = flags(s.reads as u8);
            }
            return Ok(s.flags.clone());
        }
        if s.selected_fail {
            return Err(Error::Encoding("selected failure".into()));
        }
        Ok(s.selected.clone())
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Encoding("read only".into()))
    }
}
async fn fixture(
    kind: CovNotificationKind,
    ordinary: bool,
) -> (Fixture, Arc<StdMutex<State>>, CovSubscriptionSnapshot) {
    let f = Fixture::new(false);
    let s = Arc::new(StdMutex::new(State {
        present: true,
        flags: flags(0),
        flags_fail: false,
        selected_fail: false,
        selected: PropertyValue::Real(10.0),
        reads: 0,
        changing: false,
        increment: Some(2.0),
    }));
    let mut db = clocked_test_database();
    db.add(Box::new(Probe(s.clone()))).unwrap();
    *f.db.write().await = db;
    let mut sub = proposal(kind, false, PropertyIdentifier::RELINQUISH_DEFAULT);
    sub.last_notified_observation = None;
    sub.cov_increment = Some(2.0);
    if ordinary {
        sub.monitored_property = None;
    }
    let accepted = f.table.write().await.subscribe(sub).unwrap();
    (f, s, accepted)
}
async fn baseline(
    f: &Fixture,
    sub: &CovSubscriptionSnapshot,
) -> Option<crate::cov::CovObservation> {
    f.table
        .read()
        .await
        .get_subscription(sub.key())
        .unwrap()
        .last_notified_observation
        .clone()
}
#[tokio::test]
async fn cov_status_failures_preserve_whole_observation_and_recover() {
    for (kind, ordinary) in [
        (CovNotificationKind::Single, true),
        (CovNotificationKind::Single, false),
        (CovNotificationKind::Multiple, false),
    ] {
        let (f, s, sub) = fixture(kind, ordinary).await;
        f.fire(true, std::slice::from_ref(&sub)).await;
        f.sent.lock().unwrap().clear();
        let before = baseline(&f, &sub).await;
        for mode in 0..7 {
            {
                let mut s = s.lock().unwrap();
                s.flags_fail = mode == 0;
                s.selected_fail = mode == 1;
                s.flags = match mode {
                    2 => PropertyValue::Real(1.0),
                    3 => PropertyValue::BitString {
                        unused_bits: 3,
                        data: vec![0],
                    },
                    4 => PropertyValue::BitString {
                        unused_bits: 4,
                        data: vec![1],
                    },
                    5 => PropertyValue::BitString {
                        unused_bits: 4,
                        data: vec![0, 0],
                    },
                    _ => flags(1),
                };
                s.selected = if mode == 6 {
                    PropertyValue::OctetString(vec![0; 65537])
                } else {
                    PropertyValue::Real(20.0)
                };
            }
            f.fire(false, &[]).await;
            assert!(f.sent.lock().unwrap().is_empty(), "mode {mode}");
            assert_eq!(baseline(&f, &sub).await, before);
        }
        {
            let mut s = s.lock().unwrap();
            s.selected = PropertyValue::Real(20.0);
            s.flags = flags(1);
        }
        f.fire(false, &[]).await;
        assert_eq!(f.sent.lock().unwrap().len(), 1);
        assert_eq!(baseline(&f, &sub).await.unwrap().status_flags(), Some(1));
        f.finish(false).await;
    }
}
#[tokio::test]
async fn cov_status_declared_presence_transitions_are_delivery_based() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let (f, s, sub) = fixture(kind, false).await;
        s.lock().unwrap().present = false;
        f.fire(true, std::slice::from_ref(&sub)).await;
        assert_eq!(values(f.sent.lock().unwrap().pop().unwrap(), kind).len(), 1);
        assert_eq!(baseline(&f, &sub).await.unwrap().status_flags(), None);
        s.lock().unwrap().present = true;
        f.fire(false, &[]).await;
        assert_eq!(values(f.sent.lock().unwrap().pop().unwrap(), kind).len(), 2);
        let before = baseline(&f, &sub).await;
        s.lock().unwrap().present = false;
        f.fire(false, &[]).await;
        assert!(f.sent.lock().unwrap().is_empty());
        assert_eq!(baseline(&f, &sub).await, before);
        s.lock().unwrap().selected = PropertyValue::Real(20.0);
        f.fire(false, &[]).await;
        assert_eq!(values(f.sent.lock().unwrap().pop().unwrap(), kind).len(), 1);
        assert_eq!(baseline(&f, &sub).await.unwrap().status_flags(), None);
        // An undeclared explicit selection is not converted to an absent companion.
        let mut explicit = proposal(kind, false, PropertyIdentifier::STATUS_FLAGS);
        explicit.last_notified_observation = None;
        let explicit = f.table.write().await.subscribe(explicit).unwrap();
        f.fire(true, std::slice::from_ref(&explicit)).await;
        assert!(f.sent.lock().unwrap().is_empty());
        assert!(baseline(&f, &explicit).await.is_none());
        f.finish(false).await;
    }
}
#[tokio::test]
async fn cov_status_multiple_one_read_per_context_and_independent_baselines() {
    let (f, s, first) = fixture(CovNotificationKind::Multiple, false).await;
    let mut explicit = proposal(
        CovNotificationKind::Multiple,
        false,
        PropertyIdentifier::STATUS_FLAGS,
    );
    explicit.last_notified_observation = None;
    let second = f.table.write().await.subscribe(explicit.clone()).unwrap();
    s.lock().unwrap().changing = true;
    f.fire(true, &[first.clone(), second.clone()]).await;
    assert_eq!(s.lock().unwrap().reads, 1);
    let payload = values(
        f.sent.lock().unwrap().pop().unwrap(),
        CovNotificationKind::Multiple,
    );
    assert_eq!(payload.len(), 2);
    assert_eq!(
        payload
            .iter()
            .filter(|v| v.0 == PropertyIdentifier::STATUS_FLAGS)
            .count(),
        1
    );
    assert_eq!(
        baseline(&f, &first).await.unwrap().sample().value(),
        &PropertyValue::Real(1.0)
    );
    assert_eq!(baseline(&f, &first).await.unwrap().status_flags(), Some(1));
    // Separate contexts sample successively, each pair is internally consistent.
    explicit.subscriber_process_identifier = 2;
    explicit.monitored_property = Some(PropertyIdentifier::RELINQUISH_DEFAULT);
    let third = f.table.write().await.subscribe(explicit).unwrap();
    s.lock().unwrap().reads = 0;
    f.fire(true, &[first.clone(), second.clone(), third.clone()])
        .await;
    assert_eq!(s.lock().unwrap().reads, 2);
    for sub in [&first, &third] {
        let o = baseline(&f, sub).await.unwrap();
        assert_eq!(
            o.sample().value(),
            &PropertyValue::Real(o.status_flags().unwrap() as f32)
        );
    }
    assert_eq!(f.sent.lock().unwrap().len(), 2);
    f.sent.lock().unwrap().clear();
    // One reference has already delivered the present flags; its sibling has not.
    s.lock().unwrap().changing = false;
    s.lock().unwrap().selected = PropertyValue::Real(1.0);
    s.lock().unwrap().flags = flags(3);
    let sample = crate::cov::CovSample::new(&PropertyValue::Real(1.0)).unwrap();
    let o = crate::cov::CovObservation::new(sample, Some(&flags(3))).unwrap();
    f.table
        .write()
        .await
        .set_last_notified_observation(&first, o.clone());
    f.table.write().await.unsubscribe(second.key());
    f.table.write().await.unsubscribe(third.key());
    let mut sibling = proposal(
        CovNotificationKind::Multiple,
        false,
        PropertyIdentifier::PRESENT_VALUE,
    );
    sibling.last_notified_observation =
        Some(crate::cov::CovObservation::new(o.sample().clone(), Some(&flags(2))).unwrap());
    let sibling = f.table.write().await.subscribe(sibling).unwrap();
    f.fire(false, &[]).await;
    let payload = values(
        f.sent.lock().unwrap().pop().unwrap(),
        CovNotificationKind::Multiple,
    );
    assert_eq!(payload.len(), 2);
    assert!(!payload
        .iter()
        .any(|v| v.0 == PropertyIdentifier::RELINQUISH_DEFAULT));
    assert_eq!(baseline(&f, &first).await, Some(o.clone()));
    assert_eq!(baseline(&f, &sibling).await, Some(o));
    f.finish(false).await;
}

#[tokio::test]
async fn cov_status_delivery_failures_and_denied_admission_preserve_pair() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let (f, s, sub) = fixture(kind, false).await;
        f.fire(true, std::slice::from_ref(&sub)).await;
        f.sent.lock().unwrap().clear();
        let before = baseline(&f, &sub).await;
        s.lock().unwrap().flags = flags(1);
        f.fail.store(true, Ordering::Relaxed);
        f.fire(false, &[]).await;
        assert_eq!(f.sent.lock().unwrap().len(), 1);
        assert_eq!(baseline(&f, &sub).await, before);
        f.fail.store(false, Ordering::Relaxed);
        f.sent.lock().unwrap().clear();
        f.fire(false, &[]).await;
        assert_eq!(baseline(&f, &sub).await.unwrap().status_flags(), Some(1));
        f.table.write().await.unsubscribe(sub.key());
        let mut confirmed = proposal(kind, true, PropertyIdentifier::RELINQUISH_DEFAULT);
        confirmed.last_notified_observation = before.clone();
        let confirmed = f.table.write().await.subscribe(confirmed).unwrap();
        f.sent.lock().unwrap().clear();
        let permits = f.permits.acquire_many(8).await.unwrap();
        f.fire(true, std::slice::from_ref(&confirmed)).await;
        assert!(f.sent.lock().unwrap().is_empty());
        assert_eq!(f.transactions.active_count(), 0);
        assert_eq!(baseline(&f, &confirmed).await, before);
        drop(permits);
        f.finish(false).await;
    }
}

#[tokio::test]
async fn cov_status_snapshot_captures_companion_without_live_db_fallback() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let (f, s, sub) = fixture(kind, false).await;
        let captured = Arc::new(StdMutex::new(State {
            present: true,
            flags: flags(8),
            flags_fail: false,
            selected_fail: false,
            selected: PropertyValue::Real(42.0),
            reads: 0,
            changing: false,
            increment: Some(2.0),
        }));
        s.lock().unwrap().flags_fail = true;
        BACnetServer::<HeldTransport>::fire_cov_notifications_inner(
            &f.db,
            &f.network,
            &f.table,
            &f.permits,
            &f.transactions,
            &f.comm,
            &f.config,
            &object(),
            Some(&Probe(captured.clone())),
        )
        .await;
        let values = values(f.sent.lock().unwrap().pop().unwrap(), kind);
        assert_eq!(values.len(), 2);
        assert_eq!(captured.lock().unwrap().reads, 1);
        let o = baseline(&f, &sub).await.unwrap();
        assert_eq!(o.status_flags(), Some(8));
        assert_eq!(o.sample().value(), &PropertyValue::Real(42.0));
        assert_eq!(s.lock().unwrap().reads, 0);
        f.finish(false).await;
    }
}

#[tokio::test]
async fn cov_status_ordinary_preserves_successful_nonnumeric_and_no_increment_fanout() {
    for nonnumeric in [true, false] {
        let (f, s, sub) = fixture(CovNotificationKind::Single, true).await;
        if nonnumeric {
            s.lock().unwrap().selected = PropertyValue::Boolean(false);
        } else {
            // Omitted subscription and object increments preserve ordinary fanout.
            s.lock().unwrap().increment = None;
            let mut sub = (*sub).clone();
            sub.cov_increment = None;
            f.table.write().await.subscribe(sub).unwrap();
        }
        f.fire(false, &[]).await;
        f.fire(false, &[]).await;
        assert_eq!(f.sent.lock().unwrap().len(), 2);
        f.finish(false).await;
    }
}

#[tokio::test]
async fn cov_status_explicit_single_reuses_one_read_and_one_value() {
    let (f, s, sub) = fixture(CovNotificationKind::Single, false).await;
    f.table.write().await.unsubscribe(sub.key());
    let mut explicit = proposal(
        CovNotificationKind::Single,
        false,
        PropertyIdentifier::STATUS_FLAGS,
    );
    explicit.last_notified_observation = None;
    let explicit = f.table.write().await.subscribe(explicit).unwrap();
    f.fire(true, std::slice::from_ref(&explicit)).await;
    assert_eq!(s.lock().unwrap().reads, 1);
    let values = values(
        f.sent.lock().unwrap().pop().unwrap(),
        CovNotificationKind::Single,
    );
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].0, PropertyIdentifier::STATUS_FLAGS);
    assert_eq!(
        baseline(&f, &explicit).await.unwrap().sample().value(),
        &flags(0)
    );
    f.finish(false).await;
}
