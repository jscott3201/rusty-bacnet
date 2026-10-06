use super::*;
use bacnet_objects::analog::AnalogInputObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_services::wpm::WritePropertyMultipleRequest;

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

/// The member rows that reading Group `oid` with `references` charges to a
/// ReadPropertyMultiple work budget besides the references' own rows (#1172):
/// every member reference once per whole Present_Value selected. Counts only
/// members that name their properties, which the budget tests use.
fn group_member_rows(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    references: impl IntoIterator<Item = (PropertyIdentifier, Option<u32>)>,
) -> usize {
    if oid.object_type() != ObjectType::GROUP {
        return 0;
    }
    let Ok(PropertyValue::List(members)) = db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::LIST_OF_GROUP_MEMBERS, None)
    else {
        panic!("List_Of_Group_Members is a list");
    };
    let per_read: usize = members
        .iter()
        .map(|member| {
            let PropertyValue::ApplicationData(bytes) = member else {
                panic!("{member:?}");
            };
            let (spec, _) =
                bacnet_encoding::constructed::decode_read_access_specification(bytes, 0).unwrap();
            spec.list_of_property_references.len()
        })
        .sum();
    let reads = references
        .into_iter()
        .filter(|&(property, index)| {
            property == PropertyIdentifier::PRESENT_VALUE && index.is_none()
        })
        .count();
    reads * per_read
}

/// The class, code and First Failed Element Number an AddListElement,
/// RemoveListElement or CreateObject refusal goes out with: zero unless the
/// handler named an element of the request.
fn list_refusal(result: Result<(), Error>) -> (ErrorClass, ErrorCode, u32) {
    let (class, code, element) = match result {
        Err(Error::Protocol { class, code }) => (class, code, 0),
        Err(Error::Structured {
            class,
            code,
            detail,
        }) => match *detail {
            ErrorDetail::FirstFailedElementNumber(element) => (class, code, element),
            other => panic!("expected an element number, got {other:?}"),
        },
        other => panic!("expected a protocol refusal, got {other:?}"),
    };
    (
        ErrorClass::from_raw(class as u16),
        ErrorCode::from_raw(code as u16),
        element,
    )
}

mod access_control_arrays;
mod access_door_oos_writes;
mod access_point_authorization_writes;
mod access_required_rows;
mod access_rights_accompaniment;
mod access_rights_rule_writes;
mod access_rights_rules;
mod access_typed_values;
mod access_zone_oos_writes;
mod acknowledge_alarm;
mod acknowledge_alarm_ee;
mod alarm_summary_projection;
mod alert_enrollment;
mod array_index_gating;
mod atomic_read_file_budget;
mod atomic_write_file_budget;
mod audit_log_query;
mod audit_recipient_writes;
mod averaging_window_writes;
mod binary_lighting_operations;
mod binary_lighting_relinquish_default;
mod calendar_date_list;
mod channel_present_value_writes;
mod color_command_writes;
mod command_present_value_writes;
mod cov_multiple_admission;
mod cov_multiple_parameters;
mod cov_property_parameters;
mod cov_request_parameters;
mod create_object_creation_only;
mod create_object_default_name;
mod create_object_initial_values;
mod create_object_state_count;
mod credential_data_input_oos_writes;
mod detection_enable_summary;
mod device_description_writes;
mod device_event;
mod device_reference_reads;
mod device_reference_writes;
mod elevator_landing_calls;
mod elevator_properties;
mod enrollment_summary_budget;
mod enrollment_summary_filters;
mod enrollment_summary_recipients;
mod enrollment_summary_strict;
mod enrollment_summary_support;
mod escalator_writes;
mod file_access_method;
mod file_empty_eof;
mod file_metadata;
mod file_persistence;
mod file_storage_hook;
mod framed_properties;
mod get_event_information_projection;
mod indexed_write_presence;
mod life_safety_cov;
mod life_safety_mode_writes;
mod life_safety_oos_writes;
mod life_safety_operation;
mod life_safety_reset;
mod lighting_command_writes;
mod lighting_present_value_writes;
mod lighting_required_rows;
mod list_element_edits;
mod list_element_recipients;
mod list_element_subscriptions;
mod list_element_targets;
mod list_value_writes;
mod log_reference_writes;
mod loop_properties;
mod loop_reference_follow;
mod multi_element_writes;
mod noncommandable_null_writes;
mod noncommandable_value_source;
mod passwords;
mod property_metadata;
mod pulse_converter_input_reference;
mod pulse_converter_writes;
mod read_event_arrays;
mod read_range;
mod read_range_audit_log;
mod read_range_time;
mod read_rpm;
mod reference_writes;
mod reporting_options_writes;
mod scalar_null_writes;
mod staging_writes;
mod state_text_count;
mod tags_profile_rows;
mod trend_log_multiple_options;
mod trend_log_options;
mod undefined_property_rows;
mod value_required_rows;
mod wpm_create_alarm;
mod wpm_prefix_commit;
mod write_cov_who;
mod write_property_name;
mod write_validation;

// Isolated handler fixtures explicitly assert a standalone writer. Live ingress
// derivation is tested separately through the running server's wire dispatcher.
fn sourced_wp(db: &mut ObjectDatabase, data: &[u8]) -> Result<ObjectIdentifier, Error> {
    handle_write_property_observed(
        db,
        data,
        None,
        None,
        Some(&crate::command_source::test_origin()),
    )
    .map(|(oid, _)| oid)
}
fn sourced_wpm(db: &mut ObjectDatabase, data: &[u8]) -> Result<Vec<ObjectIdentifier>, Error> {
    let mut snapshots = crate::life_safety_cov::LifeSafetyCovSnapshots::default();
    match handle_write_property_multiple_observed(
        db,
        data,
        &mut snapshots,
        None,
        None,
        None,
        Some(&crate::command_source::test_origin()),
    ) {
        WritePropertyMultipleOutcome::Success { committed_oids } => Ok(committed_oids),
        WritePropertyMultipleOutcome::Error { error, .. } => Err(error),
        WritePropertyMultipleOutcome::Reject { reason } => Err(Error::Reject {
            reason: reason.to_raw(),
        }),
    }
}
