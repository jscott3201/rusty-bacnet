//! The local-write cases (#2446, #2468), one list for every host that
//! makes the application's own writes: the server and bacnet-device-core,
//! side by side in `local_write`, and the endpoint session, whose
//! `local_write_tests` in `bacnet-endpoint` includes this file by path.
//!
//! Both crates call every case function here, so a case added for one
//! host that the other doesn't run is dead code there, which the lint gate
//! refuses. A case is a list of steps on one fresh [`database`]; each step
//! says what it is, the change, how many COV notifications the server and
//! the core send for it (the endpoint sends none) and the result.
//!
//! An object whose event detection is on starts by switching it off, so
//! the server's event pass, which neither the core nor the endpoint runs,
//! has nothing to do; BO-1 starts with it off.

use bacnet_device_core::write::local::{LocalChange, LocalCommandSource};
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::binary::{BinaryInputObject, BinaryOutputObject, BinaryValueObject};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::EXECUTED_SERVICES;
use bacnet_objects::device_identity::build_database_with_extra;
use bacnet_objects::group::GroupObject;
use bacnet_objects::life_safety::LifeSafetyPointObject;
use bacnet_objects::multistate::{MultiStateInputObject, MultiStateValueObject};
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::{BACnetObject, LocalInputValue};
use bacnet_objects::value_types::IntegerValueObject;
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P, Reliability,
};
use bacnet_types::error::{Error, RemoteError};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

/// What every host reads back after each step. Event_State shows that
/// the server's event pass, which the others don't run, moved nothing.
pub(crate) const READ_BACK: [P; 9] = [
    P::PRESENT_VALUE,
    P::PRIORITY_ARRAY,
    P::CURRENT_COMMAND_PRIORITY,
    P::STATUS_FLAGS,
    P::RELIABILITY,
    P::VALUE_SOURCE,
    P::LAST_COMMAND_TIME,
    P::OUT_OF_SERVICE,
    P::EVENT_STATE,
];

/// A refusal's class and code; `Ok` for success.
pub(crate) type Outcome = Result<(), (ErrorClass, ErrorCode)>;

const OK: Outcome = Ok(());

/// `result` as an [`Outcome`]; anything but a refusal fails the test.
pub(crate) fn outcome(what: &str, result: Result<(), Error>) -> Outcome {
    result.map_err(|error| match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("{what}: not a refusal: {other:?}"),
    })
}

/// One local change, as each host is handed it.
#[derive(Clone)]
pub(crate) enum Change {
    /// The application's Present_Value: `set_present_value_local`.
    Input(PropertyValue),
    /// The application's Reliability: `set_reliability_local`.
    Reliability(Reliability),
    /// `write_local` of Present_Value as the Device, at the priority.
    Command(PropertyValue, Option<u8>),
    /// `write_local` of Out_Of_Service as the Device.
    OutOfService(bool),
    /// `write_local` of another property as the Device.
    Write(P, PropertyValue),
}

impl Change {
    /// The change as bacnet-device-core takes it.
    pub(crate) fn core(self) -> LocalChange {
        let (property, value, priority) = match self {
            Self::Input(value) => {
                return LocalChange::Input(LocalInputValue::present_value(value));
            }
            Self::Reliability(reliability) => return LocalChange::Reliability(reliability),
            Self::Command(value, priority) => (P::PRESENT_VALUE, value, priority),
            Self::OutOfService(oos) => (P::OUT_OF_SERVICE, PropertyValue::Boolean(oos), None),
            Self::Write(property, value) => (property, value, None),
        };
        LocalChange::Property {
            property,
            array_index: None,
            value,
            priority,
            source: LocalCommandSource::ServerDevice,
        }
    }
}

/// One step of a case.
pub(crate) enum Step {
    /// A local change made on every host.
    Local {
        /// What the step is.
        what: &'static str,
        /// The object changed.
        object: ObjectIdentifier,
        /// The change.
        change: Change,
        /// The COV notifications the server and the core send for it.
        #[allow(dead_code)] // The endpoint sends none and doesn't read it.
        notifications: usize,
        /// The result on every host.
        expected: Outcome,
    },
    /// The Device leaves every host's database.
    RemoveDevice,
}

fn step(
    what: &'static str,
    object: ObjectIdentifier,
    change: Change,
    notifications: usize,
    expected: Outcome,
) -> Step {
    Step::Local {
        what,
        object,
        change,
        notifications,
        expected,
    }
}

