//! Local writes (#2446): the server's `set_present_value_local`,
//! `set_reliability_local` and `write_local`, and bacnet-device-core's
//! `Reporter::apply_local`, given the same change to the same fixture at
//! one paused instant, end alike: the same result, the same properties read
//! back afterwards and the same COV notifications to a subscriber of each
//! object.
//!
//! Each step states how many notifications it calls for, so a change both
//! sides get wrong the same way fails too.
use super::*;
use bacnet_device_core::cov::Delivery;
use bacnet_device_core::report::Bindings;
use bacnet_device_core::write::local::{LocalChange, LocalCommandSource};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::present_value_access::PresentValueAccess;
use bacnet_objects::traits::LocalInputValue;
use bacnet_objects::value_types::IntegerValueObject;
use bacnet_time::Instant;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{EventState, Reliability};
use bacnet_types::error::RemoteError;

const PV: P = P::PRESENT_VALUE;

/// What both sides read back after each change. Event_State shows that
/// the server's event pass, which the `Reporter` has yet to run (#2303),
/// moved nothing.
const READ_BACK: [P; 9] = [
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

fn msv() -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_VALUE, 1)
}

/// AV-2: read-only, publishing its source.
fn av2() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_VALUE, 2)
}

/// BV-2: writable.
fn bv2() -> ObjectIdentifier {
    oid(ObjectType::BINARY_VALUE, 2)
}

/// IV-1: read-only.
fn iv1() -> ObjectIdentifier {
    oid(ObjectType::INTEGER_VALUE, 1)
}

/// IV-2: commandable.
fn iv2() -> ObjectIdentifier {
    oid(ObjectType::INTEGER_VALUE, 2)
}

fn lsp() -> ObjectIdentifier {
    oid(ObjectType::LIFE_SAFETY_POINT, 1)
}

/// The COV fixture with [`av2`], [`bv2`], [`iv1`] and [`iv2`] besides.
fn local_fixture(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let mut db = fixture(identity);
    let mut av = AnalogValueObject::with_access(
        2,
        "AV-2",
        EngineeringUnits::DEGREES_CELSIUS,
        PresentValueAccess::ReadOnly,
    )
    .unwrap();
    av.set_value_source_tracking(true);
    let more: [Box<dyn bacnet_objects::traits::BACnetObject>; 4] = [
        Box::new(av),
        Box::new(BinaryValueObject::with_access(2, "BV-2", PresentValueAccess::Writable).unwrap()),
        Box::new(IntegerValueObject::with_access(1, "IV-1", PresentValueAccess::ReadOnly).unwrap()),
        Box::new(IntegerValueObject::new(2, "IV-2").unwrap()),
    ];
    for object in more {
        db.add(object).unwrap();
    }
    db
}

