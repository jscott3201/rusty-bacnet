//! Every Rust setter that stores a device reference, or a Command's action
//! commands, runs the shared Device check (#1308, #1285): a Device member
//! that isn't a Device identifier is PROPERTY / VALUE_OUT_OF_RANGE and
//! changes nothing, while none, or a Device, is taken.

use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList, BACnetStageLimitValue};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, EventType, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::RemoteError;

use super::*;
use crate::access_control::{
    AccessDoorObject, AccessPointObject, AccessUserObject, AccessZoneObject,
};
use crate::channel::ChannelObject;
use crate::command::CommandObject;
use crate::elevator::LiftObject;
use crate::event_enrollment::EventEnrollmentObject;
use crate::group::{GlobalGroupObject, StructuredViewObject};
use crate::life_safety::LifeSafetyPointObject;
use crate::staging::{StagingConfig, StagingObject};
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

type P = PropertyIdentifier;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn property_reference(
    object: ObjectType,
    device: Option<ObjectIdentifier>,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        object_identifier: oid(object, 1),
        property_identifier: P::PRESENT_VALUE.to_raw(),
        property_array_index: None,
        device_identifier: device,
    }
}

fn object_reference(
    object: ObjectType,
    device: Option<ObjectIdentifier>,
) -> BACnetDeviceObjectReference {
    BACnetDeviceObjectReference {
        device_identifier: device,
        object_identifier: oid(object, 1),
    }
}

/// What one setter call did: its answer, whether `property` reads the same
/// after it as before, and what it reads after it.
struct Outcome {
    result: Result<(), Error>,
    unchanged: bool,
    after: Option<PropertyValue>,
}

fn outcome<O: BACnetObject>(
    mut object: O,
    property: PropertyIdentifier,
    set: impl FnOnce(&mut O) -> Result<(), Error>,
) -> Outcome {
    let before = object.read_property(property, None).ok();
    let result = set(&mut object);
    let after = object.read_property(property, None).ok();
    Outcome {
        result,
        unchanged: after == before,
        after,
    }
}

/// The octets a read carries, every chunk joined.
fn octets(value: &PropertyValue) -> Vec<u8> {
    match value {
        PropertyValue::ApplicationData(bytes) => bytes.clone(),
        PropertyValue::List(items) => items.iter().flat_map(octets).collect(),
        _ => Vec::new(),
    }
}

/// Whether a read holds `device`'s four identifier octets.
fn holds(after: &Option<PropertyValue>, device: ObjectIdentifier) -> bool {
    let bytes = after.as_ref().map(octets).unwrap_or_default();
    bytes.windows(4).any(|window| window == device.encode())
}

/// One setter, called on a fresh object with a reference whose Device member
/// is the one given.
struct Setter {
    name: &'static str,
    set: fn(Option<ObjectIdentifier>) -> Outcome,
    /// Whether the property may name another device. One this stack holds to
    /// its own device refuses any Device member.
    remote: bool,
}

fn staging_config(device: Option<ObjectIdentifier>) -> StagingConfig {
    let stage = |limit, active| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    StagingConfig {
        present_value: 5.0,
        min_present_value: -1.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![stage(10.0, false), stage(20.0, true)],
        target_references: vec![object_reference(ObjectType::BINARY_OUTPUT, device)],
        stage_names: None,
    }
}

fn action_command(device: Option<ObjectIdentifier>) -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: device,
        object_identifier: oid(ObjectType::ANALOG_OUTPUT, 1),
        property_identifier: P::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Real(1.0),
        priority: Some(8),
        post_delay: None,
        quit_on_failure: false,
        write_successful: false,
    }
}

