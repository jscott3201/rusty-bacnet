//! The synchronous WriteProperty and WritePropertyMultiple handlers end a
//! Command run they can't make, so the Command is never left in process
//! (#1178, Clause 12.10).
//!
//! CMD-1's list 1 writes AO-1 to 50.0 at priority 8, then AO-1 to 60.0 at
//! priority 9. Both start with their write-successful flags TRUE.
use super::*;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::command::CommandObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::WriteAccessSpecification;
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use bacnet_types::enums::EngineeringUnits;

fn cmd1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COMMAND, 1).unwrap()
}

fn ao1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 1).unwrap()
}

fn write_ao1(value: f32, priority: u8) -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: None,
        object_identifier: ao1(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Real(value),
        priority: Some(priority),
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    }
}

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![BACnetActionList {
            commands: vec![write_ao1(50.0, 8), write_ao1(60.0, 9)],
        }])
        .unwrap();
    db.add(Box::new(command)).unwrap();
    db
}

fn unsigned(value: u64) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &PropertyValue::Unsigned(value)).unwrap();
    encoded.to_vec()
}

fn write_pv(db: &mut ObjectDatabase, value: u64) -> Result<ObjectIdentifier, Error> {
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: cmd1(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: unsigned(value),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    handle_write_property(db, &request)
}

fn read(db: &ObjectDatabase, oid: ObjectIdentifier, property: PropertyIdentifier) -> PropertyValue {
    db.get(&oid).unwrap().read_property(property, None).unwrap()
}

/// In_Process and All_Writes_Successful, in that order.
fn state(db: &ObjectDatabase) -> (PropertyValue, PropertyValue) {
    (
        read(db, cmd1(), PropertyIdentifier::IN_PROCESS),
        read(db, cmd1(), PropertyIdentifier::ALL_WRITES_SUCCESSFUL),
    )
}

/// The write-successful flags of list 1.
fn flags(db: &ObjectDatabase) -> Vec<bool> {
    let PropertyValue::ApplicationData(bytes) = db
        .get(&cmd1())
        .unwrap()
        .read_property(PropertyIdentifier::ACTION, Some(1))
        .unwrap()
    else {
        panic!("Action element 1 is framed bytes");
    };
    let (list, _) = bacnet_encoding::constructed::decode_action_list(&bytes, 0).unwrap();
    list.commands
        .iter()
        .map(|command| command.write_successful)
        .collect()
}

fn ended_unmade(db: &ObjectDatabase) {
    let ended = (PropertyValue::Boolean(false), PropertyValue::Boolean(false));
    assert_eq!(state(db), ended, "In_Process, All_Writes_Successful");
    assert_eq!(flags(db), [false, false]);
    // Neither write was made.
    let slots = read(db, ao1(), PropertyIdentifier::PRIORITY_ARRAY);
    let PropertyValue::List(slots) = slots else {
        panic!("Priority_Array is a list: {slots:?}");
    };
    assert!(slots.iter().all(|slot| *slot == PropertyValue::Null));
}

#[test]
fn bare_write_property_ends_a_command_run_it_cannot_make() {
    let mut db = database();
    assert_eq!(write_pv(&mut db, 1).unwrap(), cmd1());
    assert_eq!(
        read(&db, cmd1(), PropertyIdentifier::PRESENT_VALUE),
        PropertyValue::Unsigned(1)
    );
    ended_unmade(&db);
    // Nothing is left running, so the next write isn't BUSY.
    write_pv(&mut db, 1).unwrap();
    ended_unmade(&db);
    write_pv(&mut db, 0).unwrap();
    assert_eq!(
        state(&db),
        (PropertyValue::Boolean(false), PropertyValue::Boolean(true))
    );
}

#[test]
fn bare_write_property_multiple_ends_the_runs_its_committed_prefix_started() {
    let mut db = database();
    // The first write starts list 1, so the second finds CMD-1 busy and the
    // request fails there with the first write committed.
    let mut request = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: cmd1(),
            list_of_properties: [1, 1]
                .into_iter()
                .map(|value| BACnetPropertyValue {
                    property_identifier: PropertyIdentifier::PRESENT_VALUE,
                    property_array_index: None,
                    value: unsigned(value),
                    priority: None,
                })
                .collect(),
        }],
    }
    .encode(&mut request)
    .unwrap();
    let error = handle_write_property_multiple(&mut db, &request).unwrap_err();
    assert!(
        matches!(error, Error::Structured { class, code, .. } | Error::Protocol { class, code }
            if class == ErrorClass::OBJECT
                && code == ErrorCode::BUSY),
        "{error:?}"
    );
    ended_unmade(&db);
    write_pv(&mut db, 1).unwrap();
    ended_unmade(&db);
}
