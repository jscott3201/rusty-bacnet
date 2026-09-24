use super::*;
use crate::cov::*;
use bacnet_objects::{
    accumulator::AccumulatorObject, analog::AnalogValueObject, color::ColorObject,
    database::ObjectDatabase,
};
use bacnet_services::{
    common::PropertyReference,
    cov::SubscribeCOVPropertyRequest,
    cov_multiple::{
        COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
    },
};
use bacnet_types::{
    constructed::{BACnetPrescale, BACnetScale},
    primitives::ObjectIdentifier,
};
use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};
fn prepared(
    object: &dyn BACnetObject,
    p: PropertyIdentifier,
    i: Option<u32>,
    inc: Option<f32>,
) -> PreparedCovValue {
    prepare_value(object, p, i, inc, &object.read_property(p, i).unwrap()).unwrap()
}
#[test]
fn cov_sample_builtin_array_coordinates_and_pv_threshold_inheritance() {
    let mut av = AnalogValueObject::new(1, "value", 95).unwrap();
    av.write_property(
        PropertyIdentifier::COV_INCREMENT,
        None,
        PropertyValue::Real(5.0),
        None,
    )
    .unwrap();
    let initial = prepared(&av, PropertyIdentifier::PRESENT_VALUE, None, None);
    av.set_present_value(1.0);
    assert!(
        !prepared(&av, PropertyIdentifier::PRESENT_VALUE, None, None)
            .reports(Some(&initial.sample))
    );
    assert!(
        prepared(&av, PropertyIdentifier::PRESENT_VALUE, None, Some(0.5))
            .reports(Some(&initial.sample))
    );
    let initial = prepared(&av, PropertyIdentifier::RELINQUISH_DEFAULT, None, None);
    av.write_property(
        PropertyIdentifier::RELINQUISH_DEFAULT,
        None,
        PropertyValue::Real(0.1),
        None,
    )
    .unwrap();
    assert!(
        prepared(&av, PropertyIdentifier::RELINQUISH_DEFAULT, None, None)
            .reports(Some(&initial.sample))
    );
    assert!(
        !prepared(&av, PropertyIdentifier::RELINQUISH_DEFAULT, None, Some(1.0))
            .reports(Some(&initial.sample))
    );
    let count = prepared(&av, PropertyIdentifier::PRIORITY_ARRAY, Some(0), Some(-1.0));
    assert!(!count.reports(Some(&count.sample)));
    let changed_count = prepare_value(
        &av,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(0),
        Some(f32::INFINITY),
        &PropertyValue::Unsigned(17),
    )
    .unwrap();
    assert!(changed_count.reports(Some(&count.sample)));
    let empty = prepared(
        &av,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        Some(f32::INFINITY),
    );
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(10.0),
        Some(8),
    )
    .unwrap();
    let value = prepared(
        &av,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        Some(f32::INFINITY),
    );
    assert!(value.reports(Some(&empty.sample)));
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(11.0),
        Some(8),
    )
    .unwrap();
    assert!(
        !prepared(&av, PropertyIdentifier::PRIORITY_ARRAY, Some(8), Some(2.0))
            .reports(Some(&value.sample))
    );
    assert!(
        prepared(&av, PropertyIdentifier::PRIORITY_ARRAY, Some(8), Some(1.0))
            .reports(Some(&value.sample))
    );
    for priority in 1..=16 {
        av.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(10.0),
            Some(priority),
        )
        .unwrap();
    }
    let full = prepared(
        &av,
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        Some(f32::NAN),
    );
    assert!(
        matches!(full.sample.value(),PropertyValue::List(v) if v.iter().all(|v|matches!(v,PropertyValue::Real(10.0))))
    );
    assert!(!full.reports(Some(&full.sample)));
    av.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(10.1),
        Some(16),
    )
    .unwrap();
    assert!(prepared(
        &av,
        PropertyIdentifier::PRIORITY_ARRAY,
        None,
        Some(f32::INFINITY)
    )
    .reports(Some(&full.sample)));
}
#[test]
fn cov_sample_structured_color_and_accumulator_ignore_increment() {
    let mut color = ColorObject::new(1, "color").unwrap();
    let first = prepared(&color, PropertyIdentifier::PRESENT_VALUE, None, Some(-1.0));
    assert!(!first.reports(Some(&first.sample)));
    color.set_present_value(0.1, 0.2);
    assert!(prepared(
        &color,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        Some(f32::INFINITY)
    )
    .reports(Some(&first.sample)));
    let mut accumulator = AccumulatorObject::new(1, "acc", 95).unwrap();
    accumulator.set_prescale(BACnetPrescale {
        multiplier: 1,
        modulo_divide: 2,
    });
    let first = prepared(&accumulator, PropertyIdentifier::PRESCALE, None, Some(0.0));
    accumulator.set_prescale(BACnetPrescale {
        multiplier: 2,
        modulo_divide: 2,
    });
    assert!(prepared(
        &accumulator,
        PropertyIdentifier::PRESCALE,
        None,
        Some(f32::NAN)
    )
    .reports(Some(&first.sample)));
    let first = prepared(
        &accumulator,
        PropertyIdentifier::SCALE,
        None,
        Some(f32::INFINITY),
    );
    accumulator.set_scale(BACnetScale::FloatScale(2.0));
    assert!(prepared(
        &accumulator,
        PropertyIdentifier::SCALE,
        None,
        Some(f32::INFINITY)
    )
    .reports(Some(&first.sample)));
}
const CUSTOM: PropertyIdentifier = PropertyIdentifier::from_raw(650);
const VARIABLE: PropertyIdentifier = PropertyIdentifier::from_raw(651);
struct Custom(Arc<Mutex<PropertyValue>>);
impl BACnetObject for Custom {
    fn object_identifier(&self) -> ObjectIdentifier {
        ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap()
    }
    fn object_name(&self) -> &str {
        "custom"
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(vec![PropertyIdentifier::PRESENT_VALUE, CUSTOM, VARIABLE])
    }
    fn supports_cov(&self) -> bool {
        true
    }
    fn is_array_property(&self, p: PropertyIdentifier) -> bool {
        p == CUSTOM
    }
    fn read_property(&self, p: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        if p == PropertyIdentifier::PRESENT_VALUE {
            Ok(PropertyValue::Unsigned(1))
        } else if p == VARIABLE {
            Ok(self.0.lock().unwrap().clone())
        } else if p == CUSTOM {
            Ok(PropertyValue::List(vec![PropertyValue::Unsigned(1)]))
        } else {
            Err(property_error(ErrorCode::UNKNOWN_PROPERTY))
        }
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(property_error(ErrorCode::WRITE_ACCESS_DENIED))
    }
}
fn single(p: PropertyIdentifier, index: Option<u32>) -> BytesMut {
    let mut b = BytesMut::new();
    SubscribeCOVPropertyRequest {
        subscriber_process_identifier: 1,
        monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        issue_confirmed_notifications: Some(false),
        lifetime: Some(300),
        monitored_property_identifier: p,
        monitored_property_array_index: index,
        cov_increment: None,
    }
    .encode(&mut b)
    .unwrap();
    b
}
#[test]
fn cov_sample_admission_coordinate_and_size_failure_preserve_entire_context() {
    let value = Arc::new(Mutex::new(PropertyValue::Unsigned(1)));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(Custom(value.clone()))).unwrap();
    let mut table = CovSubscriptionTable::with_policy(
        CovPolicy::unlimited(),
        Arc::new(AtomicCovCounters::default()),
    );
    let request = |properties: Vec<(PropertyIdentifier, Option<f32>)>, lifetime| {
        SubscribeCOVPropertyMultipleRequest {
            subscriber_process_identifier: 1,
            issue_confirmed_notifications: false,
            lifetime: Some(lifetime),
            max_notification_delay: Some(0),
            list_of_cov_subscription_specifications: vec![COVSubscriptionSpecification {
                monitored_object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1)
                    .unwrap(),
                list_of_cov_references: properties
                    .into_iter()
                    .map(|(property, inc)| COVReference {
                        monitored_property: PropertyReference {
                            property_identifier: property,
                            property_array_index: None,
                        },
                        cov_increment: inc,
                        timestamped: false,
                    })
                    .collect(),
            }],
        }
    };
    let first = crate::handlers::handle_subscribe_cov_property_multiple_request_endpoint(
        &mut table,
        &db,
        &[1],
        None,
        request(vec![(PropertyIdentifier::PRESENT_VALUE, None)], 300),
    )
    .unwrap()
    .remove(0);
    for increment in [None, Some(0.5)] {
        let error = crate::handlers::handle_subscribe_cov_property_multiple_request_endpoint(
            &mut table,
            &db,
            &[1],
            None,
            request(
                vec![
                    (PropertyIdentifier::PRESENT_VALUE, None),
                    (CUSTOM, increment),
                ],
                600,
            ),
        )
        .unwrap_err();
        assert!(
            matches!(error,Error::Protocol{class,code} if class==ErrorClass::PROPERTY.to_raw() as u32 && code==ErrorCode::NOT_COV_PROPERTY.to_raw() as u32)
        );
        assert!(table.is_current(&first));
        assert_eq!(
            table.get_subscription(first.key()).unwrap().expires_at,
            first.expires_at
        );
        assert_eq!(table.len(), 1);
    }
    for index in [0, 1] {
        let error = crate::handlers::handle_subscribe_cov_property(
            &mut table,
            &db,
            &[1],
            &single(PropertyIdentifier::PRESENT_VALUE, Some(index)),
        )
        .unwrap_err();
        assert!(
            matches!(error,Error::Protocol{code,..} if code==ErrorCode::PROPERTY_IS_NOT_AN_ARRAY.to_raw() as u32)
        );
    }
    let error = crate::handlers::handle_subscribe_cov_property(
        &mut table,
        &db,
        &[1],
        &single(PropertyIdentifier::from_raw(999), Some(0)),
    )
    .unwrap_err();
    assert!(
        matches!(error,Error::Protocol{code,..} if code==ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32)
    );
    *value.lock().unwrap() = PropertyValue::List(vec![PropertyValue::List(vec![]); 1024]);
    let error = crate::handlers::handle_subscribe_cov_property_multiple_request_endpoint(
        &mut table,
        &db,
        &[1],
        None,
        request(
            vec![(PropertyIdentifier::PRESENT_VALUE, None), (VARIABLE, None)],
            600,
        ),
    )
    .unwrap_err();
    assert!(
        matches!(error,Error::Protocol{class,code} if class==ErrorClass::RESOURCES.to_raw() as u32 && code==ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT.to_raw() as u32)
    );
    let error = crate::handlers::handle_subscribe_cov_property(
        &mut table,
        &db,
        &[1],
        &single(VARIABLE, None),
    )
    .unwrap_err();
    assert!(
        matches!(error,Error::Protocol{class,..} if class==ErrorClass::RESOURCES.to_raw() as u32)
    );
    assert!(table.is_current(&first));
    assert_eq!(
        table.get_subscription(first.key()).unwrap().expires_at,
        first.expires_at
    );
    assert_eq!(table.len(), 1);
}
