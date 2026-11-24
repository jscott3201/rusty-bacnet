//! A record's run-time state and details (#2620): Event_State and
//! Reliability with values, Device_Type and Profile_Name with details, and
//! the priority in control, an output's Current_Command_Priority or a
//! value object's worked out from its Priority_Array. Each judged by its
//! datatype from rows, and read from a device by ReadPropertyMultiple and
//! by ReadProperty, a device that lacks the optional ones failing nothing.

use bacnet_objects::access_control::AccessDoorObject;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject};
use bacnet_objects::command_source::CommandOrigin;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::lighting::{BinaryLightingOutputObject, LightingOutputObject};
use bacnet_objects::multistate::MultiStateOutputObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_objects::value_types::IntegerValueObject;
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::primitives::PropertyValue as V;

use super::record::record;
use super::test_lab::{
    Answer, Answering, Faults, LAB, count, lab_device, oid, refused, refusing_rpm, row,
    serve_logged, stop, text,
};
use super::*;
use crate::device::DeviceAddress;
use crate::format::RawRow;

/// ReadProperty and ReadPropertyMultiple, as the send log names them.
const RP: u8 = 12;
const RPM: u8 = 14;

/// The properties this file is about.
const NEW: [P; 5] = [
    P::DEVICE_TYPE,
    P::PROFILE_NAME,
    P::EVENT_STATE,
    P::RELIABILITY,
    P::CURRENT_COMMAND_PRIORITY,
];

fn of(object_type: ObjectType, rows: Vec<RawRow>) -> PointRecord {
    let at = DeviceAddress::local(&[2]).unwrap();
    record(&at, 7, oid(object_type, 1), rows)
}

/// Values and details both.
fn everything() -> InventoryOptions {
    InventoryOptions::new().with_values(true).with_details(true)
}

/// The datatype `property` has, as a problem names it, and a value of it.
fn datatype(property: P) -> (&'static str, V) {
    match property {
        P::DEVICE_TYPE | P::PROFILE_NAME => ("a character string", text("x")),
        P::EVENT_STATE | P::RELIABILITY => ("an Enumerated", V::Enumerated(0)),
        P::CURRENT_COMMAND_PRIORITY => ("an Unsigned or NULL", V::Unsigned(8)),
        other => panic!("{other} isn't one of these"),
    }
}

/// Whether `record` holds a value for `property`.
fn held(record: &PointRecord, property: P) -> bool {
    match property {
        P::DEVICE_TYPE => record.device_type.is_some(),
        P::PROFILE_NAME => record.profile_name.is_some(),
        P::EVENT_STATE => record.event_state.is_some(),
        P::RELIABILITY => record.reliability.is_some(),
        P::CURRENT_COMMAND_PRIORITY => record.command_priority.is_some(),
        other => panic!("{other} isn't one of these"),
    }
}

fn said(record: &PointRecord, property: P) -> Option<String> {
    record.problem(property).map(ToString::to_string)
}