pub(crate) fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

pub(crate) fn ai() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

pub(crate) fn msv() -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_VALUE, 1)
}

fn bv1() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 1)
}

/// AV-2: read-only, publishing its source.
pub(crate) fn av2() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 2)
}

/// BV-2: writable.
pub(crate) fn bv2() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 2)
}

/// IV-1: read-only.
pub(crate) fn iv1() -> ObjectIdentifier {
    oid(ObjectType::INTEGER_VALUE, 1)
}

/// IV-2: commandable.
pub(crate) fn iv2() -> ObjectIdentifier {
    oid(ObjectType::INTEGER_VALUE, 2)
}

/// IV-3: writable.
pub(crate) fn iv3() -> ObjectIdentifier {
    oid(ObjectType::INTEGER_VALUE, 3)
}

pub(crate) fn lsp() -> ObjectIdentifier {
    oid(ObjectType::LIFE_SAFETY_POINT, 1)
}

pub(crate) fn bo1() -> ObjectIdentifier {
    oid(ObjectType::BINARY_OUTPUT, 1)
}

pub(crate) fn bi1() -> ObjectIdentifier {
    oid(ObjectType::BINARY_INPUT, 1)
}

/// MSI-1, three states.
pub(crate) fn msi1() -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_INPUT, 1)
}

fn group() -> ObjectIdentifier {
    oid(ObjectType::GROUP, 1)
}

/// The objects the cases change and every host holds: AI-1 at 21.5,
/// BV-1, MSV-1 (three states), Group-1 of the first two and LSP-1, then
/// [`av2`], [`bv2`], [`iv1`], [`iv2`], [`iv3`], [`bo1`], [`bi1`] and
/// [`msi1`].
pub(crate) fn objects() -> Vec<Box<dyn BACnetObject>> {
    let mut analog = AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    analog.set_present_value(21.5);
    let mut group = GroupObject::new(1, "Group-1").unwrap();
    for member in [ai(), bv1()] {
        group
            .add_member(ReadAccessSpecification {
                object_identifier: member,
                list_of_property_references: vec![PropertyReference {
                    property_identifier: P::PRESENT_VALUE,
                    property_array_index: None,
                }],
            })
            .unwrap();
    }
    let mut av = AnalogValueObject::with_access(
        2,
        "AV-2",
        EngineeringUnits::DEGREES_CELSIUS,
        PresentValueAccess::ReadOnly,
    )
    .unwrap();
    av.set_value_source_tracking(true);
    let writable = PresentValueAccess::Writable;
    vec![
        Box::new(analog),
        Box::new(BinaryValueObject::new(1, "BV-1").unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        Box::new(group),
        Box::new(LifeSafetyPointObject::new(1, "LSP-1").unwrap()),
        Box::new(av),
        Box::new(BinaryValueObject::with_access(2, "BV-2", writable).unwrap()),
        Box::new(IntegerValueObject::with_access(1, "IV-1", PresentValueAccess::ReadOnly).unwrap()),
        Box::new(IntegerValueObject::new(2, "IV-2").unwrap()),
        Box::new(IntegerValueObject::with_access(3, "IV-3", writable).unwrap()),
        Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()),
        Box::new(BinaryInputObject::new(1, "BI-1").unwrap()),
        Box::new(MultiStateInputObject::new(1, "MSI-1", 3).unwrap()),
    ]
}

/// The Device `identity` describes, declaring the executed services, and
/// [`objects`]. Each host sets its own clock.
pub(crate) fn database(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let identity = identity.clone().with_services(EXECUTED_SERVICES);
    build_database_with_extra(&identity, objects()).unwrap()
}

/// A write of Event_Detection_Enable. Off, the server's event pass has
/// nothing to do for the object, as the core and the endpoint, which run
/// none, haven't.
pub(crate) fn detection(on: bool) -> Change {
    Change::Write(P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(on))
}

fn denied() -> Outcome {
    Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED))
}

fn range() -> Outcome {
    Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE))
}

fn mistyped() -> Outcome {
    Err((ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE))
}

fn unsupported() -> Outcome {
    Err((
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    ))
}

fn input(value: PropertyValue) -> Change {
    Change::Input(value)
}

fn at(value: PropertyValue, priority: u8) -> Change {
    Change::Command(value, Some(priority))
}

