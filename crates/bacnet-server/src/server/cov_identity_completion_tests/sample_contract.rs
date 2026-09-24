use super::*;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use std::borrow::Cow;
const SELECTED: PropertyIdentifier = PropertyIdentifier::from_raw(650);
struct State {
    value: PropertyValue,
    fail: bool,
    array: bool,
    reads: usize,
    changing: bool,
}
struct Probe(Arc<StdMutex<State>>);
impl BACnetObject for Probe {
    fn object_identifier(&self) -> ObjectIdentifier {
        object()
    }
    fn object_name(&self) -> &str {
        "selected probe"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(vec![PropertyIdentifier::PRESENT_VALUE, SELECTED])
    }
    fn supports_cov(&self) -> bool {
        true
    }
    fn is_array_property(&self, p: PropertyIdentifier) -> bool {
        p == SELECTED && self.0.lock().unwrap().array
    }
    fn cov_increment(&self) -> Option<f32> {
        Some(100.0)
    }
    fn read_property(&self, p: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        if p == PropertyIdentifier::PRESENT_VALUE {
            return Ok(PropertyValue::Real(10.0));
        }
        if p == SELECTED {
            let mut s = self.0.lock().unwrap();
            s.reads += 1;
            if s.fail {
                return Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::READ_ACCESS_DENIED.to_raw() as u32,
                });
            }
            if s.changing {
                return Ok(PropertyValue::Unsigned(s.reads as u64));
            }
            return Ok(s.value.clone());
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
        })
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Encoding("test read only".into()))
    }
}
async fn fixture(
    kind: CovNotificationKind,
) -> (Fixture, Arc<StdMutex<State>>, CovSubscriptionSnapshot) {
    let fixture = Fixture::new(false);
    let state = Arc::new(StdMutex::new(State {
        value: PropertyValue::Unsigned(5),
        fail: false,
        array: false,
        reads: 0,
        changing: false,
    }));
    let mut db = clocked_test_database();
    db.add(Box::new(Probe(state.clone()))).unwrap();
    *fixture.db.write().await = db;
    state.lock().unwrap().reads = 0;
    let mut sub = proposal(kind, false, SELECTED);
    sub.last_notified_observation = None;
    sub.cov_increment = None;
    let accepted = fixture.table.write().await.subscribe(sub).unwrap();
    (fixture, state, accepted)
}
fn payload(f: &Fixture, kind: CovNotificationKind) -> Vec<u8> {
    let frames = std::mem::take(&mut *f.sent.lock().unwrap());
    assert_eq!(frames.len(), 1);
    let Apdu::UnconfirmedRequest(r) =
        decode_apdu(decode_npdu(frames[0].clone()).unwrap().payload).unwrap()
    else {
        panic!()
    };
    match kind {
        CovNotificationKind::Single => {
            let n = COVNotificationRequest::decode(&r.service_request).unwrap();
            assert_eq!(n.list_of_values.len(), 1);
            assert_eq!(n.list_of_values[0].property_identifier, SELECTED);
            n.list_of_values[0].value.clone()
        }
        CovNotificationKind::Multiple => {
            let n = COVNotificationMultipleRequest::decode(&r.service_request).unwrap();
            assert_eq!(n.list_of_cov_notifications.len(), 1);
            assert_eq!(n.list_of_cov_notifications[0].list_of_values.len(), 1);
            n.list_of_cov_notifications[0].list_of_values[0]
                .value
                .clone()
        }
    }
}
fn encoded(v: PropertyValue) -> Vec<u8> {
    let mut b = BytesMut::new();
    encode_property_value(&mut b, &v).unwrap();
    b.to_vec()
}
#[tokio::test]
async fn cov_sample_one_selected_read_drives_wire_and_baseline() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let (f, s, accepted) = fixture(kind).await;
        s.lock().unwrap().changing = true;
        for (initial, n) in [(true, 1), (false, 2)] {
            f.fire(initial, std::slice::from_ref(&accepted)).await;
            assert_eq!(s.lock().unwrap().reads, n);
            assert_eq!(
                payload(&f, kind),
                encoded(PropertyValue::Unsigned(n as u64))
            );
            assert_eq!(
                f.table
                    .read()
                    .await
                    .get_subscription(accepted.key())
                    .unwrap()
                    .last_notified_observation
                    .as_ref()
                    .unwrap()
                    .sample()
                    .value(),
                &PropertyValue::Unsigned(n as u64)
            );
        }
        f.finish(false).await;
    }
}
#[tokio::test]
async fn cov_sample_failed_oversized_or_changed_shape_never_falls_back_or_advances() {
    for kind in [CovNotificationKind::Single, CovNotificationKind::Multiple] {
        let (f, s, accepted) = fixture(kind).await;
        f.fire(true, std::slice::from_ref(&accepted)).await;
        payload(&f, kind);
        let before = f
            .table
            .read()
            .await
            .get_subscription(accepted.key())
            .unwrap()
            .last_notified_observation
            .clone();
        for mode in 0..4 {
            {
                let mut s = s.lock().unwrap();
                s.fail = mode == 0;
                s.array = mode == 3;
                s.value = match mode {
                    1 => PropertyValue::List(vec![PropertyValue::List(vec![]); 1024]),
                    2 => PropertyValue::OctetString(vec![0; 65537]),
                    _ => PropertyValue::Unsigned(6),
                };
            }
            f.fire(false, &[]).await;
            assert!(f.sent.lock().unwrap().is_empty());
            assert_eq!(
                f.table
                    .read()
                    .await
                    .get_subscription(accepted.key())
                    .unwrap()
                    .last_notified_observation,
                before
            );
        }
        {
            let mut s = s.lock().unwrap();
            s.fail = false;
            s.array = false;
            s.value = PropertyValue::Unsigned(6);
        }
        f.fire(false, &[]).await;
        assert_eq!(payload(&f, kind), encoded(PropertyValue::Unsigned(6)));
        f.finish(false).await;
    }
}
#[tokio::test]
async fn cov_sample_multiple_mixed_eligibility_has_independent_payload_and_baselines() {
    let (f, s, first) = fixture(CovNotificationKind::Multiple).await;
    let mut pv = proposal(
        CovNotificationKind::Multiple,
        false,
        PropertyIdentifier::PRESENT_VALUE,
    );
    pv.last_notified_observation = None;
    pv.cov_increment = Some(100.0);
    let second = f.table.write().await.subscribe(pv).unwrap();
    f.fire(true, &[first.clone(), second.clone()]).await;
    f.sent.lock().unwrap().clear();
    s.lock().unwrap().value = PropertyValue::Unsigned(6);
    f.fire(false, &[]).await;
    assert_eq!(
        payload(&f, CovNotificationKind::Multiple),
        encoded(PropertyValue::Unsigned(6))
    );
    let table = f.table.read().await;
    assert_eq!(
        table
            .get_subscription(first.key())
            .unwrap()
            .last_notified_observation
            .as_ref()
            .unwrap()
            .sample()
            .value(),
        &PropertyValue::Unsigned(6)
    );
    assert_eq!(
        table
            .get_subscription(second.key())
            .unwrap()
            .last_notified_observation
            .as_ref()
            .unwrap()
            .sample()
            .value(),
        &PropertyValue::Real(10.0)
    );
    drop(table);
    f.finish(false).await;
}