/// Every type the standard defines up to Color Temperature, for each of
/// these properties it reads: one of its datatype is held with no
/// problem; one of another (an unrelated value, the nearest, two values)
/// is a problem naming the datatype and what came back, and octets that
/// don't decode are a problem saying so, the field empty either way.
#[test]
fn each_is_judged_by_its_datatype() {
    let mut judged = 0;
    for raw in 0..=ObjectType::COLOR_TEMPERATURE.to_raw() {
        let object_type = ObjectType::from_raw(raw);
        let asked = properties(object_type, &everything());
        for property in NEW.into_iter().filter(|p| asked.contains(p)) {
            judged += 1;
            let (name, good) = datatype(property);
            let read = of(object_type, vec![row(property, good.clone())]);
            assert!(
                read.problems.is_empty(),
                "{object_type} {property}: {read:?}"
            );
            assert!(held(&read, property), "{object_type} {property}");
            let once = row(property, good.clone()).1.unwrap();
            let unrelated = match name {
                "a character string" => V::Real(1.0),
                _ => text("wrong"),
            };
            let nearest = match name {
                "a character string" => V::OctetString(vec![0x41]),
                _ => V::Signed(2),
            };
            let cases = [
                (row(property, unrelated), format!("not {name} (got ")),
                (row(property, nearest), format!("not {name} (got ")),
                (
                    (property, Ok([once.clone(), once].concat())),
                    format!("not {name} (got several values, "),
                ),
                ((property, Ok(vec![0x44, 0x3F])), "can't be decoded".into()),
            ];
            for (wrong, expected) in cases {
                let read = of(object_type, vec![wrong]);
                let said = said(&read, property).unwrap_or_default();
                assert!(
                    said.starts_with(&expected),
                    "{object_type} {property}: {said}"
                );
                assert!(!held(&read, property), "{object_type} {property}");
            }
        }
    }
    // Every type reads the two details and the two states; six take
    // commands and read their Current_Command_Priority.
    let types = ObjectType::COLOR_TEMPERATURE.to_raw() as usize + 1;
    assert_eq!(judged, 4 * types + 6);

    // A text in a character set this stack doesn't convert names the set.
    let jis = (P::PROFILE_NAME, Ok(vec![0x73, 0x02, 0x30, 0x30]));
    let read = of(ObjectType::ANALOG_INPUT, vec![jis]);
    let expected = "can't be decoded: a character string in JIS X 0208";
    assert_eq!(said(&read, P::PROFILE_NAME).as_deref(), Some(expected));
    // A vendor's type reads them too, judged the same way.
    let vendor = ObjectType::from_raw(700);
    let read = of(vendor, vec![row(P::EVENT_STATE, V::Unsigned(2))]);
    let expected = "not an Enumerated (got an Unsigned, 2)";
    assert_eq!(said(&read, P::EVENT_STATE).as_deref(), Some(expected));
}

/// An output's Current_Command_Priority is named: the priority and what
/// the project calls it, or `relinquish default` for NULL. One outside 1
/// to 16 is kept as the device gave it, with a problem.
#[test]
fn an_outputs_priority_in_control_is_named() {
    let cases = [
        (V::Null, "relinquish default"),
        (V::Unsigned(1), "1 (life safety, manual)"),
        (V::Unsigned(8), "8 (operator, manual)"),
        (V::Unsigned(16), "16 (site defined)"),
    ];
    for (value, text) in cases {
        let read = of(
            ObjectType::BINARY_OUTPUT,
            vec![row(P::CURRENT_COMMAND_PRIORITY, value.clone())],
        );
        assert!(read.problems.is_empty(), "{read:?}");
        let held = read.command_priority.unwrap();
        assert_eq!(held.text, text);
        assert_eq!(held.decode().unwrap(), value);
    }
    for level in [0, 17] {
        let read = of(
            ObjectType::ANALOG_OUTPUT,
            vec![row(P::CURRENT_COMMAND_PRIORITY, V::Unsigned(level))],
        );
        assert_eq!(
            read.command_priority.as_ref().unwrap().text,
            level.to_string()
        );
        let said = said(&read, P::CURRENT_COMMAND_PRIORITY);
        assert_eq!(said.as_deref(), Some("not a priority (1 to 16)"));
    }
    // An older device's output without the property: none, no problem.
    let read = of(
        ObjectType::MULTI_STATE_OUTPUT,
        vec![refused(P::CURRENT_COMMAND_PRIORITY)],
    );
    assert!(read.command_priority.is_none() && read.problems.is_empty());
}

/// Sixteen slots, NULL but for those `held` names (priority, value).
fn slots(held: &[(usize, V)]) -> V {
    let mut slots = vec![V::Null; 16];
    for (level, value) in held {
        slots[level - 1] = value.clone();
    }
    V::List(slots)
}