fn relinquish(priority: u8) -> Change {
    Change::Command(PropertyValue::Null, Some(priority))
}

pub(crate) fn fault(reliability: Reliability) -> Change {
    Change::Reliability(reliability)
}

pub(crate) fn oos(on: bool) -> Change {
    Change::OutOfService(on)
}

fn enumerated(raw: u32) -> PropertyValue {
    PropertyValue::Enumerated(raw)
}

fn unsigned(raw: u32) -> PropertyValue {
    PropertyValue::Unsigned(u64::from(raw))
}

fn real(value: f32) -> PropertyValue {
    PropertyValue::Real(value)
}

fn signed(value: i32) -> PropertyValue {
    PropertyValue::Signed(value)
}

/// An input is taken in service and refused out of it, where a command
/// simulates the value instead; back in service, the input is taken again.
pub(crate) fn input_in_and_out_of_service() -> Vec<Step> {
    vec![
        step("in service", ai(), input(real(22.0)), 1, OK),
        step("out of service", ai(), oos(true), 1, OK),
        step(
            "input, out of service",
            ai(),
            input(real(23.0)),
            0,
            denied(),
        ),
        step("simulated", ai(), at(real(30.0), 8), 1, OK),
        step("back in service", ai(), oos(false), 1, OK),
        step("input, back in service", ai(), input(real(24.0)), 1, OK),
        step("command, in service", ai(), at(real(31.0), 8), 0, denied()),
    ]
}

/// A command below the active one waits and one above it takes over; each
/// relinquish hands Present_Value to the next, and the last to
/// Relinquish_Default.
pub(crate) fn commands_and_relinquishes() -> Vec<Step> {
    vec![
        step("command at 8", msv(), at(unsigned(2), 8), 1, OK),
        step("command below, at 12", msv(), at(unsigned(3), 12), 0, OK),
        step("command above, at 6", msv(), at(unsigned(3), 6), 1, OK),
        step("relinquish 6", msv(), relinquish(6), 1, OK),
        step("relinquish 8", msv(), relinquish(8), 1, OK),
        step(
            "relinquish 12, to the default",
            msv(),
            relinquish(12),
            1,
            OK,
        ),
    ]
}

/// Values the objects refuse, refused alike and reported to no one: a
/// non-finite or mistyped input, a state out of range or a priority outside
/// 1 to 16, and an input to a commandable Present_Value.
pub(crate) fn refused_values() -> Vec<Step> {
    vec![
        step("NaN", ai(), input(real(f32::NAN)), 0, range()),
        step(
            "a BOOLEAN",
            ai(),
            input(PropertyValue::Boolean(true)),
            0,
            mistyped(),
        ),
        step("state 4 of 3", msv(), at(unsigned(4), 8), 0, range()),
        step("state 0", msv(), at(unsigned(0), 8), 0, range()),
        step("priority 0", msv(), at(unsigned(2), 0), 0, range()),
        step("priority 17", msv(), at(unsigned(2), 17), 0, range()),
        step(
            "an input to a commandable value",
            msv(),
            input(unsigned(2)),
            0,
            unsupported(),
        ),
    ]
}

/// The application's Reliability, with AI-1's event detection off: taken
/// in service, where Status_Flags' FAULT reports, and again unchanged with
/// nothing to report; refused outside BACnetReliability, while
/// Reliability_Evaluation_Inhibit is TRUE, out of service, and by an object
/// that takes none. A Life Safety Point's goes through its own COV path.
pub(crate) fn reliability() -> Vec<Step> {
    let inhibit = |on| {
        Change::Write(
            P::RELIABILITY_EVALUATION_INHIBIT,
            PropertyValue::Boolean(on),
        )
    };
    vec![
        step("event detection off", ai(), detection(false), 0, OK),
        step("a fault", ai(), fault(Reliability::NO_SENSOR), 1, OK),
        step("the same fault", ai(), fault(Reliability::NO_SENSOR), 0, OK),
        step(
            "outside the enumeration",
            ai(),
            fault(Reliability::from_raw(26)),
            0,
            range(),
        ),
        step(
            "the fault cleared",
            ai(),
            fault(Reliability::NO_FAULT_DETECTED),
            1,
            OK,
        ),
        step("inhibited", ai(), inhibit(true), 0, OK),
        step(
            "a fault, inhibited",
            ai(),
            fault(Reliability::OVER_RANGE),
            0,
            denied(),
        ),
        step("not inhibited", ai(), inhibit(false), 0, OK),
        step("out of service", ai(), oos(true), 1, OK),
        step(
            "a fault, out of service",
            ai(),
            fault(Reliability::OVER_RANGE),
            0,
            denied(),
        ),
        step("back in service", ai(), oos(false), 1, OK),
        step("a fault again", ai(), fault(Reliability::OVER_RANGE), 1, OK),
        step(
            "a Life Safety fault",
            lsp(),
            fault(Reliability::OPEN_LOOP),
            1,
            OK,
        ),
        step(
            "an object that takes none",
            group(),
            fault(Reliability::NO_SENSOR),
            0,
            unsupported(),
        ),
    ]
}

