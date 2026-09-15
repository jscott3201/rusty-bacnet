use super::*;
use bacnet_objects::{timer::TimerObject, traits::BACnetObject};
use bacnet_types::primitives::{Date, PropertyValue, Time};
use PropertyIdentifier as P;

#[test]
fn pics_timer_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in projection order.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::PRESENT_VALUE, false, true),
        (P::TIMER_STATE, false, false),
        (P::TIMER_RUNNING, false, false),
        (P::INITIAL_TIMEOUT, true, true),
        (P::UPDATE_TIME, false, false),
        (P::EXPIRATION_TIME, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, true, true),
        (P::RELIABILITY, false, false),
        (P::EVENT_STATE, true, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let mut object = TimerObject::new(7, "TMR-7").unwrap();
            if configured {
                object
                    .write_property(
                        P::DESCRIPTION,
                        None,
                        PropertyValue::CharacterString("long timer label".repeat(100)),
                        None,
                    )
                    .unwrap();
                object.start();
                object.set_initial_timeout(5000);
                let date = Date {
                    year: 126,
                    month: 9,
                    day: 14,
                    day_of_week: 1,
                };
                let time = Time {
                    hour: 12,
                    minute: 30,
                    second: 15,
                    hundredths: 25,
                };
                object.set_update_time(date, time);
                object.set_expiration_time(date, time);
            }
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::TIMER);
            assert!(!support.createable);
            assert!(support.deleteable);
            let rows: Vec<_> = support
                .supported_properties
                .iter()
                .map(|row| {
                    assert!(row.access.readable);
                    (row.property_id, row.access.optional, row.access.writable)
                })
                .collect();
            assert_eq!(
                rows, expected,
                "configured={configured}, OOS={out_of_service}"
            );
            assert_eq!(
                rows.iter()
                    .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                    .collect::<Vec<_>>(),
                required.as_ref()
            );
        }
    }
}