const SETTERS: [Setter; 18] = [
    Setter {
        name: "EventEnrollmentObject::set_object_property_reference",
        set: |device| {
            let ee = EventEnrollmentObject::new(1, "EE-1", EventType::CHANGE_OF_VALUE).unwrap();
            outcome(ee, P::OBJECT_PROPERTY_REFERENCE, |ee| {
                ee.set_object_property_reference(Some(property_reference(
                    ObjectType::ANALOG_INPUT,
                    device,
                )))
            })
        },
        remote: true,
    },
    Setter {
        name: "GlobalGroupObject::set_group_members",
        set: |device| {
            let group = GlobalGroupObject::new(1, "GG-1").unwrap();
            outcome(group, P::GROUP_MEMBERS, |group| {
                // The refusal is all or nothing: the valid first member
                // isn't kept either.
                group.set_group_members(vec![
                    property_reference(ObjectType::ANALOG_INPUT, None),
                    property_reference(ObjectType::ANALOG_VALUE, device),
                ])
            })
        },
        remote: true,
    },
    Setter {
        name: "GlobalGroupObject::add_group_member",
        set: |device| {
            let group = GlobalGroupObject::new(1, "GG-1").unwrap();
            outcome(group, P::GROUP_MEMBERS, |group| {
                group.add_group_member(property_reference(ObjectType::ANALOG_INPUT, device))
            })
        },
        remote: true,
    },
    Setter {
        name: "CommandObject::set_action",
        set: |device| {
            let command = CommandObject::new(1, "CMD-1").unwrap();
            outcome(command, P::ACTION, |command| {
                command.set_action(vec![BACnetActionList {
                    commands: vec![action_command(None), action_command(device)],
                }])
            })
        },
        remote: true,
    },
    Setter {
        name: "TrendLogObject::set_log_device_object_property",
        set: |device| {
            let log = TrendLogObject::new(1, "TL-1", 8).unwrap();
            outcome(log, P::LOG_DEVICE_OBJECT_PROPERTY, |log| {
                log.set_log_device_object_property(Some(property_reference(
                    ObjectType::ANALOG_INPUT,
                    device,
                )))
            })
        },
        remote: true,
    },
    Setter {
        name: "TrendLogMultipleObject::add_property_reference",
        set: |device| {
            let log = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
            outcome(log, P::LOG_DEVICE_OBJECT_PROPERTY, |log| {
                log.add_property_reference(property_reference(ObjectType::ANALOG_INPUT, device))
            })
        },
        remote: true,
    },
    Setter {
        name: "ChannelObject::set_members",
        set: |device| {
            let channel = ChannelObject::new(1, "CH-1", 1).unwrap();
            outcome(channel, P::LIST_OF_OBJECT_PROPERTY_REFERENCES, |channel| {
                channel.set_members(vec![property_reference(ObjectType::ANALOG_OUTPUT, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "StructuredViewObject::add_subordinate",
        set: |device| {
            let view = StructuredViewObject::new(1, "SV-1").unwrap();
            outcome(view, P::SUBORDINATE_LIST, |view| {
                view.add_subordinate(object_reference(ObjectType::ANALOG_INPUT, device), "a")
            })
        },
        remote: true,
    },
    Setter {
        name: "AccessDoorObject::set_door_members",
        set: |device| {
            let door = AccessDoorObject::new(1, "DOOR-1").unwrap();
            outcome(door, P::DOOR_MEMBERS, |door| {
                door.set_door_members([object_reference(ObjectType::BINARY_OUTPUT, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "AccessPointObject::set_access_doors",
        set: |device| {
            let point = AccessPointObject::new(1, "AP-1").unwrap();
            outcome(point, P::ACCESS_DOORS, |point| {
                point.set_access_doors([object_reference(ObjectType::ACCESS_DOOR, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "AccessZoneObject::set_entry_points",
        set: |device| {
            let zone = AccessZoneObject::new(1, "AZ-1").unwrap();
            outcome(zone, P::ENTRY_POINTS, |zone| {
                zone.set_entry_points([object_reference(ObjectType::ACCESS_POINT, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "AccessZoneObject::set_exit_points",
        set: |device| {
            let zone = AccessZoneObject::new(1, "AZ-1").unwrap();
            outcome(zone, P::EXIT_POINTS, |zone| {
                zone.set_exit_points([object_reference(ObjectType::ACCESS_POINT, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "AccessUserObject::set_credentials",
        set: |device| {
            let user = AccessUserObject::new(1, "AU-1").unwrap();
            outcome(user, P::CREDENTIALS, |user| {
                user.set_credentials([object_reference(ObjectType::ACCESS_CREDENTIAL, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "AccessUserObject::set_members",
        set: |device| {
            let user = AccessUserObject::new(1, "AU-1").unwrap();
            outcome(user, P::MEMBERS, |user| {
                user.set_members([object_reference(ObjectType::ACCESS_USER, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "AccessUserObject::set_member_of",
        set: |device| {
            let user = AccessUserObject::new(1, "AU-1").unwrap();
            outcome(user, P::MEMBER_OF, |user| {
                user.set_member_of([object_reference(ObjectType::ACCESS_USER, device)])
            })
        },
        remote: true,
    },
    Setter {
        name: "LiftObject::set_energy_meter_ref",
        set: |device| {
            let lift = LiftObject::new(1, "LIFT-1", 2).unwrap();
            outcome(lift, P::ENERGY_METER_REF, |lift| {
                lift.set_energy_meter_ref(object_reference(ObjectType::ACCUMULATOR, device))
            })
        },
        remote: true,
    },
    Setter {
        name: "LifeSafetyPointObject::add_member",
        set: |device| {
            let point = LifeSafetyPointObject::new(1, "LSP-1").unwrap();
            outcome(point, P::MEMBER_OF, |point| {
                point.add_member(object_reference(ObjectType::LIFE_SAFETY_ZONE, device))
            })
        },
        remote: true,
    },
    Setter {
        name: "StagingObject::new",
        // A refused configuration builds no object, so nothing can change.
        set: |device| match StagingObject::new(1, "STG-1", staging_config(device)) {
            Ok(object) => Outcome {
                result: Ok(()),
                unchanged: false,
                after: object.read_property(P::TARGET_REFERENCES, None).ok(),
            },
            Err(error) => Outcome {
                result: Err(error),
                unchanged: true,
                after: None,
            },
        },
        remote: false,
    },
];

fn assert_property_code(result: &Result<(), Error>, code: ErrorCode, context: &str) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code: c }))
            if *class == ErrorClass::PROPERTY && *c == code),
        "{context}: expected PROPERTY / {code:?}, got {result:?}"
    );
}

#[test]
fn every_device_reference_setter_refuses_a_non_device_and_takes_a_device() {
    for setter in &SETTERS {
        let local = (setter.set)(None);
        assert!(local.result.is_ok(), "{}: {:?}", setter.name, local.result);
        assert!(!local.unchanged, "{}: the reference is stored", setter.name);
        let device = oid(ObjectType::DEVICE, 9);
        let remote = (setter.set)(Some(device));
        if setter.remote {
            assert!(
                remote.result.is_ok(),
                "{}: {:?}",
                setter.name,
                remote.result
            );
            assert!(
                holds(&remote.after, device),
                "{}: the Device member is stored: {:?}",
                setter.name,
                remote.after
            );
        } else {
            // The property is held to this device: any Device member is
            // refused, with the code the shared local check gives.
            assert_property_code(
                &remote.result,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
                setter.name,
            );
        }
        // Another object type is no Device, the empty instance included.
        for device in [
            oid(ObjectType::ANALOG_VALUE, 9),
            oid(
                ObjectType::ANALOG_VALUE,
                ObjectIdentifier::WILDCARD_INSTANCE,
            ),
        ] {
            let context = format!("{}: device identifier {device:?}", setter.name);
            let refused = (setter.set)(Some(device));
            assert_property_code(&refused.result, ErrorCode::VALUE_OUT_OF_RANGE, &context);
            assert!(refused.unchanged, "{context}: a refusal changes nothing");
        }
    }
}
