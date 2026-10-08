use super::value_source::{decoded, FIELDS};
use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use std::borrow::Cow;

#[derive(Default)]
struct State {
    fail: Option<PropertyIdentifier>,
    overrides: HashMap<PropertyIdentifier, PropertyValue>,
    reads: HashMap<PropertyIdentifier, usize>,
    noncommandable: bool,
}
struct Probe {
    inner: AnalogValueObject,
    state: Arc<StdMutex<State>>,
}
impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        object()
    }
    fn object_name(&self) -> &str {
        "command COV probe"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        let mut list = self.inner.property_list().into_owned();
        if self.state.lock().unwrap().noncommandable {
            list.retain(|p| *p != PropertyIdentifier::PRIORITY_ARRAY);
        }
        Cow::Owned(list)
    }
    fn supports_cov(&self) -> bool {
        true
    }
    fn cov_increment(&self) -> Option<f64> {
        Some(2.0)
    }
    fn read_property(&self, p: PropertyIdentifier, i: Option<u32>) -> Result<PropertyValue, Error> {
        let mut state = self.state.lock().unwrap();
        *state.reads.entry(p).or_default() += 1;
        if state.fail == Some(p) {
            return Err(Error::Encoding("injected companion read failure".into()));
        }
        if let Some(value) = state.overrides.get(&p) {
            return Ok(value.clone());
        }
        self.inner.read_property(p, i)
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Encoding("read only probe".into()))
    }
}
async fn fixture(
    kind: CovNotificationKind,
) -> (Fixture, Arc<StdMutex<State>>, CovSubscriptionSnapshot) {
    let f = Fixture::new(false);
    let state = Arc::new(StdMutex::new(State::default()));
    let mut av = AnalogValueObject::new(3, "probe", 95).unwrap();
    av.set_relinquish_default(10.0).unwrap();
    let mut db = clocked_test_database();
    db.add(Box::new(Probe {
        inner: av,
        state: state.clone(),
    }))
    .unwrap();
    *f.db.write().await = db;
    let mut proposal = proposal(kind, false, PropertyIdentifier::VALUE_SOURCE);
    proposal.last_notified_observation = None;
    let sub = f.table.write().await.admit_for_test(proposal, 0).unwrap();
    (f, state, sub)
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
async fn value_source_cov_failed_companion_atomicity_and_recovery() {
    for noncommandable in [false, true] {
        failed_companion_atomicity_and_recovery(noncommandable).await;
    }
}
async fn failed_companion_atomicity_and_recovery(noncommandable: bool) {
    let fields = &FIELDS[..if noncommandable { 3 } else { 5 }];
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let (f, state, sub) = fixture(kind).await;
        state.lock().unwrap().noncommandable = noncommandable;
        f.fire(true, std::slice::from_ref(&sub)).await;
        f.sent.lock().unwrap().clear();
        let before = baseline(&f, &sub).await;
        for &property in fields {
            state.lock().unwrap().fail = Some(property);
            for force in [false, true] {
                f.fire(force, std::slice::from_ref(&sub)).await;
                assert!(
                    f.sent.lock().unwrap().is_empty(),
                    "{property:?} force={force}"
                );
                assert_eq!(baseline(&f, &sub).await, before);
            }
        }
        state.lock().unwrap().fail = None;
        for (property, value) in [
            (
                PropertyIdentifier::VALUE_SOURCE,
                PropertyValue::ApplicationData(vec![0; 65537]),
            ),
            (
                PropertyIdentifier::VALUE_SOURCE,
                PropertyValue::ApplicationData(vec![0x08, 0x08]),
            ),
            (
                PropertyIdentifier::LAST_COMMAND_TIME,
                PropertyValue::ApplicationData(vec![0x19, 1, 0]),
            ),
            (
                PropertyIdentifier::LAST_COMMAND_TIME,
                PropertyValue::Unsigned(1),
            ),
            (
                PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
                PropertyValue::Unsigned(17),
            ),
            (
                PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
                PropertyValue::Unsigned(0),
            ),
            (
                PropertyIdentifier::PRESENT_VALUE,
                PropertyValue::Real(f32::NAN),
            ),
            (
                PropertyIdentifier::STATUS_FLAGS,
                PropertyValue::BitString {
                    unused_bits: 4,
                    data: vec![1],
                },
            ),
        ] {
            if !fields.contains(&property) {
                continue;
            }
            state.lock().unwrap().overrides.insert(property, value);
            f.fire(true, std::slice::from_ref(&sub)).await;
            assert!(f.sent.lock().unwrap().is_empty(), "{property:?}");
            assert_eq!(baseline(&f, &sub).await, before);
            state.lock().unwrap().overrides.clear();
        }
        state
            .lock()
            .unwrap()
            .overrides
            .insert(PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.0));
        f.fire(false, &[]).await;
        assert_eq!(f.sent.lock().unwrap().len(), 1);
        assert_ne!(baseline(&f, &sub).await, before);
        f.finish(false).await;
    }
}
#[tokio::test]
async fn value_source_cov_time_only_and_noncommandable_control() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let (f, state, sub) = fixture(kind).await;
        f.fire(true, std::slice::from_ref(&sub)).await;
        f.sent.lock().unwrap().clear();
        let before = baseline(&f, &sub).await;
        state.lock().unwrap().overrides.insert(
            PropertyIdentifier::LAST_COMMAND_TIME,
            PropertyValue::ApplicationData(vec![0x19, 42]),
        );
        f.fire(false, &[]).await;
        assert!(f.sent.lock().unwrap().is_empty());
        assert_eq!(baseline(&f, &sub).await, before);
        state.lock().unwrap().noncommandable = true;
        f.fire(true, std::slice::from_ref(&sub)).await;
        let values = decoded(f.sent.lock().unwrap().pop().unwrap(), kind);
        assert_eq!(
            values.iter().map(|v| v.0).collect::<Vec<_>>(),
            [
                PropertyIdentifier::PRESENT_VALUE,
                PropertyIdentifier::STATUS_FLAGS,
                PropertyIdentifier::VALUE_SOURCE,
            ]
        );
        f.finish(false).await;
    }
}
#[tokio::test]
async fn value_source_cov_multiple_dedup_qualification_and_timestamps() {
    for noncommandable in [false, true] {
        multiple_dedup_qualification_and_timestamps(noncommandable).await;
    }
}
async fn multiple_dedup_qualification_and_timestamps(noncommandable: bool) {
    let fields = &FIELDS[..if noncommandable { 3 } else { 5 }];
    let kind = CovNotificationKind::Multiple;
    let (f, state, source) = fixture(kind).await;
    state.lock().unwrap().noncommandable = noncommandable;
    let mut snapshots = vec![source.clone()];
    for &property in fields {
        let mut p = proposal(kind, false, property);
        p.last_notified_observation = None;
        p.timestamped = property == PropertyIdentifier::VALUE_SOURCE;
        p.cov_increment = Some(100.0);
        let accepted = f.table.write().await.admit_for_test(p, 0).unwrap();
        snapshots.retain(|s| s.monitored_property != Some(property));
        snapshots.push(accepted);
    }
    // Use current generations after the admissions above.
    let snapshots = f
        .table
        .write()
        .await
        .subscriptions_for(&object())
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    state.lock().unwrap().reads.clear();
    f.fire(true, &snapshots).await;
    let frame = f.sent.lock().unwrap().pop().unwrap();
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
    else {
        panic!()
    };
    let report = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    let values = &report.list_of_cov_notifications[0].list_of_values;
    assert_eq!(values.len(), fields.len());
    for &field in fields {
        assert_eq!(
            values
                .iter()
                .filter(|v| v.property_identifier == field)
                .count(),
            1
        );
        assert_eq!(
            values
                .iter()
                .find(|v| v.property_identifier == field)
                .unwrap()
                .time_of_change
                .is_some(),
            field == PropertyIdentifier::VALUE_SOURCE,
            "qualified explicit false selector controls {field:?}"
        );
        assert_eq!(
            state.lock().unwrap().reads[&field],
            1,
            "one capture of {field:?}"
        );
    }
    if noncommandable {
        let state = state.lock().unwrap();
        assert!(!state
            .reads
            .contains_key(&PropertyIdentifier::LAST_COMMAND_TIME));
        assert!(!state
            .reads
            .contains_key(&PropertyIdentifier::CURRENT_COMMAND_PRIORITY));
    }
    let pv = snapshots
        .iter()
        .find(|s| s.monitored_property == Some(PropertyIdentifier::PRESENT_VALUE))
        .unwrap();
    let before = baseline(&f, pv).await;
    // Source reference qualifies at object increment2; selected PV increment100
    // does not. Inclusion as a companion must not commit that selected baseline.
    state
        .lock()
        .unwrap()
        .overrides
        .insert(PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.0));
    f.fire(false, &[]).await;
    let frame = f.sent.lock().unwrap().pop().unwrap();
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
    else {
        panic!()
    };
    let report = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    let values = &report.list_of_cov_notifications[0].list_of_values;
    assert_eq!(values.len(), fields.len());
    for value in values {
        assert_eq!(
            value.time_of_change.is_some(),
            value.property_identifier == PropertyIdentifier::VALUE_SOURCE,
            "an explicit false selector governs {:?} even unqualified (#856)",
            value.property_identifier
        );
    }
    assert_eq!(baseline(&f, pv).await, before);
    let source = snapshots
        .iter()
        .find(|s| s.monitored_property == Some(PropertyIdentifier::VALUE_SOURCE))
        .unwrap();
    assert_ne!(
        baseline(&f, source)
            .await
            .unwrap()
            .source_companions()
            .unwrap()
            .0,
        before.unwrap().sample().clone()
    );
    f.finish(false).await;
}
#[tokio::test]
async fn value_source_cov_multiple_failed_source_keeps_healthy_sibling() {
    let kind = CovNotificationKind::Multiple;
    let (f, state, source) = fixture(kind).await;
    let mut p = proposal(kind, false, PropertyIdentifier::RELINQUISH_DEFAULT);
    p.last_notified_observation = None;
    f.table.write().await.admit_for_test(p, 0).unwrap();
    let snapshots = f
        .table
        .write()
        .await
        .subscriptions_for(&object())
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    state.lock().unwrap().fail = Some(PropertyIdentifier::LAST_COMMAND_TIME);
    f.fire(true, &snapshots).await;
    let values = decoded(f.sent.lock().unwrap().pop().unwrap(), kind);
    assert_eq!(
        values.iter().map(|v| v.0).collect::<Vec<_>>(),
        [
            PropertyIdentifier::RELINQUISH_DEFAULT,
            PropertyIdentifier::STATUS_FLAGS
        ]
    );
    assert!(baseline(&f, &source).await.is_none());
    state.lock().unwrap().fail = None;
    f.fire(false, &[]).await;
    assert_eq!(
        decoded(f.sent.lock().unwrap().pop().unwrap(), kind).len(),
        5
    );
    f.finish(false).await;
}