/// One local change, as each host is handed it.
#[derive(Clone)]
enum Change {
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
    fn core(self) -> LocalChange {
        let (property, value, priority) = match self {
            Self::Input(value) => {
                return LocalChange::Input(LocalInputValue::present_value(value));
            }
            Self::Reliability(reliability) => return LocalChange::Reliability(reliability),
            Self::Command(value, priority) => (PV, value, priority),
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

/// A write of Event_Detection_Enable. Off, the server's event pass has
/// nothing to do for the object, as the `Reporter`, which runs none yet
/// (#2303), hasn't.
fn detection(on: bool) -> Change {
    Change::Write(P::EVENT_DETECTION_ENABLE, PropertyValue::Boolean(on))
}

/// A refusal's class and code; `Ok` for success.
type Outcome = Result<(), (ErrorClass, ErrorCode)>;

fn outcome(what: &str, result: Result<(), Error>) -> Outcome {
    result.map_err(|error| match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("{what}: not a refusal: {other:?}"),
    })
}

impl CovParity {
    /// Both sides of [`local_fixture`] under the default policy, the
    /// server's database reading the fixed clock the core's reads, and one
    /// unconfirmed SubscribeCOV from one subscriber on each object the
    /// changes make.
    async fn start_local() -> Self {
        let mut cov = Self::start_from(CovPolicy::default(), local_fixture).await;
        // Start installs the system clock; serve the fixed one.
        cov.parity
            .server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        let a = direct(1);
        let objects = [ai(), msv(), av2(), bv2(), iv1(), iv2(), lsp()];
        for (process, object) in (1..).zip(objects) {
            let body = subscribe(process, object, Some(false), Some(600));
            let (reply, initial) = cov.exchange("subscribe", &a, SUBSCRIBE, &body).await;
            assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
            assert_eq!(initial, 1, "the initial report");
        }
        cov
    }

    /// Make `change` to `object` on both sides at one instant. The results
    /// agree, the notifications match and number `notifications`, and the
    /// [`READ_BACK`] rows read alike afterwards, or the object is on
    /// neither side. The result.
    async fn local(
        &mut self,
        what: &str,
        object: ObjectIdentifier,
        change: Change,
        notifications: usize,
    ) -> Outcome {
        let (server, core, frames, now) = self.both_local(what, object, change).await;
        assert_eq!(server, core, "{what}: the result, server then core");
        let sent = self.same_notifications(what, &frames, now);
        assert_eq!(sent, notifications, "{what}: the notifications");
        self.same_rows(what, object, &[]).await;
        server
    }

    /// Make `change` to `object` on the server, then the core, at one
    /// instant: each side's result, what the server sent and the instant.
    async fn both_local(
        &mut self,
        what: &str,
        object: ObjectIdentifier,
        change: Change,
    ) -> (Outcome, Outcome, Vec<test_transport::SentFrame>, Instant) {
        let now = runtime_clock::core_now();
        let server = &self.parity.server;
        let source = LocalCommandSource::ServerDevice;
        let result = match change.clone() {
            Change::Input(value) => server.set_present_value_local(&object, value).await,
            Change::Reliability(reliability) => {
                server.set_reliability_local(&object, reliability).await
            }
            Change::Command(value, priority) => {
                server
                    .write_local(&object, PV, None, value, priority, source)
                    .await
            }
            Change::OutOfService(oos) => {
                let value = PropertyValue::Boolean(oos);
                let property = P::OUT_OF_SERVICE;
                server
                    .write_local(&object, property, None, value, None, source)
                    .await
            }
            Change::Write(property, value) => {
                server
                    .write_local(&object, property, None, value, None, source)
                    .await
            }
        };
        let server = outcome(what, result);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let frames = self.parity.sent.take();
        let parity = &mut self.parity;
        let applied = parity
            .reporter
            .apply_local(&mut parity.db, object, change.core(), now);
        (server, outcome(what, applied.map(|_| ())), frames, now)
    }

    /// The [`READ_BACK`] rows of `object` but `except` read alike on both
    /// sides, or the object is on neither.
    async fn same_rows(&self, what: &str, object: ObjectIdentifier, except: &[P]) {
        let db = self.parity.server.database().read().await;
        let held = (
            db.get(&object).is_some(),
            self.parity.db.get(&object).is_some(),
        );
        assert_eq!(held.0, held.1, "{what}: the object, server then core");
        let rows = READ_BACK
            .into_iter()
            .filter(|property| !except.contains(property));
        for property in rows.filter(|_| held.0) {
            let read = |db: &ObjectDatabase| {
                let object = db.get(&object).expect("both hold the object");
                format!("{:?}", object.read_property(property, None))
            };
            assert_eq!(
                read(&db),
                read(&self.parity.db),
                "{what}: {property:?}, server then core"
            );
        }
    }
}

/// An input is taken in service and refused out of it, where a command
/// simulates the value instead; back in service, the input is taken again.
#[tokio::test(start_paused = true)]
async fn an_input_in_and_out_of_service_matches() {
    let mut cov = CovParity::start_local().await;
    let real = PropertyValue::Real;
    let denied = Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
    let input = Change::Input;
    assert_eq!(
        cov.local("in service", ai(), input(real(22.0)), 1).await,
        Ok(())
    );
    let oos = Change::OutOfService(true);
    assert_eq!(cov.local("out of service", ai(), oos, 1).await, Ok(()));
    let refused = cov.local("input, out of service", ai(), input(real(23.0)), 0);
    assert_eq!(refused.await, denied);
    let simulated = Change::Command(real(30.0), Some(8));
    assert_eq!(cov.local("simulated", ai(), simulated, 1).await, Ok(()));
    let back = Change::OutOfService(false);
    assert_eq!(cov.local("back in service", ai(), back, 1).await, Ok(()));
    let after = cov.local("input, back in service", ai(), input(real(24.0)), 1);
    assert_eq!(after.await, Ok(()));
    let commanded = Change::Command(real(31.0), Some(8));
    let refused = cov.local("command, in service", ai(), commanded, 0);
    assert_eq!(refused.await, denied);
    cov.stop().await;
}

/// A command below the active one waits and one above it takes over; each
/// relinquish hands Present_Value to the next, and the last to
/// Relinquish_Default.
#[tokio::test(start_paused = true)]
async fn commands_above_and_below_and_the_relinquish_to_the_default_match() {
    let mut cov = CovParity::start_local().await;
    let state = PropertyValue::Unsigned;
    let at = |value, priority| Change::Command(value, Some(priority));
    let relinquish = |priority| Change::Command(PropertyValue::Null, Some(priority));
    for (what, change, notifications) in [
        ("command at 8", at(state(2), 8), 1),
        ("command below, at 12", at(state(3), 12), 0),
        ("command above, at 6", at(state(3), 6), 1),
        ("relinquish 6", relinquish(6), 1),
        ("relinquish 8", relinquish(8), 1),
        ("relinquish 12, to the default", relinquish(12), 1),
    ] {
        assert_eq!(cov.local(what, msv(), change, notifications).await, Ok(()));
    }
    cov.stop().await;
}

/// Values the objects refuse, refused alike and reported to no one: a
/// non-finite or mistyped input, a state out of range or a priority outside
/// 1 to 16, and an input to a commandable Present_Value.
#[tokio::test(start_paused = true)]
async fn refused_values_match() {
    let mut cov = CovParity::start_local().await;
    let range = Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE));
    let mistyped = Err((ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE));
    let state = PropertyValue::Unsigned;
    for (what, object, change, expected) in [
        (
            "NaN",
            ai(),
            Change::Input(PropertyValue::Real(f32::NAN)),
            range,
        ),
        (
            "a BOOLEAN",
            ai(),
            Change::Input(PropertyValue::Boolean(true)),
            mistyped,
        ),
        (
            "state 4 of 3",
            msv(),
            Change::Command(state(4), Some(8)),
            range,
        ),
        ("state 0", msv(), Change::Command(state(0), Some(8)), range),
        (
            "priority 0",
            msv(),
            Change::Command(state(2), Some(0)),
            range,
        ),
        (
            "priority 17",
            msv(),
            Change::Command(state(2), Some(17)),
            range,
        ),
        (
            "an input to a commandable value",
            msv(),
            Change::Input(state(2)),
            Err((
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            )),
        ),
    ] {
        assert_eq!(cov.local(what, object, change, 0).await, expected, "{what}");
    }
    cov.stop().await;
}

