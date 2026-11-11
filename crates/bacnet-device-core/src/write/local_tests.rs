//! The staged local write against a borrowed database (#2446): its checks
//! and their order, what the observer hears, the source a write is made
//! as, the commands an application makes at its priorities, the Reliability
//! it sets and the Input_Reference judged again after a write.
use super::*;
use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_objects::accumulator::PulseConverterObject;
use bacnet_objects::analog::{AnalogInputObject, AnalogValueObject};
use bacnet_objects::averaging::AveragingObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::device_identity::{DeviceIdentity, build_database_with_extra};
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, PropertyIdentifier as P, Reliability};
use bacnet_types::error::RemoteError;
use bytes::BytesMut;

const DEVICE: u32 = 1234;

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn device() -> ObjectIdentifier {
    oid(ObjectType::DEVICE, DEVICE)
}

fn ai1() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

fn av2() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 2)
}

fn bv1() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 1)
}

fn bv2() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 2)
}

fn msv1() -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_VALUE, 1)
}

fn averaging() -> ObjectIdentifier {
    oid(ObjectType::AVERAGING, 1)
}

fn permissive() -> ObjectIdentifier {
    oid(ObjectType::DEVICE, 4_000_000)
}

/// A custom Device-typed object, instance 4000000, whose writer takes every
/// value, so only the executor-owned gate refuses a write to it. It is above
/// Device 1234, which an unbound database takes as its local Device.
struct Permissive;

impl BACnetObject for Permissive {
    fn object_identifier(&self) -> ObjectIdentifier {
        permissive()
    }
    fn object_name(&self) -> &str {
        "permissive"
    }
    fn read_property(&self, _: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        Ok(PropertyValue::Null)
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Ok(())
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::rows(&[P::DESCRIPTION], &[P::DESCRIPTION])
    }
}

/// The objects the tests write: AI-1, AV-2 (read-only, tracking its
/// source), BV-1 (writable), BV-2 (commandable), MSV-1 (commandable, three
/// states) and AVG-1.
fn objects() -> Vec<Box<dyn BACnetObject>> {
    let mut av = AnalogValueObject::with_access(
        2,
        "AV-2",
        EngineeringUnits::DEGREES_CELSIUS,
        PresentValueAccess::ReadOnly,
    )
    .unwrap();
    av.set_value_source_tracking(true);
    vec![
        Box::new(AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap()),
        Box::new(av),
        Box::new(BinaryValueObject::with_access(1, "BV-1", PresentValueAccess::Writable).unwrap()),
        Box::new(BinaryValueObject::new(2, "BV-2").unwrap()),
        Box::new(MultiStateValueObject::new(1, "MSV-1", 3).unwrap()),
        Box::new(AveragingObject::new(1, "AVG-1").unwrap()),
    ]
}

/// Device 1234, [`objects`] and [`Permissive`].
fn database() -> ObjectDatabase {
    let identity = DeviceIdentity::new(DEVICE, 0x105).unwrap();
    let mut objects = objects();
    objects.push(Box::new(Permissive));
    build_database_with_extra(&identity, objects).unwrap()
}

/// [`objects`] alone: no Device-typed object at all.
fn deviceless() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    for object in objects() {
        db.add(object).unwrap();
    }
    db
}

fn refused(result: Result<Applied, Error>) -> (ErrorClass, ErrorCode) {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) => (class, code),
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn ok(result: Result<Applied, Error>) -> Applied {
    result.unwrap_or_else(|error| panic!("refused: {error:?}"))
}

