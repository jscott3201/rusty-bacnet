//! The rules an Access Door's and an Access Zone's kept event algorithm
//! parameters follow beyond being kept (#2331): each list's setter gives
//! way to a saved list until the clear, the clear drops them with the
//! rest, a type keeps only its own rows, and a request's writes are staged
//! as one save, each list checked by the object's own write path.

use bacnet_types::enums::{
    AccessZoneOccupancyState as Z, DoorAlarmState as S, ErrorClass, ErrorCode, ObjectType,
    PropertyIdentifier as P,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::PropertyValue;

use super::access_parameters_tests::{
    KINDS, access, covered, door, list_row, lists, open, states, try_open, written_list, zone,
};
use super::parameters_tests::{holding, read, saved, write};
use super::reporting_tests::refused_with;
use super::reporting_types_tests::{Guarded, each_setter_gives_way_to_its_own_row};
use super::test_storage::MemoryConfig;
use super::{AlgorithmWriteConfig, ReportingWriteConfig};
use crate::access_control::{AccessDoorObject, AccessZoneObject};
use crate::snapshot::write_config::parameters::ROWS;
use crate::traits::BACnetObject;

/// A list of `property` on a door, as a write carries it and as the setter
/// takes it.
fn door_row(
    row: P,
    over: fn(&mut AccessDoorObject),
    apply: fn(&mut AccessDoorObject),
) -> Guarded<AccessDoorObject> {
    Guarded {
        row,
        written: states(&written_list(ObjectType::ACCESS_DOOR, row)),
        over,
        apply,
        applied: states(&[S::EGRESS_OPEN.to_raw()]),
    }
}

/// Each of a door's three list setters gives way to a list a write saved to
/// its own property, on a door rebuilt on the storage, while the other two
/// still apply; after `clear_written_reporting` it applies again.
#[test]
fn each_door_list_setter_gives_way_to_its_own_saved_list_until_cleared() {
    each_setter_gives_way_to_its_own_row(
        |storage| door(Some(storage)).unwrap(),
        AccessDoorObject::clear_written_reporting,
        &[
            door_row(
                P::ALARM_VALUES,
                |door| door.set_alarm_values([S::EGRESS_OPEN]).unwrap(),
                |door| door.set_alarm_values([S::EGRESS_OPEN]).unwrap(),
            ),
            door_row(
                P::FAULT_VALUES,
                |door| door.set_fault_values([S::EGRESS_OPEN]).unwrap(),
                |door| door.set_fault_values([S::EGRESS_OPEN]).unwrap(),
            ),
            door_row(
                P::MASKED_ALARM_VALUES,
                |door| door.set_masked_alarm_values([S::EGRESS_OPEN]).unwrap(),
                |door| door.set_masked_alarm_values([S::EGRESS_OPEN]).unwrap(),
            ),
        ],
    );
}

/// A zone's `set_alarm_values` gives way the same way.
#[test]
fn a_zone_list_setter_gives_way_to_a_saved_list_until_cleared() {
    each_setter_gives_way_to_its_own_row(
        |storage| zone(Some(storage)).unwrap(),
        AccessZoneObject::clear_written_reporting,
        &[Guarded {
            row: P::ALARM_VALUES,
            written: states(&written_list(ObjectType::ACCESS_ZONE, P::ALARM_VALUES)),
            over: |zone| zone.set_alarm_values([Z::DISABLED]).unwrap(),
            apply: |zone| zone.set_alarm_values([Z::DISABLED]).unwrap(),
            applied: states(&[Z::DISABLED.to_raw()]),
        }],
    );
}

/// The setter gives way on the object that took the write too, not only on
/// one rebuilt on its storage, and goes on checking its argument: a list a
/// write would refuse is refused, kept list or not. Without storage, and
/// with storage holding no written list, it applies as it always has.
#[test]
fn a_list_setter_still_checks_what_it_does_not_take() {
    let storage = MemoryConfig::new();
    let mut kept = door(Some(&storage)).unwrap();
    for &property in lists(ObjectType::ACCESS_DOOR) {
        let list = written_list(ObjectType::ACCESS_DOOR, property);
        write(&mut kept, property, states(&list));
    }
    kept.set_alarm_values([S::EGRESS_OPEN]).unwrap();
    kept.set_fault_values([S::EGRESS_OPEN]).unwrap();
    kept.set_masked_alarm_values([S::EGRESS_OPEN]).unwrap();
    for &property in lists(ObjectType::ACCESS_DOOR) {
        let list = written_list(ObjectType::ACCESS_DOOR, property);
        assert_eq!(read(&kept, property), states(&list), "{property:?}");
    }
    assert!(kept.set_alarm_values([S::NORMAL]).is_err());
    assert!(kept.set_fault_values([S::from_raw(9)]).is_err());
    assert!(kept.set_masked_alarm_values([S::NORMAL]).is_err());

    let storage = MemoryConfig::new();
    let mut zoned = zone(Some(&storage)).unwrap();
    let list = written_list(ObjectType::ACCESS_ZONE, P::ALARM_VALUES);
    write(&mut zoned, P::ALARM_VALUES, states(&list));
    zoned.set_alarm_values([Z::DISABLED]).unwrap();
    assert_eq!(read(&zoned, P::ALARM_VALUES), states(&list));
    assert!(zoned.set_alarm_values([Z::NORMAL]).is_err());
    assert!(zoned.set_alarm_values([Z::from_raw(7)]).is_err());

    // Storage with another row written and no list: every setter applies.
    for unwritten in [None, Some(MemoryConfig::new())] {
        let mut door = door(unwritten.as_ref()).unwrap();
        write(&mut door, P::TIME_DELAY, PropertyValue::Unsigned(7));
        door.set_alarm_values([S::EGRESS_OPEN]).unwrap();
        door.set_fault_values([S::EGRESS_OPEN]).unwrap();
        door.set_masked_alarm_values([S::EGRESS_OPEN]).unwrap();
        for &property in lists(ObjectType::ACCESS_DOOR) {
            assert_eq!(read(&door, property), states(&[S::EGRESS_OPEN.to_raw()]));
        }
        let mut zone = zone(unwritten.as_ref()).unwrap();
        write(&mut zone, P::TIME_DELAY, PropertyValue::Unsigned(7));
        zone.set_alarm_values([Z::DISABLED]).unwrap();
        assert_eq!(
            read(&zone, P::ALARM_VALUES),
            states(&[Z::DISABLED.to_raw()])
        );
    }
}

/// The clear drops every parameter a type keeps with the reporting rows
/// and Description, in one save; the values served stay until the next
/// start, when what the application configures applies again. A clear
/// whose save fails keeps them all.
#[test]
fn the_clear_drops_parameters_reporting_rows_and_description_together() {
    for kind in KINDS {
        let storage = MemoryConfig::new();
        let configured = open(kind, None);
        let mut object = access(kind, Some(&storage));
        for (property, value, _) in covered(kind) {
            write(object.as_mut(), property, value);
        }
        let nine = PropertyValue::Unsigned(9);
        write(object.as_mut(), P::NOTIFICATION_CLASS, nine);
        let label = PropertyValue::CharacterString("lobby".into());
        write(object.as_mut(), P::DESCRIPTION, label);
        let kept = saved(&storage);
        assert_eq!(
            crate::snapshot::write_config::parameters::mask(&kept.algorithm),
            crate::snapshot::write_config::parameters::kept(kind),
            "{kind}: every row the type keeps"
        );
        assert_eq!(kept.notification_class, Some(9));
        assert_eq!(kept.description.as_deref(), Some("lobby"));
        drop(object);
        let saves = storage.saves();

        let mut object = access(kind, Some(&storage));
        assert_eq!(object.written(), kept, "{kind}");
        storage.fail(true);
        assert!(object.clear().is_err());
        assert_eq!(object.written(), kept, "{kind}: a failed clear keeps them");
        storage.fail(false);
        object.clear().unwrap();
        assert_eq!(object.written(), ReportingWriteConfig::default(), "{kind}");
        for (property, value, _) in covered(kind) {
            assert_eq!(read(object.as_ref(), property), value, "{kind}");
        }
        drop(object);
        assert_eq!(storage.saves(), saves + 1, "{kind}: one save clears all");
        assert_eq!(saved(&storage), ReportingWriteConfig::default(), "{kind}");
        let rebuilt = open(kind, Some(&storage));
        for (property, _, _) in covered(kind) {
            assert_eq!(
                read(rebuilt.as_ref(), property),
                read(configured.as_ref(), property),
                "{kind} {property:?}"
            );
        }
        assert_eq!(
            read(rebuilt.as_ref(), P::NOTIFICATION_CLASS),
            PropertyValue::Unsigned(0)
        );
    }
}

/// A write of each row another type keeps and `kind` doesn't, and storage
/// holding it.
fn uncovered(kind: ObjectType) -> Vec<(P, PropertyValue, AlgorithmWriteConfig)> {
    use bacnet_types::bitstring::LimitEnable;
    use bacnet_types::constructed::{BACnetEventParameter, FaultParameters};
    use bacnet_types::enums::BinaryPV;
    let none = AlgorithmWriteConfig::default;
    let mut rows = vec![
        (
            P::HIGH_LIMIT,
            PropertyValue::Real(80.0),
            AlgorithmWriteConfig {
                high_limit: Some(80.0),
                ..none()
            },
        ),
        (
            P::LOW_LIMIT,
            PropertyValue::Real(-10.0),
            AlgorithmWriteConfig {
                low_limit: Some(-10.0),
                ..none()
            },
        ),
        (
            P::DEADBAND,
            PropertyValue::Real(2.5),
            AlgorithmWriteConfig {
                deadband: Some(2.5),
                ..none()
            },
        ),
        (
            P::LIMIT_ENABLE,
            PropertyValue::BitString {
                unused_bits: 6,
                data: vec![LimitEnable::all().to_bacnet()],
            },
            AlgorithmWriteConfig {
                limit_enable: Some(LimitEnable::all()),
                ..none()
            },
        ),
        (
            P::ALARM_VALUE,
            PropertyValue::Enumerated(0),
            AlgorithmWriteConfig {
                alarm_value: Some(BinaryPV::INACTIVE),
                ..none()
            },
        ),
        // An Event Enrollment's two, each as its `none` choice.
        (
            P::EVENT_PARAMETERS,
            PropertyValue::ApplicationData(vec![0xF8, 20]),
            AlgorithmWriteConfig {
                event_parameters: Some(BACnetEventParameter::None),
                ..none()
            },
        ),
        (
            P::FAULT_PARAMETERS,
            PropertyValue::ApplicationData(vec![0x08]),
            AlgorithmWriteConfig {
                fault_parameters: Some(FaultParameters::FaultNone),
                ..none()
            },
        ),
    ];
    if kind == ObjectType::ACCESS_ZONE {
        for property in [P::FAULT_VALUES, P::MASKED_ALARM_VALUES] {
            rows.push((property, states(&[1]), list_row(property, vec![1])));
        }
    }
    // A multi-state object's State_Text (#2414).
    rows.push(super::state_parameters_tests::state_text_row());
    rows
}

/// A type keeps only its own parameters. Storage holding a row another type
/// keeps fails the build, a zone's holding one of a door's two other lists
/// included, and a write of its property is answered as without storage and
/// saves nothing. The properties these objects don't keep as parameters are
/// no rows: a door's own state and times, and a zone's occupancy limits,
/// which are read-only over the network.
#[test]
fn a_type_keeps_only_its_own_parameters() {
    for kind in KINDS {
        let rows = uncovered(kind);
        assert_eq!(rows.len() + covered(kind).len(), ROWS.len(), "{kind}");
        let storage = MemoryConfig::new();
        let mut kept = open(kind, Some(&storage));
        let mut plain = open(kind, None);
        for (property, value, stored) in rows {
            let built = try_open(kind, Some(&holding(stored)));
            assert!(matches!(built, Err(Error::Encoding(_))), "{kind}");
            let expected = plain.write_property(property, None, value.clone(), None);
            assert!(expected.is_err(), "{kind} has no {property:?}");
            let result = kept.write_property(property, None, value, None);
            assert_eq!(
                result.map_err(|error| error.to_string()),
                expected.map_err(|error| error.to_string()),
                "{kind} {property:?}"
            );
            assert_eq!(storage.saves(), 0, "{kind} {property:?}");
        }
        let never = [
            P::DOOR_ALARM_STATE,
            P::DOOR_PULSE_TIME,
            P::DOOR_OPEN_TOO_LONG_TIME,
            P::OCCUPANCY_UPPER_LIMIT,
            P::OCCUPANCY_LOWER_LIMIT,
            P::ADJUST_VALUE,
        ];
        for property in never {
            assert!(!super::reporting::kept_row(kind, property), "{kind}");
        }
    }
    // A zone's limits take no write, so there is nothing of them to keep.
    let mut zone = zone(Some(&MemoryConfig::new())).unwrap();
    for limit in [P::OCCUPANCY_UPPER_LIMIT, P::OCCUPANCY_LOWER_LIMIT] {
        let result = zone.write_property(limit, None, PropertyValue::Unsigned(9), None);
        assert!(
            refused_with(
                &result,
                ErrorClass::PROPERTY,
                ErrorCode::WRITE_ACCESS_DENIED
            ),
            "{limit:?}: {result:?}"
        );
    }
}

#[cfg(feature = "durable-writes")]
mod staged {
    use super::super::access_parameters_tests::outside;
    use super::super::parameters_tests::pending;
    use super::*;
    use crate::durable::StageStep;

    /// A request writing two parameters and a reporting row stages one
    /// save of them all, ahead of the object serving any; its writes then
    /// take their steps without saving again. When that save fails each
    /// write is refused and the old values stay.
    #[test]
    fn a_request_of_parameters_and_a_reporting_row_saves_once() {
        for kind in KINDS {
            let list = written_list(kind, P::ALARM_VALUES);
            let request = [
                pending(P::ALARM_VALUES, states(&list)),
                pending(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
                pending(P::TIME_DELAY, PropertyValue::Unsigned(7)),
            ];
            for fails in [false, true] {
                let storage = MemoryConfig::new();
                let mut object = open(kind, Some(&storage));
                let before = request
                    .each_ref()
                    .map(|write| read(object.as_ref(), write.property));
                storage.fail(fails);
                let durable = object.durable_writes().unwrap();
                let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                    panic!("{kind}: the request stages");
                };
                wait.block();
                assert_eq!(storage.saves(), usize::from(!fails), "{kind}");
                for (write, before) in request.iter().zip(&before) {
                    assert_eq!(&read(object.as_ref(), write.property), before, "{kind}");
                }
                for (write, before) in request.iter().zip(&before) {
                    let value = write.value.clone();
                    let result = object.write_property(write.property, None, value, None);
                    if fails {
                        assert!(
                            refused_with(
                                &result,
                                ErrorClass::DEVICE,
                                ErrorCode::OPERATIONAL_PROBLEM
                            ),
                            "{kind} {:?}: {result:?}",
                            write.property
                        );
                        assert_eq!(&read(object.as_ref(), write.property), before, "{kind}");
                    } else {
                        result.unwrap();
                        assert_eq!(read(object.as_ref(), write.property), write.value);
                    }
                }
                object.durable_writes().unwrap().release_staged_write(&wait);
                drop(object);
                assert_eq!(storage.saves(), usize::from(!fails), "{kind}");
                if fails {
                    assert_eq!(storage.saved(), None, "{kind}");
                    continue;
                }
                let kept = saved(&storage);
                assert_eq!(kept.notification_class, Some(9));
                let algorithm = AlgorithmWriteConfig {
                    time_delay: Some(7),
                    ..list_row(P::ALARM_VALUES, list.clone())
                };
                assert_eq!(kept.algorithm, algorithm, "{kind}");
            }
        }
    }

    /// No list's check turns on another write of the request. A door's
    /// three lists and its two delays, written in one request in any order,
    /// stage one save of all five, each list checked against the
    /// enumeration alone: the same state in all three lists is taken, the
    /// masked list first or last, and so is a list written twice.
    #[test]
    fn a_doors_lists_are_staged_whatever_the_order() {
        let tamper = states(&[S::TAMPER.to_raw()]);
        let lists = [P::MASKED_ALARM_VALUES, P::ALARM_VALUES, P::FAULT_VALUES];
        let mut orders = vec![lists.to_vec()];
        orders.push(lists.iter().rev().copied().collect());
        orders.push(vec![lists[1], lists[0], lists[2], lists[1]]);
        for order in orders {
            let mut request = vec![pending(P::TIME_DELAY_NORMAL, PropertyValue::Unsigned(3))];
            request.extend(order.iter().map(|list| pending(*list, tamper.clone())));
            request.push(pending(P::TIME_DELAY, PropertyValue::Unsigned(7)));
            let storage = MemoryConfig::new();
            let mut door = door(Some(&storage)).unwrap();
            let StageStep::Staged(wait) = door.durable_writes().unwrap().stage_writes(&request)
            else {
                panic!("{order:?}: the request stages");
            };
            wait.block();
            assert_eq!(storage.saves(), 1, "{order:?}");
            for write in &request {
                door.write_property(write.property, None, write.value.clone(), None)
                    .unwrap();
            }
            door.durable_writes().unwrap().release_staged_write(&wait);
            drop(door);
            assert_eq!(storage.saves(), 1, "{order:?}");
            let tamper = Some(vec![S::TAMPER.to_raw()]);
            let algorithm = AlgorithmWriteConfig {
                time_delay: Some(7),
                time_delay_normal: Some(3),
                alarm_values: tamper.clone(),
                fault_values: tamper.clone(),
                masked_alarm_values: tamper,
                ..Default::default()
            };
            assert_eq!(saved(&storage).algorithm, algorithm, "{order:?}");
        }
    }

    /// A staged step runs the object's own write path: it refuses what the
    /// object refuses, ending the steps, so a later write isn't saved ahead
    /// of one that will never be made. Each list is held to its own type's
    /// enumeration: a state a multi-state object would take, and one the
    /// other access type names, end the steps like NORMAL does.
    #[test]
    fn a_staged_list_the_object_refuses_ends_the_steps() {
        for kind in KINDS {
            for &list in lists(kind) {
                for refused in [0, outside(kind), 65_536] {
                    let storage = MemoryConfig::new();
                    let mut object = open(kind, Some(&storage));
                    let request = [
                        pending(P::TIME_DELAY, PropertyValue::Unsigned(7)),
                        pending(list, states(&[1, refused])),
                        pending(P::NOTIFICATION_CLASS, PropertyValue::Unsigned(9)),
                    ];
                    let durable = object.durable_writes().unwrap();
                    let StageStep::Staged(wait) = durable.stage_writes(&request) else {
                        panic!("{kind}: the request stages");
                    };
                    wait.block();
                    let expected = ReportingWriteConfig {
                        algorithm: AlgorithmWriteConfig {
                            time_delay: Some(7),
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    assert_eq!(saved(&storage), expected, "{kind} {list:?} {refused}");
                    object.durable_writes().unwrap().release_staged_write(&wait);
                }
            }
        }
    }
}
