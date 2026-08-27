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

fn make_db_with_msi() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        bacnet_objects::multistate::MultiStateInputObject::new(1, "MSI-1", 3).unwrap(),
    ))
    .unwrap();
    db
}

fn make_db_with_device_and_ai() -> ObjectDatabase {
    let mut db = crate::server::clocked_test_database();
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

mod acknowledge_alarm_ee;
mod array_index_gating;
mod async_dcc;
mod cov_multiple_parameters;
mod detection_enable_summary;
mod device_event;
mod enrollment_summary_filters;
mod escalator_writes;
mod file_access_method;
mod file_persistence;
mod file_storage_hook;
mod framed_properties;
mod life_safety_operation;
mod multi_element_writes;
mod passwords;
mod property_metadata;
mod read_event_arrays;
mod read_rpm;
mod reference_writes;
mod wpm_create_alarm;
mod wpm_event_rollback;
mod wpm_parameter_rollback;
mod wpm_rollback_contract;
mod write_cov_who;
mod write_property_name;
mod write_validation;
