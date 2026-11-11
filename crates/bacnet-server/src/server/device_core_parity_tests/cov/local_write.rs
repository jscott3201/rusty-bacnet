//! Local writes (#2446): the server's `set_present_value_local` and
//! `write_local`, and bacnet-device-core's `write::local::apply`, given the
//! same change to the same fixture at one paused instant, end alike: the
//! same result, the same properties read back afterwards and the same COV
//! notifications to a subscriber of each object.
//!
//! The `Reporter` doesn't apply a local change itself yet, so the core side
//! calls `apply` and then, for a change that wrote, `changed`. Each step
//! states how many notifications it calls for, so a change both sides get
//! wrong the same way fails too.
use super::*;
use bacnet_device_core::write::Applied;
use bacnet_device_core::write::local::{self, LocalChange, LocalCommandSource};
use bacnet_objects::traits::LocalInputValue;
use bacnet_types::error::RemoteError;

const PV: P = P::PRESENT_VALUE;

/// What both sides read back after each change.
const READ_BACK: [P; 8] = [
    P::PRESENT_VALUE,
    P::PRIORITY_ARRAY,
    P::CURRENT_COMMAND_PRIORITY,
    P::STATUS_FLAGS,
    P::RELIABILITY,
    P::VALUE_SOURCE,
    P::LAST_COMMAND_TIME,
    P::OUT_OF_SERVICE,
];

fn msv() -> ObjectIdentifier {
    oid(ObjectType::MULTI_STATE_VALUE, 1)
}

/// One local change, as each host is handed it.
#[derive(Clone)]
enum Change {
    /// The application's Present_Value: `set_present_value_local`.
    Input(PropertyValue),
    /// `write_local` of Present_Value as the Device, at the priority.
    Command(PropertyValue, Option<u8>),
    /// `write_local` of Out_Of_Service as the Device.
    OutOfService(bool),
}

impl Change {
    /// The change as bacnet-device-core takes it.
    fn core(self) -> LocalChange {
        let (property, value, priority) = match self {
            Self::Input(value) => {
                return LocalChange::Input(LocalInputValue::present_value(value));
            }
            Self::Command(value, priority) => (PV, value, priority),
            Self::OutOfService(oos) => (P::OUT_OF_SERVICE, PropertyValue::Boolean(oos), None),
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

/// A refusal's class and code; `Ok` for success.
type Outcome = Result<(), (ErrorClass, ErrorCode)>;

fn outcome(what: &str, result: Result<(), Error>) -> Outcome {
    result.map_err(|error| match error {
        Error::Remote(RemoteError::Error { class, code }) => (class, code),
        other => panic!("{what}: not a refusal: {other:?}"),
    })
}

impl CovParity {
    /// Both sides under the default policy, the server's database reading
    /// the fixed clock the core's reads, and one unconfirmed SubscribeCOV
    /// from one subscriber on each of AI-1 and MSV-1.
    async fn start_local() -> Self {
        let mut cov = Self::start(CovPolicy::default()).await;
        // Start installs the system clock; serve the fixed one.
        cov.parity
            .server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        let a = direct(1);
        for (process, object) in [(1, ai()), (2, msv())] {
            let body = subscribe(process, object, Some(false), Some(600));
            let (reply, initial) = cov.exchange("subscribe", &a, SUBSCRIBE, &body).await;
            assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
            assert_eq!(initial, 1, "the initial report");
        }
        cov
    }

    /// Make `change` to `object` on both sides at one instant. The results
    /// agree, the notifications match and number `notifications`, and the
    /// [`READ_BACK`] rows read alike afterwards. The result.
    async fn local(
        &mut self,
        what: &str,
        object: ObjectIdentifier,
        change: Change,
        notifications: usize,
    ) -> Outcome {
        let now = runtime_clock::core_now();
        let server = &self.parity.server;
        let source = LocalCommandSource::ServerDevice;
        let result = match change.clone() {
            Change::Input(value) => server.set_present_value_local(&object, value).await,
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
        };
        let server = outcome(what, result);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let frames = self.parity.sent.take();
        let parity = &mut self.parity;
        let applied = local::apply(&mut parity.db, object, change.core(), None);
        if matches!(applied, Ok(Applied::Written)) {
            parity.reporter.changed(&parity.db, &[object], now);
        }
        let core = outcome(what, applied.map(|_| ()));
        assert_eq!(server, core, "{what}: the result, server then core");
        let sent = self.same_notifications(what, &frames, now);
        assert_eq!(sent, notifications, "{what}: the notifications");
        let db = self.parity.server.database().read().await;
        for property in READ_BACK {
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
        server
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