/// An Integer Value read-only and commandable: the read-only one takes the
/// application's value and Reliability in service and a client's
/// simulation out of it; the commandable one takes commands at their
/// priorities and no application value.
pub(crate) fn integer_value_in_both_access_modes() -> Vec<Step> {
    let no_sensor = || fault(Reliability::NO_SENSOR);
    vec![
        step("an input", iv1(), input(signed(5)), 1, OK),
        step("a REAL input", iv1(), input(real(5.0)), 0, mistyped()),
        step("a command in service", iv1(), at(signed(6), 8), 0, denied()),
        step("a fault", iv1(), no_sensor(), 1, OK),
        step("out of service", iv1(), oos(true), 1, OK),
        step(
            "an input, out of service",
            iv1(),
            input(signed(7)),
            0,
            denied(),
        ),
        step("simulated", iv1(), at(signed(8), 8), 1, OK),
        step("a fault, out of service", iv1(), no_sensor(), 0, denied()),
        step("back in service", iv1(), oos(false), 1, OK),
        step(
            "an input, commandable",
            iv2(),
            input(signed(1)),
            0,
            unsupported(),
        ),
        step("command at 8", iv2(), at(signed(3), 8), 1, OK),
        step("command below, at 12", iv2(), at(signed(4), 12), 0, OK),
        step("relinquish 8", iv2(), relinquish(8), 1, OK),
        step(
            "relinquish 12, to the default",
            iv2(),
            relinquish(12),
            1,
            OK,
        ),
        step("priority 17", iv2(), at(signed(3), 17), 0, range()),
        step("a fault, commandable", iv2(), no_sensor(), 1, OK),
    ]
}

/// With no Device in the database the application's value is taken with
/// no Value_Source, a command fails closed in a commandable object, and a
/// writable one takes the write. A NULL to a writable value is taken and
/// changes nothing, and an unknown object is refused before anything.
/// BV-2's event detection is off, so ACTIVE, its default alarm value,
/// raises no alarm on the server.
pub(crate) fn no_device_a_null_and_an_unknown_object() -> Vec<Step> {
    let unknown = || Err((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT));
    let gone = oid(ObjectType::ANALOG_INPUT, 99);
    vec![
        step("event detection off", bv2(), detection(false), 0, OK),
        step("a writable value", bv2(), at(enumerated(1), 8), 1, OK),
        step(
            "a NULL, at a priority",
            bv2(),
            at(PropertyValue::Null, 8),
            0,
            OK,
        ),
        step(
            "a NULL",
            bv2(),
            Change::Command(PropertyValue::Null, None),
            0,
            OK,
        ),
        step("an input from the Device", av2(), input(real(4.0)), 1, OK),
        step("an input, no object", gone, input(real(1.0)), 0, unknown()),
        step(
            "a Reliability, no object",
            gone,
            fault(Reliability::NO_SENSOR),
            0,
            unknown(),
        ),
        step("a command, no object", gone, at(real(1.0), 8), 0, unknown()),
        Step::RemoveDevice,
        step("an input, no Device", av2(), input(real(5.0)), 1, OK),
        step(
            "a command, no Device",
            msv(),
            at(unsigned(2), 8),
            0,
            denied(),
        ),
        step(
            "a writable value, no Device",
            bv2(),
            Change::Command(enumerated(0), None),
            1,
            OK,
        ),
    ]
}