/// The application's Reliability, with AI-1's event detection off: taken
/// in service, where Status_Flags' FAULT reports, and again unchanged with
/// nothing to report; refused outside BACnetReliability, while
/// Reliability_Evaluation_Inhibit is TRUE, out of service, and by an object
/// that takes none. A Life Safety Point's goes through its own COV path.
#[tokio::test(start_paused = true)]
async fn reliability_in_and_out_of_service_matches() {
    let mut cov = CovParity::start_local().await;
    let off = cov.local("event detection off", ai(), detection(false), 0);
    assert_eq!(off.await, Ok(()));
    let set = Change::Reliability;
    let denied = Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
    let inhibit = |on| {
        Change::Write(
            P::RELIABILITY_EVALUATION_INHIBIT,
            PropertyValue::Boolean(on),
        )
    };
    for (what, object, change, notifications, expected) in [
        ("a fault", ai(), set(Reliability::NO_SENSOR), 1, Ok(())),
        (
            "the same fault",
            ai(),
            set(Reliability::NO_SENSOR),
            0,
            Ok(()),
        ),
        (
            "outside the enumeration",
            ai(),
            set(Reliability::from_raw(26)),
            0,
            Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)),
        ),
        (
            "the fault cleared",
            ai(),
            set(Reliability::NO_FAULT_DETECTED),
            1,
            Ok(()),
        ),
        ("inhibited", ai(), inhibit(true), 0, Ok(())),
        (
            "a fault, inhibited",
            ai(),
            set(Reliability::OVER_RANGE),
            0,
            denied,
        ),
        ("not inhibited", ai(), inhibit(false), 0, Ok(())),
        (
            "out of service",
            ai(),
            Change::OutOfService(true),
            1,
            Ok(()),
        ),
        (
            "a fault, out of service",
            ai(),
            set(Reliability::OVER_RANGE),
            0,
            denied,
        ),
        (
            "back in service",
            ai(),
            Change::OutOfService(false),
            1,
            Ok(()),
        ),
        (
            "a fault again",
            ai(),
            set(Reliability::OVER_RANGE),
            1,
            Ok(()),
        ),
        (
            "a Life Safety fault",
            lsp(),
            set(Reliability::OPEN_LOOP),
            1,
            Ok(()),
        ),
        (
            "an object that takes none",
            oid(ObjectType::GROUP, 1),
            set(Reliability::NO_SENSOR),
            0,
            Err((
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            )),
        ),
    ] {
        let result = cov.local(what, object, change, notifications).await;
        assert_eq!(result, expected, "{what}");
    }
    cov.stop().await;
}