fn read(db: &ObjectDatabase, object: ObjectIdentifier, property: P) -> PropertyValue {
    db.get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

fn input(value: PropertyValue) -> LocalChange {
    LocalChange::Input(LocalInputValue::present_value(value))
}

/// A write of `value` to `property` at `priority`, as the local Device.
fn write(property: P, value: PropertyValue, priority: Option<u8>) -> LocalChange {
    LocalChange::Property {
        property,
        array_index: None,
        value,
        priority,
        source: LocalCommandSource::ServerDevice,
    }
}

/// What the observer heard, in order, and what it answers
/// `commit_policy` with.
#[derive(Default)]
struct Heard {
    calls: Vec<&'static str>,
    octets: Vec<u8>,
    policy: Option<Result<(), Error>>,
}

impl WriteCommitObserver for Heard {
    fn before(&mut self, _: &ObjectDatabase, write: WriteTarget<'_>) {
        self.calls.push("before");
        self.octets = write.value.to_vec();
    }
    fn commit_policy(
        &mut self,
        _: &mut ObjectDatabase,
        _: WriteTarget<'_>,
        _: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.calls.push("commit_policy");
        self.policy.take()
    }
    fn committed(&mut self, _: &mut ObjectDatabase) {
        self.calls.push("committed");
    }
    fn failed(&mut self, _: &mut ObjectDatabase, _: &Error) {
        self.calls.push("failed");
    }
    fn applied(&mut self, _: &ObjectDatabase, _: ObjectIdentifier) {
        self.calls.push("applied");
    }
}

/// [`apply`] with `heard` as the observer, shown `octets`.
fn heard(
    db: &mut ObjectDatabase,
    object: ObjectIdentifier,
    change: LocalChange,
    heard: &mut Heard,
    octets: &[u8],
) -> Result<Applied, Error> {
    let observed = Observed {
        observer: heard,
        octets,
    };
    apply(db, object, change, Some(observed))
}

/// An unknown object is refused first, and the observer hears nothing.
#[test]
fn an_unknown_object_is_refused_before_the_observer() {
    let mut db = database();
    let missing = oid(ObjectType::ANALOG_INPUT, 99);
    let unknown = (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT);
    let real = PropertyValue::Real(1.0);
    let result = apply(&mut db, missing, input(real.clone()), None);
    assert_eq!(refused(result), unknown);
    let mut observer = Heard::default();
    let change = write(P::PRESENT_VALUE, real, Some(8));
    let result = heard(&mut db, missing, change, &mut observer, &[]);
    assert_eq!(refused(result), unknown);
    assert!(observer.calls.is_empty(), "{:?}", observer.calls);
}

/// A property write's array index is checked before its new name, as
/// WriteProperty checks them (#1426), and neither refusal reaches the
/// observer or the object.
#[test]
fn the_index_then_the_name_are_checked_before_the_observer() {
    let mut db = database();
    let mut observer = Heard::default();
    let taken = PropertyValue::CharacterString("BV-1".into());
    let indexed = LocalChange::Property {
        property: P::OBJECT_NAME,
        array_index: Some(1),
        value: taken.clone(),
        priority: None,
        source: LocalCommandSource::ServerDevice,
    };
    let result = heard(&mut db, ai1(), indexed, &mut observer, &[]);
    assert_eq!(
        refused(result),
        (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
    );
    let rename = write(P::OBJECT_NAME, taken, None);
    let result = heard(&mut db, ai1(), rename, &mut observer, &[]);
    assert_eq!(
        refused(result),
        (ErrorClass::PROPERTY, ErrorCode::DUPLICATE_NAME)
    );
    assert!(observer.calls.is_empty(), "{:?}", observer.calls);
    assert_eq!(
        read(&db, ai1(), P::OBJECT_NAME),
        PropertyValue::CharacterString("AI-1".into())
    );
}

/// An input is taken in service and refused out of it, and reports to no
/// observer even when one is given.
#[test]
fn an_input_is_taken_in_service_and_reports_to_no_observer() {
    let mut db = database();
    let mut observer = Heard::default();
    let real = PropertyValue::Real(22.5);
    let result = heard(&mut db, ai1(), input(real.clone()), &mut observer, &[]);
    assert_eq!(ok(result), Applied::Written);
    assert_eq!(read(&db, ai1(), P::PRESENT_VALUE), real);
    let oos = write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), None);
    assert_eq!(ok(apply(&mut db, ai1(), oos, None)), Applied::Written);
    let later = input(PropertyValue::Real(30.0));
    let result = heard(&mut db, ai1(), later, &mut observer, &[]);
    assert_eq!(
        refused(result),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(read(&db, ai1(), P::PRESENT_VALUE), real);
    assert!(observer.calls.is_empty(), "{:?}", observer.calls);
}

/// The application's Present_Value comes from the local Device, which a
/// tracked noncommandable Value publishes as its source (#1552). With no
/// Device the value is taken without one; a command then fails closed in a
/// commandable object, while a writable one takes the write.
#[test]
fn an_input_comes_from_the_local_device_when_there_is_one() {
    let mut db = database();
    let result = apply(&mut db, av2(), input(PropertyValue::Real(4.0)), None);
    assert_eq!(ok(result), Applied::Written);
    // Value_Source naming the Device: its object choice, in brackets.
    let source = read(&db, av2(), P::VALUE_SOURCE);
    let named = [&[0x1E, 0x1C][..], &device().encode(), &[0x1F]].concat();
    assert_eq!(source, PropertyValue::ApplicationData(named));

    let mut db = deviceless();
    let result = apply(&mut db, av2(), input(PropertyValue::Real(4.0)), None);
    assert_eq!(ok(result), Applied::Written);
    assert_eq!(read(&db, av2(), P::PRESENT_VALUE), PropertyValue::Real(4.0));
    assert_ne!(read(&db, av2(), P::VALUE_SOURCE), source);
    let active = PropertyValue::Enumerated(1);
    let command = write(P::PRESENT_VALUE, active.clone(), Some(8));
    assert_eq!(
        refused(apply(&mut db, bv2(), command, None)),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    let plain = write(P::PRESENT_VALUE, active.clone(), None);
    assert_eq!(ok(apply(&mut db, bv1(), plain, None)), Applied::Written);
    assert_eq!(read(&db, bv1(), P::PRESENT_VALUE), active);
}

/// A command above an active one takes over, one below it waits, and
/// relinquishing them in turn leaves Relinquish_Default. A priority
/// outside 1 to 16 is refused.
#[test]
fn commands_stack_by_priority_and_relinquish_to_the_default() {
    let mut db = database();
    let state = PropertyValue::Unsigned;
    let mut at = |value: PropertyValue, priority: u8| {
        let change = write(P::PRESENT_VALUE, value, Some(priority));
        apply(&mut db, msv1(), change, None)
    };
    assert_eq!(ok(at(state(2), 8)), Applied::Written);
    assert_eq!(ok(at(state(3), 12)), Applied::Written);
    for priority in [0, 17] {
        assert_eq!(
            refused(at(state(3), priority)),
            (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
        );
    }
    assert_eq!(ok(at(PropertyValue::Null, 8)), Applied::Written);
    assert_eq!(read(&db, msv1(), P::PRESENT_VALUE), state(3));
    let mut at = |value: PropertyValue, priority: u8| {
        let change = write(P::PRESENT_VALUE, value, Some(priority));
        apply(&mut db, msv1(), change, None)
    };
    assert_eq!(ok(at(PropertyValue::Null, 12)), Applied::Written);
    assert_eq!(read(&db, msv1(), P::PRESENT_VALUE), state(1));
}

/// An executor-owned Device property is refused after `before`, as a
/// failed write; any other write the object takes is committed, and
/// `applied` is never called.
#[test]
fn the_observer_hears_each_outcome_but_applied() {
    let mut db = database();
    let mut observer = Heard::default();
    let services = write(P::PROTOCOL_SERVICES_SUPPORTED, PropertyValue::Null, None);
    let result = heard(&mut db, permissive(), services, &mut observer, &[0x00]);
    assert_eq!(
        refused(result),
        (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED)
    );
    assert_eq!(observer.calls, ["before", "commit_policy", "failed"]);
    let mut observer = Heard::default();
    let text = PropertyValue::CharacterString("d".into());
    let octets = [0x75, 0x02, 0x00, b'd'];
    let description = write(P::DESCRIPTION, text, None);
    let result = heard(&mut db, permissive(), description, &mut observer, &octets);
    assert_eq!(ok(result), Applied::Written);
    assert_eq!(observer.calls, ["before", "commit_policy", "committed"]);
    assert_eq!(observer.octets, octets);
}

/// A NULL to a property that isn't commandable and has no NULL in its
/// datatype succeeds unchanged (#1396): the observer hears it committed, and
/// nothing changed.
#[test]
fn a_null_the_property_leaves_as_it_was_succeeds_unchanged() {
    let mut db = database();
    let mut observer = Heard::default();
    let null = write(P::OBJECT_NAME, PropertyValue::Null, None);
    let result = heard(&mut db, bv1(), null, &mut observer, &[0x00]);
    assert_eq!(ok(result), Applied::Unchanged);
    assert_eq!(observer.calls, ["before", "commit_policy", "committed"]);
    assert_eq!(
        read(&db, bv1(), P::OBJECT_NAME),
        PropertyValue::CharacterString("BV-1".into())
    );
}

/// An answer from `commit_policy` is the write's result: the object's
/// writer doesn't run.
#[test]
fn a_policy_commit_owns_the_result() {
    let mut db = database();
    let active = PropertyValue::Enumerated(1);
    let mut observer = Heard {
        policy: Some(Ok(())),
        ..Heard::default()
    };
    let change = write(P::PRESENT_VALUE, active.clone(), None);
    let result = heard(&mut db, bv1(), change, &mut observer, &[]);
    assert_eq!(ok(result), Applied::Written);
    assert_eq!(observer.calls, ["before", "commit_policy", "committed"]);
    assert_eq!(
        read(&db, bv1(), P::PRESENT_VALUE),
        PropertyValue::Enumerated(0)
    );
    let denied = Error::protocol(ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED);
    let mut observer = Heard {
        policy: Some(Err(denied)),
        ..Heard::default()
    };
    let change = write(P::PRESENT_VALUE, active, None);
    let result = heard(&mut db, bv1(), change, &mut observer, &[]);
    assert_eq!(
        refused(result),
        (ErrorClass::SERVICES, ErrorCode::SERVICE_REQUEST_DENIED)
    );
    assert_eq!(observer.calls, ["before", "commit_policy", "failed"]);
}

/// A rename moves the object in the name index once it commits.
#[test]
fn a_rename_moves_the_name_index() {
    let mut db = database();
    let name = PropertyValue::CharacterString("AI-renamed".into());
    let rename = write(P::OBJECT_NAME, name, None);
    assert_eq!(ok(apply(&mut db, ai1(), rename, None)), Applied::Written);
    let renamed = db
        .find_by_name("AI-renamed")
        .map(|object| object.object_identifier());
    assert_eq!(renamed, Some(ai1()));
    assert!(db.find_by_name("AI-1").is_none());
}

/// A reference naming the local Device is written in its local form, which
/// the Averaging object takes; it refuses one naming another Device.
#[test]
fn a_reference_naming_the_local_device_is_localized() {
    let mut db = database();
    let reference = |device_identifier| {
        let mut octets = BytesMut::new();
        encode_device_object_property_reference(
            &mut octets,
            &BACnetDeviceObjectPropertyReference {
                object_identifier: ai1(),
                property_identifier: P::PRESENT_VALUE.to_raw(),
                property_array_index: None,
                device_identifier,
            },
        );
        PropertyValue::ApplicationData(octets.to_vec())
    };
    let local = write(
        P::OBJECT_PROPERTY_REFERENCE,
        reference(Some(device())),
        None,
    );
    assert_eq!(
        ok(apply(&mut db, averaging(), local, None)),
        Applied::Written
    );
    let other = oid(ObjectType::DEVICE, 99);
    let remote = write(P::OBJECT_PROPERTY_REFERENCE, reference(Some(other)), None);
    assert!(apply(&mut db, averaging(), remote, None).is_err());
}

/// The application's Reliability (#2446): taken in service, where
/// Status_Flags' FAULT follows it, and refused out of service, while
/// Reliability_Evaluation_Inhibit is TRUE, and outside BACnetReliability,
/// each leaving it as it was. It reports to no observer, as an input
/// doesn't; an object that takes none refuses it.
#[test]
fn reliability_is_taken_in_service_and_reports_to_no_observer() {
    let mut db = database();
    let mut observer = Heard::default();
    let set = |reliability| LocalChange::Reliability(reliability);
    let enumerated = |reliability: Reliability| PropertyValue::Enumerated(reliability.to_raw());
    let flags = |db: &ObjectDatabase| match read(db, ai1(), P::STATUS_FLAGS) {
        PropertyValue::BitString { data, .. } => data[0] >> 4,
        other => panic!("Status_Flags is a BIT STRING: {other:?}"),
    };
    let result = heard(
        &mut db,
        ai1(),
        set(Reliability::NO_SENSOR),
        &mut observer,
        &[],
    );
    assert_eq!(ok(result), Applied::Written);
    assert_eq!(
        read(&db, ai1(), P::RELIABILITY),
        enumerated(Reliability::NO_SENSOR)
    );
    assert_eq!(flags(&db), 0b0100, "FAULT");
    let denied = (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    let range = (ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE);
    let outside = set(Reliability::from_raw(26));
    assert_eq!(refused(apply(&mut db, ai1(), outside, None)), range);
    let inhibit = P::RELIABILITY_EVALUATION_INHIBIT;
    let on = write(inhibit, PropertyValue::Boolean(true), None);
    assert_eq!(ok(apply(&mut db, ai1(), on, None)), Applied::Written);
    let inhibited = apply(&mut db, ai1(), set(Reliability::OVER_RANGE), None);
    assert_eq!(refused(inhibited), denied);
    let off = write(inhibit, PropertyValue::Boolean(false), None);
    assert_eq!(ok(apply(&mut db, ai1(), off, None)), Applied::Written);
    let oos = write(P::OUT_OF_SERVICE, PropertyValue::Boolean(true), None);
    assert_eq!(ok(apply(&mut db, ai1(), oos, None)), Applied::Written);
    let out = apply(&mut db, ai1(), set(Reliability::OVER_RANGE), None);
    assert_eq!(refused(out), denied);
    assert_ne!(
        read(&db, ai1(), P::RELIABILITY),
        enumerated(Reliability::OVER_RANGE)
    );
    let result = heard(
        &mut db,
        averaging(),
        set(Reliability::NO_SENSOR),
        &mut observer,
        &[],
    );
    assert_eq!(
        refused(result),
        (
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
        )
    );
    assert!(observer.calls.is_empty(), "{:?}", observer.calls);
}

/// After a write of a Pulse Converter's Input_Reference commits, the
/// database judges the reference at once (#1341): one naming a REAL is a
/// CONFIGURATION_ERROR, one naming an Unsigned is no fault.
#[test]
fn an_input_reference_write_is_judged_as_it_commits() {
    let mut db = database();
    let converter = PulseConverterObject::new(1, "PC-1", EngineeringUnits::NO_UNITS).unwrap();
    let pc1 = converter.object_identifier();
    db.add(Box::new(converter)).unwrap();
    let reliability = |db: &ObjectDatabase| read(db, pc1, P::RELIABILITY);
    let no_fault = PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw());
    assert_eq!(reliability(&db), no_fault);
    // [0] the object, [1] present-value.
    let reference = |object: ObjectIdentifier| {
        let named = [&[0x0C][..], &object.encode(), &[0x19, 0x55]].concat();
        write(
            P::INPUT_REFERENCE,
            PropertyValue::ApplicationData(named),
            None,
        )
    };
    for (named, expected) in [
        (ai1(), Reliability::CONFIGURATION_ERROR),
        (msv1(), Reliability::NO_FAULT_DETECTED),
    ] {
        assert_eq!(
            ok(apply(&mut db, pc1, reference(named), None)),
            Applied::Written
        );
        assert_eq!(
            reliability(&db),
            PropertyValue::Enumerated(expected.to_raw()),
            "{named:?}"
        );
    }
}