/// A Binary Output: no application value; a command stored in service and
/// out of it, with its Value_Source; a state out of range; the
/// application's Reliability in service and not out of it.
pub(crate) fn an_output() -> Vec<Step> {
    vec![
        step("an input", bo1(), input(enumerated(1)), 0, unsupported()),
        step("command at 8", bo1(), at(enumerated(1), 8), 1, OK),
        step("state 2", bo1(), at(enumerated(2), 8), 0, range()),
        step("a fault", bo1(), fault(Reliability::NO_SENSOR), 1, OK),
        step("out of service", bo1(), oos(true), 1, OK),
        step(
            "a fault, OOS",
            bo1(),
            fault(Reliability::OVER_RANGE),
            0,
            denied(),
        ),
        step(
            "command above, at 6, OOS",
            bo1(),
            at(enumerated(0), 6),
            1,
            OK,
        ),
        step("back in service", bo1(), oos(false), 1, OK),
        step("relinquish 6", bo1(), relinquish(6), 1, OK),
        step("relinquish 8, to the default", bo1(), relinquish(8), 1, OK),
    ]
}

/// A Binary and a Multi-state Input, as the Analog Input's rows: the
/// application's value and Reliability in service, neither out of
/// service, where a command simulates the value; a command refused in
/// service; a state out of range. A binary state is an Enumerated, a
/// multi-state one an Unsigned.
pub(crate) fn the_other_inputs() -> Vec<Step> {
    type Kind = fn(u32) -> PropertyValue;
    let inputs: [(ObjectIdentifier, Kind, u32, u32); 2] =
        [(bi1(), enumerated, 1, 0), (msi1(), unsigned, 2, 3)];
    let mut steps = Vec::new();
    for (object, kind, value, other) in inputs {
        let (value, other) = (kind(value), kind(other));
        steps.extend([
            step("event detection off", object, detection(false), 0, OK),
            step(
                "a command, in service",
                object,
                at(value.clone(), 8),
                0,
                denied(),
            ),
            step("an input", object, input(value), 1, OK),
            step("a fault", object, fault(Reliability::NO_SENSOR), 1, OK),
            step("out of service", object, oos(true), 1, OK),
            step("an input, OOS", object, input(other.clone()), 0, denied()),
            step(
                "a fault, OOS",
                object,
                fault(Reliability::OVER_RANGE),
                0,
                denied(),
            ),
            step("simulated", object, at(other, 8), 1, OK),
            step("back in service", object, oos(false), 1, OK),
            step("state 4", object, input(kind(4)), 0, range()),
        ]);
    }
    steps
}

/// The Values: a read-only one refuses a command in service and an
/// application value out of it; a writable one takes the application's
/// value; a writable Integer Value takes a command at any priority as its
/// value and a NULL as nothing; a commandable one written with no priority
/// commands slot 16; each takes the application's Reliability; a binary
/// state 2 is out of range.
pub(crate) fn the_values() -> Vec<Step> {
    let null = || Change::Command(PropertyValue::Null, None);
    vec![
        step("AV-2 event detection off", av2(), detection(false), 0, OK),
        step(
            "AV-2 command, in service",
            av2(),
            at(real(1.0), 8),
            0,
            denied(),
        ),
        step("AV-2 input", av2(), input(real(3.0)), 1, OK),
        step("AV-2 fault", av2(), fault(Reliability::NO_SENSOR), 1, OK),
        step("AV-2 out of service", av2(), oos(true), 1, OK),
        step("AV-2 input, OOS", av2(), input(real(4.0)), 0, denied()),
        step(
            "AV-2 fault, OOS",
            av2(),
            fault(Reliability::OVER_RANGE),
            0,
            denied(),
        ),
        step("AV-2 back in service", av2(), oos(false), 1, OK),
        step("BV-2 event detection off", bv2(), detection(false), 0, OK),
        step("BV-2 input", bv2(), input(enumerated(1)), 1, OK),
        step("BV-2 fault", bv2(), fault(Reliability::NO_SENSOR), 1, OK),
        step("IV-3 input", iv3(), input(signed(2)), 1, OK),
        step("IV-3 command at 8", iv3(), at(signed(3), 8), 1, OK),
        step("IV-3 NULL", iv3(), null(), 0, OK),
        step("IV-3 fault", iv3(), fault(Reliability::NO_SENSOR), 1, OK),
        step(
            "IV-2 no priority, slot 16",
            iv2(),
            Change::Command(signed(9), None),
            1,
            OK,
        ),
        step("IV-2 relinquish 16", iv2(), null(), 1, OK),
        step("BV-1 state 2", bv1(), at(enumerated(2), 8), 0, range()),
    ]
}
