use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::traits::BACnetObject;

fn make_db_with_ai() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut ai = AnalogInputObject::new(1, "AI-1", 62).unwrap();
    ai.set_present_value(72.5);
    db.add(Box::new(ai)).unwrap();
    db
}

fn make_db_with_device_and_ai() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let device = bacnet_objects::device::DeviceObject::new(bacnet_objects::device::DeviceConfig {
        instance: 1,
        name: "TestDevice".into(),
        ..Default::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(AnalogInputObject::new(1, "AI-1", 62).unwrap()))
        .unwrap();
    db
}

mod async_dcc;
mod device_event;
mod passwords;
mod read_rpm;
mod wpm_create_alarm;
mod write_cov_who;
