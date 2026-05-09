use super::*;

// --- AccessDoorObject ---

#[test]
fn access_door_create_and_read_defaults() {
    let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    assert_eq!(door.object_name(), "DOOR-1");
    assert_eq!(
        door.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // closed
    );
}

#[test]
fn access_door_object_type() {
    let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    assert_eq!(
        door.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ACCESS_DOOR.to_raw())
    );
}

#[test]
fn access_door_property_list() {
    let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    let list = door.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::DOOR_STATUS));
    assert!(list.contains(&PropertyIdentifier::LOCK_STATUS));
    assert!(list.contains(&PropertyIdentifier::SECURED_STATUS));
    assert!(list.contains(&PropertyIdentifier::DOOR_ALARM_STATE));
    assert!(list.contains(&PropertyIdentifier::DOOR_MEMBERS));
}

#[test]
fn access_door_read_door_members_empty() {
    let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    assert_eq!(
        door.read_property(PropertyIdentifier::DOOR_MEMBERS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn access_door_write_present_value() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    // Must be out-of-service to write present value
    door.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    door.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1), // opened
        None,
    )
    .unwrap();
    assert_eq!(
        door.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
}

#[test]
fn access_door_write_present_value_commandable() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    // AccessDoor is commandable — writing PV with priority should succeed
    let result = door.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1), // opened
        Some(16),
    );
    assert!(result.is_ok());
    // Verify PV changed
    let pv = door
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Enumerated(1));
    // Relinquish — write NULL
    let result = door.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Null,
        Some(16),
    );
    assert!(result.is_ok());
    // PV should revert to relinquish default (0 = closed)
    let pv = door
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap();
    assert_eq!(pv, PropertyValue::Enumerated(0));
}

#[test]
fn access_door_write_present_value_wrong_type() {
    let mut door = AccessDoorObject::new(1, "DOOR-1").unwrap();
    door.write_property(
        PropertyIdentifier::OUT_OF_SERVICE,
        None,
        PropertyValue::Boolean(true),
        None,
    )
    .unwrap();
    let result = door.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Real(1.0),
        None,
    );
    assert!(result.is_err());
}

// --- AccessCredentialObject ---

#[test]
fn access_credential_create_and_read_defaults() {
    let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
    assert_eq!(cred.object_name(), "CRED-1");
    assert_eq!(
        cred.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // inactive
    );
}

#[test]
fn access_credential_object_type() {
    let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
    assert_eq!(
        cred.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ACCESS_CREDENTIAL.to_raw())
    );
}

#[test]
fn access_credential_property_list() {
    let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
    let list = cred.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::CREDENTIAL_STATUS));
    assert!(list.contains(&PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS));
    assert!(list.contains(&PropertyIdentifier::AUTHENTICATION_FACTORS));
}

