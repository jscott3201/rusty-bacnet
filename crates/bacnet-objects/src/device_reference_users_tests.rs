//! Every writable device reference property answers the same value the same
//! way (#1313, #1308): one table of local, empty, non-Device, malformed and
//! wrong-datatype values, written to each property through the shared
//! helpers. A refused value leaves the property as it was. The setters'
//! table is `device_reference_setter_tests.rs`.
//!
//! The Loop and Pulse Converter references (`reference.rs`) are in the table
//! too, for every row but the Device member ones: their production has no
//! Device member, and they give the same answers through the same
//! single-reference decoder (#1395).

use bacnet_types::error::RemoteError;
use std::sync::Arc;

use bacnet_types::constructed::BACnetStageLimitValue;
use bacnet_types::enums::{EngineeringUnits, ErrorClass, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{Date, Time};

use super::*;
use crate::accumulator::PulseConverterObject;
use crate::averaging::AveragingObject;
use crate::channel::ChannelObject;
use crate::clock::{ClockFrame, ClockReader};
use crate::loop_obj::LoopObject;
use crate::reference::{object_property_reference_value, setpoint_reference_value};
use crate::schedule::ScheduleObject;
use crate::staging::{StagingConfig, StagingObject};
use crate::traits::BACnetObject;
use crate::trend::{TrendLogMultipleObject, TrendLogObject};

type P = PropertyIdentifier;

const EMPTY: u32 = ObjectIdentifier::WILDCARD_INSTANCE;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

/// A clock for the trend logs, whose reference change purges the buffer
/// with a time-stamped record.
struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 10,
                day: 3,
                day_of_week: 6,
            },
            local_time: Time {
                hour: 9,
                minute: 0,
                second: 0,
                hundredths: 0,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

/// The class and code of a refusal, whether or not it names an element.
fn class_and_code(error: &Error) -> (ErrorClass, ErrorCode) {
    match error {
        Error::Remote(RemoteError::Error { class, code })
        | Error::Structured { class, code, .. } => (*class, *code),
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

fn property_code(code: ErrorCode) -> Option<(ErrorClass, ErrorCode)> {
    Some((ErrorClass::PROPERTY, code))
}

/// Which Clause 21 production the property holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Production {
    Property,
    Object,
    /// `BACnetObjectPropertyReference`, with no Device member.
    Bare,
    /// `BACnetSetpointReference`: a bare reference framed in context tag 0,
    /// whose empty value holds no reference.
    Setpoint,
}

impl Production {
    fn has_device_member(self) -> bool {
        matches!(self, Production::Property | Production::Object)
    }
}

/// The encoding of one reference to `object`, with `device` as its Device
/// member (none on a production without one).
fn encoded(
    production: Production,
    object: ObjectIdentifier,
    device: Option<ObjectIdentifier>,
) -> Vec<u8> {
    let local = BACnetObjectPropertyReference::new(object, P::PRESENT_VALUE.to_raw());
    let value = match production {
        Production::Property => reference_value(&BACnetDeviceObjectPropertyReference {
            device_identifier: device,
            ..local_property_reference(&local)
        }),
        Production::Object => reference_value(&BACnetDeviceObjectReference {
            device_identifier: device,
            object_identifier: object,
        }),
        Production::Bare => object_property_reference_value(Some(&local), ObjectType::ACCUMULATOR),
        Production::Setpoint => setpoint_reference_value(Some(&local)),
    };
    assert!(device.is_none() || production.has_device_member());
    octets(&value)
}

/// One writable device reference property, on a fresh object holding one
/// valid reference to `target`.
struct Writable {
    name: &'static str,
    build: fn() -> Box<dyn BACnetObject>,
    property: PropertyIdentifier,
    index: Option<u32>,
    production: Production,
    target: ObjectType,
    /// The value holds exactly one reference (a single reference, or one
    /// array element by index).
    single: bool,
}

fn averaging() -> Box<dyn BACnetObject> {
    Box::new(AveragingObject::new(1, "AVG-1").unwrap())
}

fn trend_log() -> Box<dyn BACnetObject> {
    let mut log = TrendLogObject::new(1, "TL-1", 8).unwrap();
    log.bind_clock_internal(Some(Arc::new(FixedClock)));
    Box::new(log)
}

fn trend_log_multiple() -> Box<dyn BACnetObject> {
    let mut log = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
    log.add_property_reference(BACnetDeviceObjectPropertyReference::new_local(
        oid(ObjectType::ANALOG_INPUT, 1),
        P::PRESENT_VALUE.to_raw(),
    ))
    .unwrap();
    log.bind_clock_internal(Some(Arc::new(FixedClock)));
    Box::new(log)
}

fn channel() -> Box<dyn BACnetObject> {
    let mut channel = ChannelObject::new(1, "CH-1", 1).unwrap();
    channel
        .set_members(vec![BACnetDeviceObjectPropertyReference::new_local(
            oid(ObjectType::ANALOG_OUTPUT, 1),
            P::PRESENT_VALUE.to_raw(),
        )])
        .unwrap();
    Box::new(channel)
}

fn schedule() -> Box<dyn BACnetObject> {
    Box::new(ScheduleObject::new(1, "SCH-1", PropertyValue::Real(0.0)).unwrap())
}

fn staging() -> Box<dyn BACnetObject> {
    let stage = |limit, active| BACnetStageLimitValue {
        limit,
        values: vec![active],
        deadband: 1.0,
    };
    let config = StagingConfig {
        present_value: 5.0,
        min_present_value: -1.0,
        units: EngineeringUnits::DEGREES_CELSIUS,
        priority_for_writing: 8,
        stages: vec![stage(10.0, false), stage(20.0, true)],
        target_references: vec![oid(ObjectType::BINARY_OUTPUT, 1).into()],
        stage_names: None,
    };
    Box::new(StagingObject::new(1, "STG-1", config).unwrap())
}

fn loop_object() -> Box<dyn BACnetObject> {
    let reference = |object_type| {
        BACnetObjectPropertyReference::new(oid(object_type, 1), P::PRESENT_VALUE.to_raw())
    };
    let mut lo = LoopObject::new(1, "LOOP-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    lo.set_controlled_variable_reference(reference(ObjectType::ANALOG_INPUT));
    lo.set_manipulated_variable_reference(reference(ObjectType::ANALOG_OUTPUT));
    lo.set_setpoint_reference(reference(ObjectType::ANALOG_VALUE));
    Box::new(lo)
}

fn pulse_converter() -> Box<dyn BACnetObject> {
    let mut pc = PulseConverterObject::new(1, "PC-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    pc.set_input_reference(BACnetObjectPropertyReference::new(
        oid(ObjectType::ACCUMULATOR, 1),
        P::PRESENT_VALUE.to_raw(),
    ));
    Box::new(pc)
}

const WRITABLE: [Writable; 13] = [
    Writable {
        name: "Averaging Object_Property_Reference",
        build: averaging,
        property: P::OBJECT_PROPERTY_REFERENCE,
        index: None,
        production: Production::Property,
        target: ObjectType::ANALOG_INPUT,
        single: true,
    },
    Writable {
        name: "Trend Log Log_DeviceObjectProperty",
        build: trend_log,
        property: P::LOG_DEVICE_OBJECT_PROPERTY,
        index: None,
        production: Production::Property,
        target: ObjectType::ANALOG_INPUT,
        single: true,
    },
    Writable {
        name: "Trend Log Multiple Log_DeviceObjectProperty[1]",
        build: trend_log_multiple,
        property: P::LOG_DEVICE_OBJECT_PROPERTY,
        index: Some(1),
        production: Production::Property,
        target: ObjectType::ANALOG_INPUT,
        single: true,
    },
    Writable {
        name: "Trend Log Multiple Log_DeviceObjectProperty",
        build: trend_log_multiple,
        property: P::LOG_DEVICE_OBJECT_PROPERTY,
        index: None,
        production: Production::Property,
        target: ObjectType::ANALOG_INPUT,
        single: false,
    },
    Writable {
        name: "Channel List_Of_Object_Property_References[1]",
        build: channel,
        property: P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        index: Some(1),
        production: Production::Property,
        target: ObjectType::ANALOG_OUTPUT,
        single: true,
    },
    Writable {
        name: "Channel List_Of_Object_Property_References",
        build: channel,
        property: P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        index: None,
        production: Production::Property,
        target: ObjectType::ANALOG_OUTPUT,
        single: false,
    },
    Writable {
        name: "Schedule List_Of_Object_Property_References",
        build: schedule,
        property: P::LIST_OF_OBJECT_PROPERTY_REFERENCES,
        index: None,
        production: Production::Property,
        target: ObjectType::ANALOG_VALUE,
        single: false,
    },
    Writable {
        name: "Staging Target_References[1]",
        build: staging,
        property: P::TARGET_REFERENCES,
        index: Some(1),
        production: Production::Object,
        target: ObjectType::BINARY_OUTPUT,
        single: true,
    },
    Writable {
        name: "Staging Target_References",
        build: staging,
        property: P::TARGET_REFERENCES,
        index: None,
        production: Production::Object,
        target: ObjectType::BINARY_OUTPUT,
        single: false,
    },
    Writable {
        name: "Loop Controlled_Variable_Reference",
        build: loop_object,
        property: P::CONTROLLED_VARIABLE_REFERENCE,
        index: None,
        production: Production::Bare,
        target: ObjectType::ANALOG_INPUT,
        single: true,
    },
    Writable {
        name: "Loop Manipulated_Variable_Reference",
        build: loop_object,
        property: P::MANIPULATED_VARIABLE_REFERENCE,
        index: None,
        production: Production::Bare,
        target: ObjectType::ANALOG_OUTPUT,
        single: true,
    },
    Writable {
        name: "Loop Setpoint_Reference",
        build: loop_object,
        property: P::SETPOINT_REFERENCE,
        index: None,
        production: Production::Setpoint,
        target: ObjectType::ANALOG_VALUE,
        single: true,
    },
    Writable {
        name: "Pulse Converter Input_Reference",
        build: pulse_converter,
        property: P::INPUT_REFERENCE,
        index: None,
        production: Production::Bare,
        target: ObjectType::ACCUMULATOR,
        single: true,
    },
];

/// The class and code of a refusal, or `None` for acceptance.
type Answer = Option<(ErrorClass, ErrorCode)>;

/// One row of the write table: the value, built for a user, and the answer
/// every user gives it, one for a single-reference value and one for a list
/// or array written whole.
struct Row {
    what: &'static str,
    single_only: bool,
    value: fn(&Writable) -> PropertyValue,
    single: Answer,
    list: Answer,
    /// The value carries a Device member, so only productions with one
    /// take part.
    device: bool,
    /// The value holds no octets: the empty `BACnetSetpointReference`,
    /// which Setpoint_Reference takes, and no reference anywhere else.
    empty: bool,
}

impl Row {
    /// The answer `user` gives the row.
    fn answer(&self, user: &Writable) -> Answer {
        if self.empty && user.production == Production::Setpoint {
            None
        } else if user.single {
            self.single
        } else {
            self.list
        }
    }
}

fn reference_to(user: &Writable, instance: u32, device: Option<ObjectIdentifier>) -> Vec<u8> {
    encoded(user.production, oid(user.target, instance), device)
}

/// One reference to instance 2, then `trailing`.
fn followed_by(user: &Writable, trailing: &[u8]) -> PropertyValue {
    PropertyValue::ApplicationData([reference_to(user, 2, None), trailing.to_vec()].concat())
}

/// A row every user takes part in.
fn row(
    what: &'static str,
    value: fn(&Writable) -> PropertyValue,
    single: Answer,
    list: Answer,
) -> Row {
    Row {
        what,
        single_only: false,
        value,
        single,
        list,
        device: false,
        empty: false,
    }
}

fn rows() -> Vec<Row> {
    let taken = None;
    let out_of_range = property_code(ErrorCode::VALUE_OUT_OF_RANGE);
    let encoding = property_code(ErrorCode::INVALID_DATA_ENCODING);
    let datatype = property_code(ErrorCode::INVALID_DATA_TYPE);
    vec![
        row(
            "a reference to an object in this device",
            |user| PropertyValue::ApplicationData(reference_to(user, 2, None)),
            taken,
            taken,
        ),
        row(
            "the empty object instance 4194303",
            |user| PropertyValue::ApplicationData(reference_to(user, EMPTY, None)),
            taken,
            taken,
        ),
        Row {
            device: true,
            ..row(
                "a non-Device device identifier",
                |user| {
                    let device = Some(oid(ObjectType::ANALOG_VALUE, 9));
                    PropertyValue::ApplicationData(reference_to(user, 2, device))
                },
                out_of_range,
                out_of_range,
            )
        },
        Row {
            device: true,
            ..row(
                "a non-Device device identifier at the empty instance",
                |user| {
                    let device = Some(oid(ObjectType::ANALOG_VALUE, EMPTY));
                    PropertyValue::ApplicationData(reference_to(user, EMPTY, device))
                },
                out_of_range,
                out_of_range,
            )
        },
        Row {
            single_only: true,
            ..row(
                "a second reference in a single-reference value",
                |user| {
                    let one = reference_to(user, 2, None);
                    PropertyValue::ApplicationData([one.clone(), one].concat())
                },
                encoding,
                encoding,
            )
        },
        row(
            "a truncated reference",
            |user| {
                let one = reference_to(user, 2, None);
                PropertyValue::ApplicationData(one[..one.len() - 1].to_vec())
            },
            encoding,
            encoding,
        ),
        // After one whole reference, anything at all is an encoding fault in a
        // single-reference value; in a list it is the next element, which
        // either can't open a reference or doesn't decode.
        row(
            "a context tag [4] after the reference",
            |user| followed_by(user, &[0x49, 0x01]),
            encoding,
            datatype,
        ),
        row(
            "an application Unsigned after the reference",
            |user| followed_by(user, &[0x21, 0x01]),
            encoding,
            datatype,
        ),
        row(
            "a context tag [0] after the reference",
            |user| followed_by(user, &[0x09, 0x01]),
            encoding,
            encoding,
        ),
        row("a REAL", |_| PropertyValue::Real(1.0), datatype, datatype),
        row(
            "an application-tagged object identifier",
            |user| {
                let id = oid(user.target, 2);
                let mut bytes = vec![0xC4];
                bytes.extend_from_slice(&id.encode());
                PropertyValue::ApplicationData(bytes)
            },
            datatype,
            datatype,
        ),
        // The shapes only a direct write can hand over (#1395): the server
        // passes these properties' octets whole.
        Row {
            single_only: true,
            ..row(
                "the reference as a list of one-octet chunks",
                |user| {
                    let pieces = reference_to(user, 2, None).into_iter();
                    PropertyValue::List(
                        pieces
                            .map(|octet| PropertyValue::ApplicationData(vec![octet]))
                            .collect(),
                    )
                },
                taken,
                taken,
            )
        },
        row(
            "a list mixing a chunk with a decoded value",
            |user| {
                PropertyValue::List(vec![
                    PropertyValue::ApplicationData(reference_to(user, 2, None)),
                    PropertyValue::Unsigned(1),
                ])
            },
            datatype,
            datatype,
        ),
        Row {
            single_only: true,
            empty: true,
            ..row(
                "no octets",
                |_| PropertyValue::ApplicationData(Vec::new()),
                encoding,
                encoding,
            )
        },
        Row {
            single_only: true,
            empty: true,
            ..row(
                "an empty list",
                |_| PropertyValue::List(Vec::new()),
                encoding,
                encoding,
            )
        },
    ]
}

/// The octets a value carries: raw octets, or the chunks of a list joined.
fn octets(value: &PropertyValue) -> Vec<u8> {
    match value {
        PropertyValue::ApplicationData(bytes) => bytes.clone(),
        PropertyValue::List(items) => items.iter().flat_map(octets).collect(),
        other => panic!("not reference octets: {other:?}"),
    }
}

#[test]
fn every_writable_device_reference_answers_the_same_value_the_same_way() {
    for user in &WRITABLE {
        for row in rows() {
            if (row.single_only && !user.single)
                || (row.device && !user.production.has_device_member())
            {
                continue;
            }
            let mut object = (user.build)();
            let before = object.read_property(user.property, user.index).unwrap();
            let value = (row.value)(user);
            let result = object.write_property(user.property, user.index, value.clone(), None);
            let context = format!("{}: {}", user.name, row.what);
            let read = object.read_property(user.property, user.index).unwrap();
            match row.answer(user) {
                None => {
                    result.unwrap_or_else(|error| panic!("{context}: refused with {error:?}"));
                    assert_eq!(
                        octets(&read),
                        octets(&value),
                        "{context}: stored as written"
                    );
                }
                Some(expected) => {
                    let error = result.expect_err(&context);
                    assert_eq!(class_and_code(&error), expected, "{context}: {error:?}");
                    assert_eq!(read, before, "{context}: a refused value changes nothing");
                }
            }
        }
    }
}