/// An Integer Value read-only and commandable: the read-only one takes the
/// application's value and Reliability in service and a client's
/// simulation out of it; the commandable one takes commands at their
/// priorities and no application value.
#[tokio::test(start_paused = true)]
async fn integer_value_in_both_access_modes_matches() {
    let mut cov = CovParity::start_local().await;
    let signed = PropertyValue::Signed;
    let at = |value, priority| Change::Command(value, Some(priority));
    let relinquish = |priority| Change::Command(PropertyValue::Null, Some(priority));
    let denied = Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
    let not_supported = Err((
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    ));
    let fault = Change::Reliability(Reliability::NO_SENSOR);
    for (what, object, change, notifications, expected) in [
        ("an input", iv1(), Change::Input(signed(5)), 1, Ok(())),
        (
            "a REAL input",
            iv1(),
            Change::Input(PropertyValue::Real(5.0)),
            0,
            Err((ErrorClass::PROPERTY, ErrorCode::INVALID_DATA_TYPE)),
        ),
        ("a command in service", iv1(), at(signed(6), 8), 0, denied),
        ("a fault", iv1(), fault.clone(), 1, Ok(())),
        (
            "out of service",
            iv1(),
            Change::OutOfService(true),
            1,
            Ok(()),
        ),
        (
            "an input, out of service",
            iv1(),
            Change::Input(signed(7)),
            0,
            denied,
        ),
        ("simulated", iv1(), at(signed(8), 8), 1, Ok(())),
        ("a fault, out of service", iv1(), fault.clone(), 0, denied),
        (
            "back in service",
            iv1(),
            Change::OutOfService(false),
            1,
            Ok(()),
        ),
        (
            "an input, commandable",
            iv2(),
            Change::Input(signed(1)),
            0,
            not_supported,
        ),
        ("command at 8", iv2(), at(signed(3), 8), 1, Ok(())),
        ("command below, at 12", iv2(), at(signed(4), 12), 0, Ok(())),
        ("relinquish 8", iv2(), relinquish(8), 1, Ok(())),
        (
            "relinquish 12, to the default",
            iv2(),
            relinquish(12),
            1,
            Ok(()),
        ),
        (
            "priority 17",
            iv2(),
            at(signed(3), 17),
            0,
            Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)),
        ),
        ("a fault, commandable", iv2(), fault, 1, Ok(())),
    ] {
        let result = cov.local(what, object, change, notifications).await;
        assert_eq!(result, expected, "{what}");
    }
    cov.stop().await;
}

