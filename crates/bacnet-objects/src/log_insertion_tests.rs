use super::*;

#[test]
fn fallible_trend_hook_preserves_state_on_clock_error_and_retries() {
    for kind in [FamilyKind::Trend, FamilyKind::TrendMultiple] {
        for clock in [None, Some(TestClock::invalid())] {
            let mut object = kind.object(2);
            object
                .object_mut()
                .bind_clock_internal(clock.map(|clock| clock as Arc<dyn ClockReader>));
            object
                .write(
                    PropertyIdentifier::STOP_WHEN_FULL,
                    PropertyValue::Boolean(true),
                )
                .unwrap();
            object
                .object_mut()
                .add_trend_record(ordinary(1, 10))
                .unwrap();
            let records = object.records().clone();
            let identities = object.identities();
            let total = object.total();
            assert_protocol(
                object
                    .object_mut()
                    .add_trend_record(ordinary(2, 20))
                    .unwrap_err(),
                ErrorClass::DEVICE,
                ErrorCode::OPERATIONAL_PROBLEM,
            );
            assert_eq!(object.records(), &records);
            assert_eq!(object.identities(), identities);
            assert_eq!(object.total(), total);
            assert!(object.enabled());

            object.bind_clock(TestClock::valid());
            object
                .object_mut()
                .add_trend_record(ordinary(2, 20))
                .unwrap();
            assert!(!object.enabled());
            assert_eq!(object.total(), total + 1);
            assert_eq!(object.records().len(), 2);
            assert_eq!(object.records()[0], records[0]);
            assert_eq!(object.identities()[0], identities[0]);
            assert_status(&object, LOG_DISABLED);

            object.object_mut().bind_clock_internal(None);
            let before = object.records().clone();
            let identities = object.identities();
            object
                .object_mut()
                .add_trend_record(ordinary(3, 30))
                .unwrap();
            assert_eq!(
                object.records(),
                &before,
                "disabled insertion succeeds unchanged"
            );
            assert_eq!(object.identities(), identities);
            assert_eq!(object.total(), total + 1);
        }
    }
}

#[test]
fn fallible_insertion_keeps_count_only_success_and_rejects_unsupported_objects() {
    for kind in FamilyKind::ALL {
        let mut object = kind.object(0);
        object.add_record(ordinary(1, 10)).unwrap();
        assert!(object.records().is_empty());
        assert!(object.identities().is_empty());
        assert_eq!(object.total(), 1);
        if !matches!(kind, FamilyKind::Event) {
            object
                .object_mut()
                .add_trend_record(ordinary(2, 20))
                .unwrap();
            assert!(object.records().is_empty());
            assert!(object.identities().is_empty());
            assert_eq!(object.total(), 2);
        }
    }
    let mut unsupported = crate::analog::AnalogInputObject::new(1, "AI", 62).unwrap();
    assert_protocol(
        unsupported.add_trend_record(ordinary(1, 10)).unwrap_err(),
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    );
}
