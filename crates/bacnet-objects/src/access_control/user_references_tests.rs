//! Access User Credentials, Members and Member_Of: BACnetLISTs of
//! BACnetDeviceObjectReference naming Access Credentials and Access Users
//! (Clauses 12.33.12 to 12.33.14, #1394).

use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;

use super::*;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// `object_type` `instance` in Device 9.
fn remote(object_type: ObjectType, instance: u32) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 9)),
        object_identifier: oid(object_type, instance),
    }
}

fn read(user: &AccessUserObject, property: P) -> PropertyValue {
    user.read_property(property, None).unwrap()
}

fn assert_value_out_of_range(result: Result<(), Error>, context: &str) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code }))
            if class == ErrorClass::PROPERTY
                && code == ErrorCode::VALUE_OUT_OF_RANGE),
        "{context}: expected PROPERTY / VALUE_OUT_OF_RANGE, got {result:?}"
    );
}

const LISTS: [P; 3] = [P::CREDENTIALS, P::MEMBERS, P::MEMBER_OF];

#[test]
fn access_user_lists_serve_device_object_references() {
    let mut user = AccessUserObject::new(1, "USER-1").unwrap();
    user.set_credentials([
        oid(ObjectType::ACCESS_CREDENTIAL, 1).into(),
        remote(ObjectType::ACCESS_CREDENTIAL, 4),
    ])
    .unwrap();
    user.set_members([oid(ObjectType::ACCESS_USER, 2)]).unwrap();
    user.set_member_of([remote(ObjectType::ACCESS_USER, 5)])
        .unwrap();
    // Object identifier [1] alone for an object in this device; device
    // identifier [0] first for one in Device 9.
    let device_9 = [0x0C, 0x02, 0x00, 0x00, 0x09];
    assert_eq!(
        read(&user, P::CREDENTIALS),
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(vec![0x1C, 0x08, 0x00, 0x00, 0x01]),
            PropertyValue::ApplicationData(
                [&device_9[..], &[0x1C, 0x08, 0x00, 0x00, 0x04]].concat()
            ),
        ])
    );
    assert_eq!(
        read(&user, P::MEMBERS),
        PropertyValue::List(vec![PropertyValue::ApplicationData(vec![
            0x1C, 0x08, 0xC0, 0x00, 0x02,
        ])])
    );
    assert_eq!(
        read(&user, P::MEMBER_OF),
        PropertyValue::List(vec![PropertyValue::ApplicationData(
            [&device_9[..], &[0x1C, 0x08, 0xC0, 0x00, 0x05]].concat()
        )])
    );
    for property in LISTS {
        assert!(user.is_list_property(property));
        assert!(!user.is_array_property(property));
        assert!(!user.is_writable_property(property));
        // Table 12-38 codes Credentials R and the other two O, with no
        // network write: a client's write of the served value is refused.
        let served = read(&user, property);
        let result = user.write_property(property, None, served.clone(), None);
        assert!(
            matches!(result, Err(Error::Remote(RemoteError::Error { code, .. }))
                if code == ErrorCode::WRITE_ACCESS_DENIED),
            "{property:?}: {result:?}"
        );
        assert_eq!(read(&user, property), served);
    }
    // An empty list clears each.
    user.set_credentials(Vec::<BACnetDeviceObjectReference>::new())
        .unwrap();
    user.set_members(Vec::<BACnetDeviceObjectReference>::new())
        .unwrap();
    user.set_member_of(Vec::<BACnetDeviceObjectReference>::new())
        .unwrap();
    for property in LISTS {
        assert_eq!(read(&user, property), PropertyValue::List(vec![]));
    }
}

#[test]
fn access_user_setters_refuse_other_objects_and_non_device_devices() {
    type Setter = fn(&mut AccessUserObject, Vec<BACnetDeviceObjectReference>) -> Result<(), Error>;
    let setters: [(&str, ObjectType, ObjectType, Setter); 3] = [
        (
            "set_credentials",
            ObjectType::ACCESS_CREDENTIAL,
            ObjectType::ACCESS_USER,
            |user, list| user.set_credentials(list),
        ),
        (
            "set_members",
            ObjectType::ACCESS_USER,
            ObjectType::ACCESS_CREDENTIAL,
            |user, list| user.set_members(list),
        ),
        (
            "set_member_of",
            ObjectType::ACCESS_USER,
            ObjectType::ACCESS_ZONE,
            |user, list| user.set_member_of(list),
        ),
    ];
    for (name, named, other, set) in setters {
        let mut user = AccessUserObject::new(1, "USER-1").unwrap();
        set(&mut user, vec![remote(named, 4)]).unwrap();
        let kept = LISTS.map(|p| read(&user, p));
        let wrong_type = [
            oid(other, 1).into(),
            remote(other, 2),
            remote(ObjectType::ANALOG_VALUE, 3),
        ];
        let not_a_device = [
            oid(ObjectType::ANALOG_VALUE, 9),
            oid(named, 9),
            oid(ObjectType::ACCESS_ZONE, ObjectIdentifier::MAX_INSTANCE),
        ]
        .map(|device| BACnetDeviceObjectReference {
            device_identifier: Some(device),
            object_identifier: oid(named, 1),
        });
        for bad in wrong_type.into_iter().chain(not_a_device) {
            // One bad reference refuses the whole list and keeps all three.
            let list = vec![oid(named, 1).into(), bad.clone()];
            assert_value_out_of_range(set(&mut user, list), &format!("{name}: {bad:?}"));
            assert_eq!(LISTS.map(|p| read(&user, p)), kept, "{name}: {bad:?}");
        }
    }
}