#[tokio::test]
async fn value_source_cov_multiple_timestamped_sibling_merges_flags_only_when_qualified() {
    let kind = CovNotificationKind::Multiple;
    let (f, state, source) = fixture(kind).await;
    let mut p = proposal(kind, false, PropertyIdentifier::PRESENT_VALUE);
    p.last_notified_observation = None;
    p.timestamped = true;
    p.cov_increment = Some(100.0);
    let pv = f.table.write().await.admit_for_test(p, 0).unwrap();
    let snapshots = f
        .table
        .write()
        .await
        .subscriptions_for(&object())
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    f.fire(true, &snapshots).await;
    let frame = f.sent.lock().unwrap().pop().unwrap();
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
    else {
        panic!()
    };
    let report = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    let values = &report.list_of_cov_notifications[0].list_of_values;
    assert_eq!(values.len(), 5);
    for v in values {
        assert_eq!(
            v.time_of_change.is_some(),
            matches!(
                v.property_identifier,
                PropertyIdentifier::PRESENT_VALUE | PropertyIdentifier::STATUS_FLAGS
            ),
            "{:?}",
            v.property_identifier
        );
    }
    let pv_before = baseline(&f, &pv).await;
    state
        .lock()
        .unwrap()
        .overrides
        .insert(PropertyIdentifier::PRESENT_VALUE, PropertyValue::Real(12.0));
    // A known preparation time, distinct from the system clock of the first
    // report.
    let prepared = crate::server::cov_wire_test_support::at(42);
    f.db.write().await.set_clock_reader(Some(Arc::new(
        crate::server::cov_wire_test_support::SharedClock(Arc::new(StdMutex::new(prepared))),
    )));
    f.fire(false, &[]).await;
    let frame = f.sent.lock().unwrap().pop().unwrap();
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
    else {
        panic!()
    };
    let report = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    // The unqualified timestamped PV selector adds no companion timestamps.
    // Its own field, carried by the Value_Source report, still needs a time.
    // PV moved less than the selector's increment, so the selector captured
    // no change: 12.0 takes the preparation time, never the time of the value
    // the selector last captured (#987).
    for v in &report.list_of_cov_notifications[0].list_of_values {
        let expected = (v.property_identifier == PropertyIdentifier::PRESENT_VALUE)
            .then_some(prepared.local_time);
        assert_eq!(v.time_of_change, expected, "{:?}", v.property_identifier);
    }
    assert_eq!(
        report.timestamp,
        Some((prepared.local_date, prepared.local_time))
    );
    // A later Value_Source report, at 50, while PV is still 12.0: the value
    // keeps the time it was first given.
    f.db.write().await.set_clock_reader(Some(Arc::new(
        crate::server::cov_wire_test_support::SharedClock(Arc::new(StdMutex::new(
            crate::server::cov_wire_test_support::at(50),
        ))),
    )));
    state.lock().unwrap().overrides.insert(
        PropertyIdentifier::CURRENT_COMMAND_PRIORITY,
        PropertyValue::Unsigned(8),
    );
    f.fire(false, &[]).await;
    let frame = f.sent.lock().unwrap().pop().unwrap();
    let Apdu::UnconfirmedRequest(request) =
        decode_apdu(decode_npdu(frame).unwrap().payload).unwrap()
    else {
        panic!()
    };
    let later = COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
    let pv_times: Vec<_> = later.list_of_cov_notifications[0]
        .list_of_values
        .iter()
        .filter(|v| v.property_identifier == PropertyIdentifier::PRESENT_VALUE)
        .map(|v| v.time_of_change)
        .collect();
    assert_eq!(pv_times, vec![Some(prepared.local_time)]);
    assert_eq!(baseline(&f, &pv).await, pv_before);
    assert!(baseline(&f, &source)
        .await
        .unwrap()
        .source_companions()
        .is_some());
    f.finish(false).await;
}
