//! Elevator_Group, Group_ID and Installation_ID on the Lift (Clause 12.59,
//! Table 12-77; #1021) and the Escalator (Clause 12.60, Table 12-78; #1022):
//! required rows of the table datatypes, read-only over the network and set
//! by the application.

use super::super::*;
use super::assert_value_out_of_range;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::error::RemoteError;

const ELEVATOR_GROUP: PropertyIdentifier = PropertyIdentifier::ELEVATOR_GROUP;
const GROUP_ID: PropertyIdentifier = PropertyIdentifier::GROUP_ID;
const INSTALLATION_ID: PropertyIdentifier = PropertyIdentifier::INSTALLATION_ID;

/// The membership API both objects share, so each test runs on both.
trait Member: BACnetObject {
    fn elevator_group(&self) -> ObjectIdentifier;
    fn set_elevator_group(&mut self, oid: ObjectIdentifier) -> Result<(), Error>;
    fn ids(&self) -> (u8, u8);
    fn set_ids(&mut self, group_id: u8, installation_id: u8);
}

macro_rules! impl_member {
    ($object:ty) => {
        impl Member for $object {
            fn elevator_group(&self) -> ObjectIdentifier {
                <$object>::elevator_group(self)
            }
            fn set_elevator_group(&mut self, oid: ObjectIdentifier) -> Result<(), Error> {
                <$object>::set_elevator_group(self, oid)
            }
            fn ids(&self) -> (u8, u8) {
                (self.group_id(), self.installation_id())
            }
            fn set_ids(&mut self, group_id: u8, installation_id: u8) {
                self.set_group_id(group_id);
                self.set_installation_id(installation_id);
            }
        }
    };
}
impl_member!(LiftObject);
impl_member!(EscalatorObject);

fn members() -> [Box<dyn Member>; 2] {
    [
        Box::new(LiftObject::new(1, "LIFT-1", 3).unwrap()),
        Box::new(EscalatorObject::new(1, "ESC-1").unwrap()),
    ]
}

fn group(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ELEVATOR_GROUP, instance).unwrap()
}

fn assert_write_access_denied(result: Result<(), Error>, context: &str) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => {
            assert_eq!(class, ErrorClass::PROPERTY, "{context}");
            assert_eq!(code, ErrorCode::WRITE_ACCESS_DENIED, "{context}");
        }
        other => panic!("{context}: expected PROPERTY/WRITE_ACCESS_DENIED, got {other:?}"),
    }
}

#[test]
fn membership_rows_are_required_and_start_ungrouped() {
    for object in members() {
        let kind = object.object_identifier().object_type();
        // No Elevator Group lists a new object: instance 4194303.
        let none = group(ObjectIdentifier::MAX_INSTANCE);
        assert_eq!(object.elevator_group(), none, "{kind:?}");
        assert_eq!(object.ids(), (0, 0), "{kind:?}");
        for (property, value) in [
            (ELEVATOR_GROUP, PropertyValue::ObjectIdentifier(none)),
            (GROUP_ID, PropertyValue::Unsigned(0)),
            (INSTALLATION_ID, PropertyValue::Unsigned(0)),
        ] {
            assert_eq!(
                object.read_property(property, None).unwrap(),
                value,
                "{kind:?} {property:?}"
            );
            assert!(object.property_list().contains(&property));
            assert!(object.required_properties().contains(&property));
        }
    }
}

#[test]
fn membership_setters_store_the_unsigned8_range_and_elevator_groups() {
    for mut object in members() {
        for (group_id, installation_id) in [(255, 0), (0, 255), (47, 3)] {
            object.set_ids(group_id, installation_id);
            assert_eq!(object.ids(), (group_id, installation_id));
            assert_eq!(
                object.read_property(GROUP_ID, None).unwrap(),
                PropertyValue::Unsigned(group_id.into())
            );
            assert_eq!(
                object.read_property(INSTALLATION_ID, None).unwrap(),
                PropertyValue::Unsigned(installation_id.into())
            );
        }
        object.set_elevator_group(group(9)).unwrap();
        assert_eq!(
            object.read_property(ELEVATOR_GROUP, None).unwrap(),
            PropertyValue::ObjectIdentifier(group(9))
        );
    }
}

#[test]
fn set_elevator_group_takes_only_elevator_group_objects() {
    for mut object in members() {
        object.set_elevator_group(group(2)).unwrap();
        for oid in [
            ObjectIdentifier::new(ObjectType::LIFT, 2).unwrap(),
            ObjectIdentifier::new(ObjectType::ESCALATOR, 2).unwrap(),
            ObjectIdentifier::new(ObjectType::POSITIVE_INTEGER_VALUE, 2).unwrap(),
        ] {
            assert_value_out_of_range(
                object.set_elevator_group(oid),
                &format!("{:?}", oid.object_type()),
            );
            assert_eq!(object.elevator_group(), group(2));
        }
    }
}

#[test]
fn membership_rows_are_read_only_over_the_network() {
    for mut object in members() {
        object.set_elevator_group(group(4)).unwrap();
        object.set_ids(7, 2);
        for (property, value) in [
            (ELEVATOR_GROUP, PropertyValue::ObjectIdentifier(group(5))),
            (GROUP_ID, PropertyValue::Unsigned(8)),
            (GROUP_ID, PropertyValue::Unsigned(256)),
            (INSTALLATION_ID, PropertyValue::Unsigned(3)),
            (INSTALLATION_ID, PropertyValue::Enumerated(3)),
        ] {
            assert!(!object.is_writable_property(property), "{property:?}");
            assert_write_access_denied(
                object.write_property(property, None, value, None),
                &format!("{property:?}"),
            );
        }
        assert_eq!(object.elevator_group(), group(4));
        assert_eq!(object.ids(), (7, 2));
    }
}