/// With no Device in the database the application's value is taken with
/// no Value_Source, a command fails closed in a commandable object, and a
/// writable one takes the write. A NULL to a writable value is taken and
/// changes nothing, and an unknown object is refused before anything.
/// BV-2's event detection is off, so ACTIVE, its default alarm value,
/// raises no alarm on the server.
#[tokio::test(start_paused = true)]
async fn no_device_a_null_and_an_unknown_object_match() {
    let mut cov = CovParity::start_local().await;
    let off = cov.local("event detection off", bv2(), detection(false), 0);
    assert_eq!(off.await, Ok(()));
    let active = PropertyValue::Enumerated(1);
    for (what, object, change, notifications) in [
        (
            "a writable value",
            bv2(),
            Change::Command(active.clone(), Some(8)),
            1,
        ),
        (
            "a NULL, at a priority",
            bv2(),
            Change::Command(PropertyValue::Null, Some(8)),
            0,
        ),
        (
            "a NULL",
            bv2(),
            Change::Command(PropertyValue::Null, None),
            0,
        ),
        (
            "an input from the Device",
            av2(),
            Change::Input(PropertyValue::Real(4.0)),
            1,
        ),
    ] {
        assert_eq!(
            cov.local(what, object, change, notifications).await,
            Ok(()),
            "{what}"
        );
    }
    let unknown = Err((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT));
    let gone = oid(ObjectType::ANALOG_INPUT, 99);
    for (what, change) in [
        (
            "an input, no object",
            Change::Input(PropertyValue::Real(1.0)),
        ),
        (
            "a Reliability, no object",
            Change::Reliability(Reliability::NO_SENSOR),
        ),
        (
            "a command, no object",
            Change::Command(PropertyValue::Real(1.0), Some(8)),
        ),
    ] {
        assert_eq!(cov.local(what, gone, change, 0).await, unknown, "{what}");
    }
    let device = oid(ObjectType::DEVICE, DEVICE);
    cov.parity
        .server
        .database()
        .write()
        .await
        .remove(&device)
        .unwrap();
    cov.parity.db.remove(&device).unwrap();
    let inactive = PropertyValue::Enumerated(0);
    let denied = Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED));
    for (what, object, change, notifications, expected) in [
        (
            "an input, no Device",
            av2(),
            Change::Input(PropertyValue::Real(5.0)),
            1,
            Ok(()),
        ),
        (
            "a command, no Device",
            msv(),
            Change::Command(PropertyValue::Unsigned(2), Some(8)),
            0,
            denied,
        ),
        (
            "a writable value, no Device",
            bv2(),
            Change::Command(inactive, None),
            1,
            Ok(()),
        ),
    ] {
        let result = cov.local(what, object, change, notifications).await;
        assert_eq!(result, expected, "{what}");
    }
    cov.stop().await;
}

/// Pinned to differ by design until #2303's third part gives the `Reporter`
/// its event pass, ahead of COV as the server runs it: with AI-1's event
/// detection on, a fault moves its Event_State to FAULT on the server, so
/// its Status_Flags read and report IN_ALARM beside FAULT, where the
/// `Reporter` reports FAULT alone. The results, the other rows and the
/// number of notifications agree. Once the `Reporter` runs events, this
/// becomes an ordinary row.
#[tokio::test(start_paused = true)]
async fn a_fault_with_event_detection_on_differs_until_the_reporter_runs_events() {
    let mut cov = CovParity::start_local().await;
    let what = "a fault, event detection on";
    let change = Change::Reliability(Reliability::NO_SENSOR);
    let (server, core, frames, now) = cov.both_local(what, ai(), change).await;
    assert_eq!((server, core), (Ok(()), Ok(())));
    let mut sent = Vec::new();
    loop {
        let mut out = BytesMut::new();
        let reporter = &mut cov.parity.reporter;
        let Some(notification) = reporter.pop(&NoBindings, &mut cov.ids, now, &mut out) else {
            break;
        };
        reporter.delivered(notification.token, Delivery::Sent, &mut cov.ids, now);
        sent.push(out.to_vec());
    }
    let [frame] = &frames[..] else {
        panic!("the server's one notification: {frames:?}");
    };
    assert_eq!(sent.len(), 1, "the core's one notification");
    assert_ne!(frame.decode_npdu().payload.to_vec(), sent[0]);
    let flags = |flags: u8| PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags << 4],
    };
    let read = |db: &ObjectDatabase, property| {
        let object = db.get(&ai()).expect("both hold AI-1");
        object.read_property(property, None).unwrap()
    };
    let db = cov.parity.server.database().read().await;
    let fault = PropertyValue::Enumerated(EventState::FAULT.to_raw());
    let normal = PropertyValue::Enumerated(EventState::NORMAL.to_raw());
    assert_eq!(read(&db, P::STATUS_FLAGS), flags(0b1100), "IN_ALARM, FAULT");
    assert_eq!(read(&db, P::EVENT_STATE), fault);
    assert_eq!(
        read(&cov.parity.db, P::STATUS_FLAGS),
        flags(0b0100),
        "FAULT"
    );
    assert_eq!(read(&cov.parity.db, P::EVENT_STATE), normal);
    drop(db);
    let except = [P::STATUS_FLAGS, P::EVENT_STATE];
    cov.same_rows(what, ai(), &except).await;
    cov.stop().await;
}

/// The core side's notifications name no Device recipient.
struct NoBindings;

impl Bindings for NoBindings {
    fn address_of(&self, _: ObjectIdentifier) -> Option<BACnetAddress> {
        None
    }
}