/// A value object's priority in control, with values, is the first slot
/// of its Priority_Array that holds a command, written as the property
/// would be; NULL when none does. Without values it isn't worked out. An
/// array that doesn't decode or doesn't hold sixteen values is a
/// Priority_Array problem saying so, the object still commandable.
#[test]
fn a_value_objects_priority_in_control_comes_from_its_array() {
    let with_values = |array: RawRow| {
        let present = row(P::PRESENT_VALUE, V::Real(21.0));
        of(ObjectType::ANALOG_VALUE, vec![array, present])
    };
    let read = with_values(row(
        P::PRIORITY_ARRAY,
        slots(&[(10, V::Real(19.0)), (12, V::Real(22.0))]),
    ));
    let held = read.command_priority.as_ref().unwrap();
    assert_eq!(held.text, "10 (site defined)");
    assert_eq!(held.decode().unwrap(), V::Unsigned(10));
    assert!(read.commandable && read.problems.is_empty(), "{read:?}");
    let read = with_values(row(P::PRIORITY_ARRAY, slots(&[])));
    let held = read.command_priority.as_ref().unwrap();
    assert_eq!(
        (held.text.as_str(), held.decode().unwrap()),
        ("relinquish default", V::Null)
    );

    // Without values, nothing is worked out.
    let array = row(P::PRIORITY_ARRAY, slots(&[(5, V::Real(1.0))]));
    let read = of(ObjectType::ANALOG_VALUE, vec![array]);
    assert!(read.command_priority.is_none() && read.problems.is_empty());

    // A DateTime Value's slot holds a date and time under a context tag.
    let mut array = vec![0x00; 4];
    array.extend([0x1E, 0xA4, 126, 11, 24, 2, 0xB4, 7, 30, 0, 0, 0x1F]);
    array.extend([0x00; 11]);
    let present = row(P::PRESENT_VALUE, V::Null);
    let read = of(
        ObjectType::DATETIME_VALUE,
        vec![(P::PRIORITY_ARRAY, Ok(array)), present],
    );
    let held = read
        .command_priority
        .as_ref()
        .map(|value| value.text.as_str());
    assert_eq!(held, Some("5 (critical equipment)"), "{read:?}");

    let broken = [
        (row(P::PRIORITY_ARRAY, V::Null), "one value, not 16"),
        (
            row(P::PRIORITY_ARRAY, V::List(vec![V::Null; 15])),
            "15 values, not 16",
        ),
        (
            (P::PRIORITY_ARRAY, Ok(vec![0x00, 0x44, 0x3F])),
            "slot 2 can't be decoded",
        ),
    ];
    for (array, expected) in broken {
        let read = with_values(array);
        assert!(read.command_priority.is_none(), "{read:?}");
        assert_eq!(said(&read, P::PRIORITY_ARRAY).as_deref(), Some(expected));
        assert!(read.commandable, "the array came, so it takes commands");
    }
}

/// The device's objects: an input, the six types whose Present_Value
/// always takes commands, and commandable analog and integer values,
/// the integer value without an Event_State, as it reports no events.
fn objects() -> Vec<Box<dyn BACnetObject>> {
    let celsius = EngineeringUnits::DEGREES_CELSIUS;
    vec![
        Box::new(AnalogInputObject::new(1, "ai", celsius).unwrap()),
        Box::new(AnalogOutputObject::new(1, "ao", celsius).unwrap()),
        Box::new(BinaryInputObject::new(1, "bi").unwrap()),
        Box::new(BinaryOutputObject::new(1, "bo").unwrap()),
        Box::new(MultiStateOutputObject::new(1, "mso", 3).unwrap()),
        Box::new(AccessDoorObject::new(1, "door").unwrap()),
        Box::new(LightingOutputObject::new(1, "light").unwrap()),
        Box::new(BinaryLightingOutputObject::new(1, "switch").unwrap()),
        Box::new(AnalogValueObject::new(1, "av", celsius).unwrap()),
        Box::new(IntegerValueObject::new(1, "iv").unwrap()),
    ]
}

/// `objects`, the analog output commanded at 8 and the analog value at
/// 12, each wrapped to answer as `answers` says for its type.
fn device(answers: impl Fn(ObjectType) -> Vec<(P, Answer)>) -> Vec<Box<dyn BACnetObject>> {
    objects()
        .into_iter()
        .map(|mut object| {
            let object_type = object.object_identifier().object_type();
            let level = match object_type {
                ObjectType::ANALOG_OUTPUT => Some(8),
                ObjectType::ANALOG_VALUE => Some(12),
                _ => None,
            };
            if let Some(level) = level {
                // A command from the device itself.
                let origin = CommandOrigin::Local {
                    owner_device: oid(ObjectType::DEVICE, LAB),
                    initiating_object: None,
                };
                let value = V::Real(40.0);
                object
                    .write_property_from(P::PRESENT_VALUE, None, value, Some(level), &origin)
                    .unwrap();
            }
            Answering::wrap(object, answers(object_type))
        })
        .collect()
}

