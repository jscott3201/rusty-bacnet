//! Local writes (#2446): the server's `set_present_value_local`,
//! `set_reliability_local` and `write_local`, and bacnet-device-core's
//! `Reporter::apply_local`, given the same change to the same fixture at
//! one paused instant, end alike: the same result, the same properties read
//! back afterwards and the same COV notifications to a subscriber of each
//! object.
//!
//! Each step states how many notifications it calls for, so a change both
//! sides get wrong the same way fails too. The steps are in
//! `tests/support/local_write_cases.rs`, which the endpoint session runs as
//! well (#2468).
use super::*;
use bacnet_device_core::cov::Delivery;
use bacnet_device_core::report::Bindings;
use bacnet_device_core::write::local::LocalCommandSource;
use bacnet_time::Instant;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{EventState, Reliability};
use cases::{Change, Outcome, READ_BACK, Step};

/// The cases every host runs, which bacnet-endpoint includes too.
#[path = "../../../../tests/support/local_write_cases.rs"]
mod cases;
mod change_back;
mod points;

const PV: P = P::PRESENT_VALUE;

/// [`cases::database`], reading the fixed clock.
fn local_fixture(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let mut db = cases::database(identity);
    db.set_clock_reader(Some(Arc::new(FixedClock)));
    db
}

impl CovParity {
    /// Both sides of [`local_fixture`] under the default policy, the
    /// server's database reading the fixed clock the core's reads, and one
    /// unconfirmed SubscribeCOV from one subscriber on each object the
    /// changes make.
    async fn start_local() -> Self {
        Self::start_local_with(false).await
    }

    /// [`start_local`](Self::start_local), the `Reporter`'s event path on
    /// when `swept`, with its first sweep made, as the server's task makes
    /// its own at start.
    async fn start_local_with(swept: bool) -> Self {
        let mut cov = Self::start_from(CovPolicy::default(), local_fixture).await;
        // Start installs the system clock; serve the fixed one.
        cov.parity
            .server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(FixedClock)));
        if swept {
            let config = &cov.parity.server.config;
            let report = ReportConfig::new(config.cov_policy.clone())
                .with_confirmed(config.cov_flight_timing())
                .with_outbox(OutboxLimits {
                    entries: 64,
                    octets: 65_536,
                })
                .with_sweep(bacnet_time::Duration::from_secs(1));
            let budget = ReadPropertyMultipleBudget::default().max_result_elements;
            let responder = Responder::new(identity(), Limits::new(budget)).unwrap();
            let parity = &mut cov.parity;
            parity.reporter = Reporter::new(responder, report).unwrap();
            let now = runtime_clock::core_now();
            parity
                .reporter
                .on_deadline(&mut parity.db, &mut cov.ids, now);
        }
        let a = direct(1);
        let objects = [
            ai(),
            cases::msv(),
            cases::av2(),
            cases::bv2(),
            cases::iv1(),
            cases::iv2(),
            cases::lsp(),
            cases::iv3(),
            cases::bo1(),
            cases::bi1(),
            cases::msi1(),
        ];
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
        let server = cases::outcome(what, result);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let frames = self.parity.sent.take();
        let parity = &mut self.parity;
        let applied = parity
            .reporter
            .apply_local(&mut parity.db, object, change.core(), now);
        (
            server,
            cases::outcome(what, applied.map(|_| ())),
            frames,
            now,
        )
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

    /// Each of `steps` on both sides of a fresh [`local_fixture`], in order.
    async fn run_local(steps: Vec<Step>) {
        let mut cov = Self::start_local().await;
        for step in steps {
            match step {
                Step::Local {
                    what,
                    object,
                    change,
                    notifications,
                    expected,
                } => {
                    let what = format!("{object:?}: {what}");
                    let result = cov.local(&what, object, change, notifications).await;
                    assert_eq!(result, expected, "{what}");
                }
                Step::RemoveDevice => {
                    let device = oid(ObjectType::DEVICE, DEVICE);
                    let server = cov.parity.server.database();
                    server.write().await.remove(&device).unwrap();
                    cov.parity.db.remove(&device).unwrap();
                }
            }
        }
        cov.stop().await;
    }
}

#[tokio::test(start_paused = true)]
async fn an_input_in_and_out_of_service_matches() {
    CovParity::run_local(cases::input_in_and_out_of_service()).await;
}

#[tokio::test(start_paused = true)]
async fn commands_above_and_below_and_the_relinquish_to_the_default_match() {
    CovParity::run_local(cases::commands_and_relinquishes()).await;
}

#[tokio::test(start_paused = true)]
async fn refused_values_match() {
    CovParity::run_local(cases::refused_values()).await;
}

#[tokio::test(start_paused = true)]
async fn reliability_in_and_out_of_service_matches() {
    CovParity::run_local(cases::reliability()).await;
}

#[tokio::test(start_paused = true)]
async fn integer_value_in_both_access_modes_matches() {
    CovParity::run_local(cases::integer_value_in_both_access_modes()).await;
}

#[tokio::test(start_paused = true)]
async fn no_device_a_null_and_an_unknown_object_match() {
    CovParity::run_local(cases::no_device_a_null_and_an_unknown_object()).await;
}

/// Pinned to differ by design: a `Reporter` without the event path (no
/// sweep, as the endpoint hosts it) reports a fault as FAULT alone, where
/// the server, whose event pass moves AI-1's Event_State to FAULT, reports
/// IN_ALARM beside it. The results, the other rows and the number of
/// notifications agree. With the event path the two match
/// (`a_fault_with_the_event_path_matches`).
#[tokio::test(start_paused = true)]
async fn a_fault_without_the_event_path_differs() {
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

/// With the event path on, the `Reporter` commits AI-1's FAULT transition
/// ahead of COV, as the server does: the same notification, IN_ALARM
/// beside FAULT on both sides.
#[tokio::test(start_paused = true)]
async fn a_fault_with_the_event_path_matches() {
    let mut cov = CovParity::start_local_with(true).await;
    let what = "a fault, the event path on";
    let change = Change::Reliability(Reliability::NO_SENSOR);
    let applied = cov.local(what, ai(), change, 1).await;
    assert_eq!(applied, Ok(()));
    let in_alarm = PropertyValue::BitString {
        unused_bits: 4,
        data: vec![0b1100 << 4],
    };
    let read = |db: &ObjectDatabase| {
        let object = db.get(&ai()).expect("both hold AI-1");
        object.read_property(P::STATUS_FLAGS, None).unwrap()
    };
    let server = read(&*cov.parity.server.database().read().await);
    assert_eq!(server, in_alarm, "IN_ALARM, FAULT");
    assert_eq!(read(&cov.parity.db), in_alarm, "the core's too");
    cov.stop().await;
}

/// The core side's notifications name no Device recipient.
struct NoBindings;

impl Bindings for NoBindings {
    fn address_of(&self, _: ObjectIdentifier) -> Option<BACnetAddress> {
        None
    }
}