#[test]
fn access_credential_read_assigned_access_rights() {
    let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
    assert_eq!(
        cred.read_property(PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn access_credential_read_authentication_factors() {
    let cred = AccessCredentialObject::new(1, "CRED-1").unwrap();
    assert_eq!(
        cred.read_property(PropertyIdentifier::AUTHENTICATION_FACTORS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

// --- AccessPointObject ---

#[test]
fn access_point_create_and_read_defaults() {
    let point = AccessPointObject::new(1, "AP-1").unwrap();
    assert_eq!(point.object_name(), "AP-1");
    assert_eq!(
        point
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn access_point_object_type() {
    let point = AccessPointObject::new(1, "AP-1").unwrap();
    assert_eq!(
        point
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ACCESS_POINT.to_raw())
    );
}

#[test]
fn access_point_property_list() {
    let point = AccessPointObject::new(1, "AP-1").unwrap();
    let list = point.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::ACCESS_EVENT));
    assert!(list.contains(&PropertyIdentifier::ACCESS_EVENT_TAG));
    assert!(list.contains(&PropertyIdentifier::ACCESS_EVENT_TIME));
    assert!(list.contains(&PropertyIdentifier::ACCESS_DOORS));
    assert!(list.contains(&PropertyIdentifier::EVENT_STATE));
}

#[test]
fn access_point_read_access_event_time() {
    let point = AccessPointObject::new(1, "AP-1").unwrap();
    let val = point
        .read_property(PropertyIdentifier::ACCESS_EVENT_TIME, None)
        .unwrap();
    match val {
        PropertyValue::List(items) => {
            assert_eq!(items.len(), 2);
        }
        other => panic!("expected List, got {other:?}"),
    }
}

#[test]
fn access_point_read_access_doors_empty() {
    let point = AccessPointObject::new(1, "AP-1").unwrap();
    assert_eq!(
        point
            .read_property(PropertyIdentifier::ACCESS_DOORS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

// --- AccessRightsObject ---

#[test]
fn access_rights_create_and_read_defaults() {
    let rights = AccessRightsObject::new(1, "AR-1").unwrap();
    assert_eq!(rights.object_name(), "AR-1");
    assert_eq!(
        rights
            .read_property(PropertyIdentifier::GLOBAL_IDENTIFIER, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn access_rights_object_type() {
    let rights = AccessRightsObject::new(1, "AR-1").unwrap();
    assert_eq!(
        rights
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ACCESS_RIGHTS.to_raw())
    );
}

#[test]
fn access_rights_property_list() {
    let rights = AccessRightsObject::new(1, "AR-1").unwrap();
    let list = rights.property_list();
    assert!(list.contains(&PropertyIdentifier::GLOBAL_IDENTIFIER));
    assert!(list.contains(&PropertyIdentifier::POSITIVE_ACCESS_RULES));
    assert!(list.contains(&PropertyIdentifier::NEGATIVE_ACCESS_RULES));
}

#[test]
fn access_rights_read_rules_counts() {
    let rights = AccessRightsObject::new(1, "AR-1").unwrap();
    assert_eq!(
        rights
            .read_property(PropertyIdentifier::POSITIVE_ACCESS_RULES, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(
        rights
            .read_property(PropertyIdentifier::NEGATIVE_ACCESS_RULES, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn access_rights_write_global_identifier() {
    let mut rights = AccessRightsObject::new(1, "AR-1").unwrap();
    rights
        .write_property(
            PropertyIdentifier::GLOBAL_IDENTIFIER,
            None,
            PropertyValue::Unsigned(42),
            None,
        )
        .unwrap();
    assert_eq!(
        rights
            .read_property(PropertyIdentifier::GLOBAL_IDENTIFIER, None)
            .unwrap(),
        PropertyValue::Unsigned(42)
    );
}

// --- AccessUserObject ---

#[test]
fn access_user_create_and_read_defaults() {
    let user = AccessUserObject::new(1, "USER-1").unwrap();
    assert_eq!(user.object_name(), "USER-1");
    assert_eq!(
        user.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn access_user_object_type() {
    let user = AccessUserObject::new(1, "USER-1").unwrap();
    assert_eq!(
        user.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ACCESS_USER.to_raw())
    );
}

#[test]
fn access_user_property_list() {
    let user = AccessUserObject::new(1, "USER-1").unwrap();
    let list = user.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::USER_TYPE));
    assert!(list.contains(&PropertyIdentifier::CREDENTIALS));
    assert!(list.contains(&PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS));
}

#[test]
fn access_user_read_credentials_empty() {
    let user = AccessUserObject::new(1, "USER-1").unwrap();
    assert_eq!(
        user.read_property(PropertyIdentifier::CREDENTIALS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn access_user_write_user_type() {
    let mut user = AccessUserObject::new(1, "USER-1").unwrap();
    user.write_property(
        PropertyIdentifier::USER_TYPE,
        None,
        PropertyValue::Enumerated(1),
        None,
    )
    .unwrap();
    assert_eq!(
        user.read_property(PropertyIdentifier::USER_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(1)
    );
}

// --- AccessZoneObject ---

#[test]
fn access_zone_create_and_read_defaults() {
    let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    assert_eq!(zone.object_name(), "ZONE-1");
    assert_eq!(
        zone.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0)
    );
}

#[test]
fn access_zone_object_type() {
    let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    assert_eq!(
        zone.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::ACCESS_ZONE.to_raw())
    );
}

#[test]
fn access_zone_property_list() {
    let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    let list = zone.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::GLOBAL_IDENTIFIER));
    assert!(list.contains(&PropertyIdentifier::OCCUPANCY_COUNT));
    assert!(list.contains(&PropertyIdentifier::ACCESS_DOORS));
    assert!(list.contains(&PropertyIdentifier::ENTRY_POINTS));
    assert!(list.contains(&PropertyIdentifier::EXIT_POINTS));
}

#[test]
fn access_zone_read_lists_empty() {
    let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    assert_eq!(
        zone.read_property(PropertyIdentifier::ACCESS_DOORS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
    assert_eq!(
        zone.read_property(PropertyIdentifier::ENTRY_POINTS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
    assert_eq!(
        zone.read_property(PropertyIdentifier::EXIT_POINTS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn access_zone_read_occupancy_count() {
    let zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    assert_eq!(
        zone.read_property(PropertyIdentifier::OCCUPANCY_COUNT, None)
            .unwrap(),
        PropertyValue::Unsigned(0)
    );
}

#[test]
fn access_zone_write_global_identifier() {
    let mut zone = AccessZoneObject::new(1, "ZONE-1").unwrap();
    zone.write_property(
        PropertyIdentifier::GLOBAL_IDENTIFIER,
        None,
        PropertyValue::Unsigned(99),
        None,
    )
    .unwrap();
    assert_eq!(
        zone.read_property(PropertyIdentifier::GLOBAL_IDENTIFIER, None)
            .unwrap(),
        PropertyValue::Unsigned(99)
    );
}

// --- CredentialDataInputObject ---

#[test]
fn credential_data_input_create_and_read_defaults() {
    let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    assert_eq!(cdi.object_name(), "CDI-1");
    assert_eq!(
        cdi.read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap(),
        PropertyValue::Enumerated(0) // notReady
    );
}

#[test]
fn credential_data_input_object_type() {
    let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    assert_eq!(
        cdi.read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap(),
        PropertyValue::Enumerated(ObjectType::CREDENTIAL_DATA_INPUT.to_raw())
    );
}

#[test]
fn credential_data_input_property_list() {
    let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    let list = cdi.property_list();
    assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
    assert!(list.contains(&PropertyIdentifier::UPDATE_TIME));
    assert!(list.contains(&PropertyIdentifier::SUPPORTED_FORMATS));
    assert!(list.contains(&PropertyIdentifier::SUPPORTED_FORMAT_CLASSES));
}

#[test]
fn credential_data_input_read_update_time() {
    let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    let val = cdi
        .read_property(PropertyIdentifier::UPDATE_TIME, None)
        .unwrap();
    match val {
        PropertyValue::List(items) => {
            assert_eq!(items.len(), 2);
        }
        other => panic!("expected List, got {other:?}"),
    }
}

#[test]
fn credential_data_input_read_supported_formats_empty() {
    let cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    assert_eq!(
        cdi.read_property(PropertyIdentifier::SUPPORTED_FORMATS, None)
            .unwrap(),
        PropertyValue::List(vec![])
    );
}

#[test]
fn credential_data_input_write_denied() {
    let mut cdi = CredentialDataInputObject::new(1, "CDI-1").unwrap();
    let result = cdi.write_property(
        PropertyIdentifier::PRESENT_VALUE,
        None,
        PropertyValue::Enumerated(1),
        None,
    );
    assert!(result.is_err());
}
