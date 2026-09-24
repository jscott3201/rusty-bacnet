use super::*;
use PropertyValue::*;
fn sample(v: PropertyValue) -> CovSample {
    CovSample::new(&v).unwrap()
}
fn reports(before: PropertyValue, after: PropertyValue, increment: Option<f32>) -> bool {
    sample(after).reports(Some(&sample(before)), increment, true)
}
#[test]
fn cov_sample_exact_integer_thresholds_and_numeric_types() {
    for base in [0, 1 << 24, 1 << 53, u64::MAX - 1] {
        assert!(reports(Unsigned(base), Unsigned(base + 1), Some(1.0)));
        assert!(!reports(Unsigned(base), Unsigned(base + 1), Some(1.01)));
    }
    assert!(!reports(
        Unsigned(0),
        Unsigned(u64::MAX),
        Some(2.0f32.powi(64))
    ));
    assert!(reports(
        Unsigned(0),
        Unsigned(u64::MAX),
        Some(f32::from_bits(2.0f32.powi(64).to_bits() - 1))
    ));
    assert!(reports(
        Signed(i32::MIN),
        Signed(i32::MAX),
        Some(4_294_967_040.0)
    ));
    assert!(!reports(
        Signed(i32::MIN),
        Signed(i32::MAX),
        Some(4_294_967_296.0)
    ));
    assert!(reports(Unsigned(0), Unsigned(1), Some(0.1)));
    assert!(!reports(Unsigned(1), Unsigned(1), None));
    assert!(reports(Unsigned(1), Signed(1), Some(f32::INFINITY)));
    assert!(reports(Null, Unsigned(0), Some(f32::NAN)));
    assert!(reports(Unsigned(0), Null, Some(f32::NAN)));
    assert!(!reports(Null, Null, Some(0.0)));
    assert!(reports(Double(1.0), Double(1.0 + f64::EPSILON), None));
    assert!(!reports(Double(1.0), Double(1.0 + f64::EPSILON), Some(0.1)));
    assert!(reports(Real(-f32::MAX), Real(f32::MAX), Some(f32::MAX)));
    assert!(reports(Double(-f64::MAX), Double(f64::MAX), Some(f32::MAX)));
}
#[test]
fn cov_sample_exceptional_thresholds_and_stable_nonfinite_values() {
    for value in [Unsigned(5), Signed(5), Real(5.0), Double(5.0)] {
        for inc in [0.0, -0.0, -1.0, f32::NEG_INFINITY] {
            assert!(reports(value.clone(), value.clone(), Some(inc)));
        }
        for inc in [f32::NAN, f32::INFINITY] {
            assert!(!reports(value.clone(), value.clone(), Some(inc)));
        }
    }
    assert!(!reports(Real(0.0), Real(-0.0), None));
    assert!(!reports(Double(0.0), Double(-0.0), None));
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(!reports(Real(v), Real(v), Some(-1.0)));
        assert!(reports(Real(1.0), Real(v), Some(f32::NAN)));
    }
    assert!(reports(
        Real(f32::from_bits(0x7fc00001)),
        Real(f32::from_bits(0x7fc00002)),
        None
    ));
    assert!(reports(
        Double(f64::NAN),
        Double(f64::INFINITY),
        Some(f32::INFINITY)
    ));
    let structured = sample(List(vec![Real(f32::NAN), Double(f64::NAN)]));
    assert!(!structured.reports(Some(&structured.clone()), Some(-1.0), false));
    assert!(sample(List(vec![Unsigned(1)])).reports(Some(&sample(Unsigned(1))), None, false));
}
fn nested(levels: usize) -> PropertyValue {
    (0..levels).fold(Null, |v, _| List(vec![v]))
}
fn resource(result: Result<CovSample, Error>) {
    assert!(
        matches!(result, Err(Error::Protocol { class,code }) if class==ErrorClass::RESOURCES.to_raw() as u32 && code==ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32)
    );
}
#[test]
fn cov_sample_retention_caps_before_encoding_or_copy() {
    assert!(CovSample::new(&nested(32)).is_ok());
    resource(CovSample::new(&nested(33)));
    assert!(CovSample::new(&List(vec![List(vec![]); 1023])).is_ok());
    resource(CovSample::new(&List(vec![List(vec![]); 1024])));
    for value in [
        OctetString(vec![0; 65536]),
        ApplicationData(vec![0; 65536]),
        CharacterString("a".repeat(65536)),
        BitString {
            unused_bits: 0,
            data: vec![0; 65536],
        },
    ] {
        assert!(CovSample::new(&value).is_ok());
        resource(CovSample::new(&List(vec![value, Boolean(true)])));
    }
    resource(CovSample::new(&OctetString(vec![0; 65537])));
    resource(CovSample::new(&List(vec![
        ApplicationData(vec![0; 32769]),
        OctetString(vec![0; 32768]),
    ])));
}
#[test]
fn cov_sample_normalizes_capacity_and_shares_accepted_storage() {
    let mut data = Vec::with_capacity(1_000_000);
    data.push(1);
    let mut list = Vec::with_capacity(4096);
    list.push(OctetString(data));
    let a = sample(List(list));
    let b = a.clone();
    assert!(Arc::ptr_eq(&a.0, &b.0));
    let List(list) = a.value() else { panic!() };
    assert_eq!(list.capacity(), 1);
    let OctetString(bytes) = &list[0] else {
        panic!()
    };
    assert_eq!(bytes.capacity(), 1);
    use crate::cov::{
        AtomicCovCounters, CovNotificationKind, CovPolicy, CovSubscription, CovSubscriptionTable,
    };
    let proposal = CovSubscription {
        subscriber_mac: bacnet_types::MacAddr::from_slice(&[1]),
        subscriber_network: None,
        subscriber_process_identifier: 1,
        monitored_object_identifier: bacnet_types::primitives::ObjectIdentifier::new(
            bacnet_types::enums::ObjectType::ANALOG_VALUE,
            1,
        )
        .unwrap(),
        issue_confirmed_notifications: false,
        expires_at: None,
        last_notified_sample: Some(a.clone()),
        monitored_property: Some(bacnet_types::enums::PropertyIdentifier::PRIORITY_ARRAY),
        monitored_property_array_index: None,
        cov_increment: None,
        notification_kind: CovNotificationKind::Single,
        timestamped: false,
    };
    let mut table = CovSubscriptionTable::with_policy(
        CovPolicy::unlimited(),
        Arc::new(AtomicCovCounters::default()),
    );
    let accepted = table.subscribe(proposal).unwrap();
    for _ in 0..10 {
        assert!(Arc::ptr_eq(
            &a.0,
            &accepted.clone().last_notified_sample.as_ref().unwrap().0
        ));
    }
    assert!(table.set_last_notified_sample(&accepted, b));
    assert!(Arc::ptr_eq(
        &a.0,
        &table
            .get_subscription(accepted.key())
            .unwrap()
            .last_notified_sample
            .as_ref()
            .unwrap()
            .0
    ));
}