/// The inventory of a device of `objects` under `options`, by
/// ReadPropertyMultiple and then by ReadProperty, each with how many
/// requests of each service it sent.
async fn both_ways(
    objects: impl Fn() -> Vec<Box<dyn BACnetObject>>,
    options: &InventoryOptions,
) -> [(Inventory, usize, usize); 2] {
    let identity = DeviceIdentity::new(LAB, 42).unwrap();
    let mut read = Vec::new();
    for faults in [Faults::default(), refusing_rpm()] {
        let database = build_database_with_extra(&identity, objects()).unwrap();
        let (client, server, log) = serve_logged(faults, identity.clone(), database).await;
        let inventory = lab_device(&client).await.inventory(options).await.unwrap();
        let sent = (count(&log, RPM), count(&log, RP));
        stop(client, server).await;
        assert!(inventory.stopped.is_none(), "{inventory:?}");
        read.push((inventory, sent.0, sent.1));
    }
    read.try_into().unwrap()
}

/// With values and details, by either service: each output's priority in
/// control named from its own property, the commandable value's from its
/// Priority_Array, the states named, and no problem where the device has
/// what the standard requires.
///
/// What it costs: values add four properties an object and five on an
/// output, details two more. The Device object reads 7 (its name came
/// with the summary), the analog input 9, the analog output, binary input
/// and analog and integer values 10 each, the binary and multi-state
/// outputs 11, Access Door and the lighting outputs 9: 105 properties. By
/// ReadPropertyMultiple, at most 32 a batch, that is four batches (the
/// Device, ai and ao: 26; bi, bo and mso: 32; door, light and switch: 27;
/// av and iv: 20), against two for these objects read plain. By
/// ReadProperty, one request a property after the refused RPM.
#[tokio::test(start_paused = true)]
async fn the_state_reads_the_same_by_both_services() {
    let options = everything();
    let [(by_rpm, rpm, rp), (by_rp, refused_rpm, read_property)] =
        both_ways(|| device(|_| Vec::new()), &options).await;
    assert_eq!(by_rpm.points, by_rp.points);
    let asked: usize = by_rpm
        .points
        .iter()
        .map(|point| properties(point.object.object_type(), &options).len())
        .sum::<usize>()
        - 1;
    assert_eq!(asked, 105);
    assert_eq!((refused_rpm, read_property), (1, 13 + 1 + asked));
    assert_eq!((rpm, rp), (1 + 4, 1), "the summary and four batches");
    // Details alone: 33 properties plain, two more an object, 55. Two
    // batches still (the Device to the binary output: 31; the rest: 24).
    let details = InventoryOptions::new().with_details(true);
    let [(_, rpm, _), (_, _, read_property)] = both_ways(|| device(|_| Vec::new()), &details).await;
    assert_eq!((rpm, read_property), (1 + 2, 13 + 1 + 55));

    let point = |object_type| by_rpm.point(oid(object_type, 1)).unwrap();
    let priority = |object_type| {
        let point = point(object_type);
        point
            .command_priority
            .as_ref()
            .map(|value| value.text.clone())
    };
    assert_eq!(
        priority(ObjectType::ANALOG_OUTPUT).as_deref(),
        Some("8 (operator, manual)")
    );
    assert_eq!(
        priority(ObjectType::ANALOG_VALUE).as_deref(),
        Some("12 (site defined)")
    );
    for relinquished in [ObjectType::BINARY_OUTPUT, ObjectType::MULTI_STATE_OUTPUT] {
        assert_eq!(
            priority(relinquished).as_deref(),
            Some("relinquish default")
        );
    }
    assert_eq!(priority(ObjectType::ANALOG_INPUT), None, "an input");
    let iv = priority(ObjectType::INTEGER_VALUE);
    assert_eq!(
        iv.as_deref(),
        Some("relinquish default"),
        "commanded at none"
    );
    let ai = point(ObjectType::ANALOG_INPUT);
    assert_eq!(ai.event_state.as_ref().unwrap().text, "normal");
    // An integer value that doesn't report events has no Event_State,
    // which its table makes no requirement: none, and no problem.
    assert!(point(ObjectType::INTEGER_VALUE).event_state.is_none());
    for record in &by_rpm.points {
        assert!(record.problems.is_empty(), "{record:?}");
    }
}

/// A device without the optional properties (an older device's output
/// without Current_Command_Priority, objects without Reliability, a
/// Device_Type or a Profile_Name) fails nothing and has no problem for
/// them; a binary input without its required Event_State has one. One
/// that gives them of the wrong datatype has a problem saying what came,
/// by either service.
#[tokio::test(start_paused = true)]
async fn a_device_without_them_or_with_odd_ones() {
    let unknown: Answer = Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY));
    let lacking = |object_type| match object_type {
        ObjectType::ANALOG_OUTPUT | ObjectType::BINARY_OUTPUT => {
            vec![(P::CURRENT_COMMAND_PRIORITY, unknown.clone())]
        }
        ObjectType::BINARY_INPUT => vec![(P::EVENT_STATE, unknown.clone())],
        _ => vec![(P::RELIABILITY, unknown.clone())],
    };
    let options = InventoryOptions::new().with_values(true);
    for (inventory, ..) in both_ways(|| device(lacking), &options).await {
        for record in &inventory.points {
            let object_type = record.object.object_type();
            let problems: Vec<P> = record.problems.iter().map(|p| p.property).collect();
            if object_type == ObjectType::BINARY_INPUT {
                assert_eq!(problems, [P::EVENT_STATE], "required: {record:?}");
            } else {
                assert!(problems.is_empty(), "{record:?}");
            }
        }
        let ao = inventory.point(oid(ObjectType::ANALOG_OUTPUT, 1)).unwrap();
        assert!(ao.command_priority.is_none(), "{ao:?}");
    }

    let odd = |object_type| match object_type {
        ObjectType::ANALOG_OUTPUT => vec![(P::CURRENT_COMMAND_PRIORITY, Ok(V::Real(1.5)))],
        ObjectType::MULTI_STATE_OUTPUT => {
            vec![(P::CURRENT_COMMAND_PRIORITY, Ok(V::Unsigned(17)))]
        }
        ObjectType::ANALOG_INPUT => vec![
            (P::EVENT_STATE, Ok(V::Unsigned(2))),
            (P::RELIABILITY, Ok(text("fine"))),
        ],
        _ => Vec::new(),
    };
    let expected = [
        (
            ObjectType::ANALOG_OUTPUT,
            P::CURRENT_COMMAND_PRIORITY,
            "not an Unsigned or NULL (got a Real, 1.5)",
        ),
        (
            ObjectType::MULTI_STATE_OUTPUT,
            P::CURRENT_COMMAND_PRIORITY,
            "not a priority (1 to 16)",
        ),
        (
            ObjectType::ANALOG_INPUT,
            P::EVENT_STATE,
            "not an Enumerated (got an Unsigned, 2)",
        ),
        (
            ObjectType::ANALOG_INPUT,
            P::RELIABILITY,
            "not an Enumerated (got a character string, \"fine\")",
        ),
    ];
    for (inventory, ..) in both_ways(|| device(odd), &options).await {
        for (object_type, property, error) in expected {
            let record = inventory.point(oid(object_type, 1)).unwrap();
            assert_eq!(said(record, property).as_deref(), Some(error), "{record:?}");
        }
        let mso = inventory
            .point(oid(ObjectType::MULTI_STATE_OUTPUT, 1))
            .unwrap();
        assert_eq!(mso.command_priority.as_ref().unwrap().text, "17", "kept");
        let ai = inventory.point(oid(ObjectType::ANALOG_INPUT, 1)).unwrap();
        assert!(ai.event_state.is_none() && ai.reliability.is_none());
    }
}
