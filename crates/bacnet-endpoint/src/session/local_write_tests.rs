//! The endpoint's local writes (#2468).
//!
//! Parity is by read-back: each case of the local-write list the server and
//! bacnet-device-core run side by side (`cases`, included from
//! `bacnet-server`'s parity tests) runs on a live endpoint session and on
//! bacnet-device-core's `apply` over the same database, and each step ends
//! with the result the list gives and the same properties read back. The
//! server's own run of the list holds it to the core's rows, so the three
//! hosts agree. What the endpoint does differently by design is pinned
//! below: no event pass; a Command's, Channel's or Timer's run ended
//! unmade; and no Timer task.

use super::*;
use crate::DeviceIdentity;
use crate::session::bus_port::BusPort;
use crate::session::device_write_tests::bus_client;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::command::CommandObject;
use bacnet_objects::timer::TimerObject;
use bacnet_types::constructed::{
    BACnetActionCommand, BACnetActionList, BACnetDeviceObjectPropertyReference,
    BACnetTimerStateChangeValue,
};
use bacnet_types::enums::{
    EngineeringUnits, ErrorClass, ErrorCode, ObjectType, Reliability, TimerState, WriteStatus,
};
use bacnet_types::primitives::{Date, Time};
use cases::{Change, READ_BACK, Step};
use std::net::Ipv4Addr;

#[path = "../../../bacnet-server/tests/support/local_write_cases.rs"]
mod cases;

const P_PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const DEVICE: u32 = 4321;

/// One instant, so Last_Command_Time reads alike on both sides.
struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 11,
                day: 11,
                day_of_week: 3,
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

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_name("parity-device")
}

/// The cases' database, reading the fixed clock.
fn fixture() -> ObjectDatabase {
    let mut db = cases::database(&identity());
    db.set_clock_reader(Some(Arc::new(FixedClock)));
    db
}

/// A running server-role session over `db` on the in-memory link, and its
/// B/IP MAC.
async fn endpoint(db: ObjectDatabase) -> (EndpointSession<BusPort>, [u8; 6]) {
    let mut session =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(db)
            .build_session_over(BusPort::new())
            .unwrap();
    session.start().await.unwrap();
    let port = session.bip_local_address().unwrap().port();
    (
        session,
        bacnet_transport::bvll::encode_bip_mac([127, 0, 0, 1], port),
    )
}

/// `change` to `object` through the endpoint call the case names.
async fn call(
    session: &EndpointSession<BusPort>,
    object: ObjectIdentifier,
    change: Change,
) -> Result<(), Error> {
    let source = LocalCommandSource::ServerDevice;
    let write = |property, value, priority| {
        session.write_local(&object, property, None, value, priority, source)
    };
    match change {
        Change::Input(value) => session.set_present_value_local(&object, value).await,
        Change::Reliability(reliability) => {
            session.set_reliability_local(&object, reliability).await
        }
        Change::Command(value, priority) => write(P_PV, value, priority).await,
        Change::OutOfService(on) => {
            let value = PropertyValue::Boolean(on);
            write(PropertyIdentifier::OUT_OF_SERVICE, value, None).await
        }
        Change::Write(property, value) => write(property, value, None).await,
    }
}

/// `property` of `object` in `db`, as a comparable string.
fn row(db: &ObjectDatabase, object: ObjectIdentifier, property: PropertyIdentifier) -> String {
    let object = db.get(&object).expect("both hold the object");
    format!("{:?}", object.read_property(property, None))
}

/// The [`READ_BACK`] rows of `object` read alike on the endpoint and in
/// `core`, or the object is on neither.
async fn same_rows(
    what: &str,
    session: &EndpointSession<BusPort>,
    core: &ObjectDatabase,
    object: ObjectIdentifier,
) {
    let endpoint = session.database.as_ref().unwrap().read().await;
    let held = (endpoint.get(&object).is_some(), core.get(&object).is_some());
    assert_eq!(held.0, held.1, "{what}: the object, endpoint then core");
    for property in READ_BACK.into_iter().filter(|_| held.0) {
        assert_eq!(
            row(&endpoint, object, property),
            row(core, object, property),
            "{what}: {property:?}, endpoint then core"
        );
    }
}

/// One case of the shared list: its steps.
type Case = fn() -> Vec<Step>;

/// Every case of the shared list, each on a fresh endpoint and core.
#[tokio::test(start_paused = true)]
async fn the_shared_local_write_cases_match_the_core() {
    let all: [(&str, Case); 9] = [
        ("input", cases::input_in_and_out_of_service),
        ("commands", cases::commands_and_relinquishes),
        ("refused", cases::refused_values),
        ("reliability", cases::reliability),
        ("integer", cases::integer_value_in_both_access_modes),
        ("no device", cases::no_device_a_null_and_an_unknown_object),
        ("output", cases::an_output),
        ("inputs", cases::the_other_inputs),
        ("values", cases::the_values),
    ];
    for (name, case) in all {
        let (mut session, _) = endpoint(fixture()).await;
        let mut core = fixture();
        for step in case() {
            match step {
                Step::Local {
                    what,
                    object,
                    change,
                    expected,
                    ..
                } => {
                    let what = format!("{name}: {object:?}: {what}");
                    let endpoint = call(&session, object, change.clone()).await;
                    let applied = local::apply(&mut core, object, change.core(), None);
                    let endpoint = cases::outcome(&what, endpoint);
                    assert_eq!(endpoint, expected, "{what}: the endpoint");
                    let core_result = cases::outcome(&what, applied.map(|_| ()));
                    assert_eq!(core_result, expected, "{what}: the core");
                    same_rows(&what, &session, &core, object).await;
                }
                Step::RemoveDevice => {
                    let device = cases::oid(ObjectType::DEVICE, DEVICE);
                    let database = session.database.as_ref().unwrap();
                    database.write().await.remove(&device).unwrap();
                    core.remove(&device).unwrap();
                }
            }
        }
        session.stop().await.unwrap();
    }
}

/// Status_Flags with `flags` (IN_ALARM, FAULT, OVERRIDDEN, OUT_OF_SERVICE).
fn flags(flags: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags << 4],
    }
}

async fn read(
    session: &EndpointSession<BusPort>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let db = session.database.as_ref().unwrap().read().await;
    db.get(&object)
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// Pinned to differ from the server by design: with AI-1's event detection
/// on, the server's event pass moves a fault's Event_State to FAULT and
/// adds IN_ALARM (the server's own parity test pins that). The endpoint
/// runs none, so it keeps Event_State NORMAL and shows FAULT alone, as the
/// core does.
#[tokio::test(start_paused = true)]
async fn a_fault_with_event_detection_on_leaves_event_state_as_the_core_does() {
    let (mut session, _) = endpoint(fixture()).await;
    let mut core = fixture();
    let fault = Change::Reliability(bacnet_types::enums::Reliability::NO_SENSOR);
    call(&session, cases::ai(), fault.clone()).await.unwrap();
    local::apply(&mut core, cases::ai(), fault.core(), None).unwrap();
    let normal = PropertyValue::Enumerated(bacnet_types::enums::EventState::NORMAL.to_raw());
    assert_eq!(
        read(&session, cases::ai(), PropertyIdentifier::STATUS_FLAGS).await,
        flags(0b0100)
    );
    assert_eq!(
        read(&session, cases::ai(), PropertyIdentifier::EVENT_STATE).await,
        normal
    );
    same_rows("a fault, event detection on", &session, &core, cases::ai()).await;
    session.stop().await.unwrap();
}

fn command() -> ObjectIdentifier {
    cases::oid(ObjectType::COMMAND, 1)
}

/// Pinned to differ from the server by design: the server makes a Command
/// run's writes, where the endpoint has no task to make them in and ends
/// the run at once, unmade and unsuccessful, as bacnet-device-core's
/// `Responder` ends one a client's write starts. The object isn't left
/// busy: the next write starts and ends a run of its own.
#[tokio::test(start_paused = true)]
async fn a_command_run_a_local_write_starts_ends_unmade() {
    let mut db = fixture();
    let mut cmd = CommandObject::new(1, "CMD-1").unwrap();
    cmd.set_action(vec![BACnetActionList {
        commands: vec![BACnetActionCommand {
            device_identifier: None,
            object_identifier: cases::bv2(),
            property_identifier: P_PV,
            property_array_index: None,
            property_value: PropertyValue::Enumerated(1),
            priority: Some(8),
            post_delay: None,
            quit_on_failure: false,
            write_successful: true,
        }],
    }])
    .unwrap();
    db.add(Box::new(cmd)).unwrap();
    let (mut session, _) = endpoint(db).await;
    let source = LocalCommandSource::ServerDevice;
    for _ in 0..2 {
        let list = PropertyValue::Unsigned(1);
        session
            .write_local(&command(), P_PV, None, list, None, source)
            .await
            .unwrap();
        let off = PropertyValue::Boolean(false);
        assert_eq!(
            read(&session, command(), PropertyIdentifier::IN_PROCESS).await,
            off
        );
        assert_eq!(
            read(
                &session,
                command(),
                PropertyIdentifier::ALL_WRITES_SUCCESSFUL
            )
            .await,
            off
        );
        // BV-2 was never written.
        assert_eq!(
            read(&session, cases::bv2(), P_PV).await,
            PropertyValue::Enumerated(0)
        );
    }
    session.stop().await.unwrap();
}

fn ao1() -> ObjectIdentifier {
    cases::oid(ObjectType::ANALOG_OUTPUT, 1)
}

/// The cases' database with AO-1 and `more` besides.
fn with_output(more: Box<dyn bacnet_objects::traits::BACnetObject>) -> ObjectDatabase {
    let mut db = fixture();
    let output = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::NO_UNITS).unwrap();
    db.add(Box::new(output)).unwrap();
    db.add(more).unwrap();
    db
}

/// AO-1 as no write has left it: Present_Value at its default, no slot
/// commanded.
async fn assert_ao1_unwritten(session: &EndpointSession<BusPort>) {
    assert_eq!(read(session, ao1(), P_PV).await, PropertyValue::Real(0.0));
    let db = session.database.as_ref().unwrap().read().await;
    let slots = db
        .get(&ao1())
        .unwrap()
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, None)
        .unwrap();
    let PropertyValue::List(slots) = slots else {
        panic!("a Priority_Array: {slots:?}")
    };
    assert!(
        slots.iter().all(|slot| *slot == PropertyValue::Null),
        "{slots:?}"
    );
}

/// Pinned to differ from the server by design: a Channel's Present_Value
/// write starts a distribution the endpoint has no task to make, so it
/// ends at once, unmade, as bacnet-device-core's `Responder` ends one:
/// Write_Status FAILED, Reliability PROCESS_ERROR with FAULT, and no
/// member written. The server writes the members. A Channel with no
/// members starts no run, and stays IDLE and without a fault.
#[tokio::test(start_paused = true)]
async fn a_channel_write_ends_its_distribution_failed_with_no_member_written() {
    let ch1 = cases::oid(ObjectType::CHANNEL, 1);
    let mut channel = ChannelObject::new(1, "CH-1", 7).unwrap();
    let member = BACnetDeviceObjectPropertyReference::new_local(ao1(), P_PV.to_raw());
    channel.set_members(vec![member]).unwrap();
    let (mut session, _) = endpoint(with_output(Box::new(channel))).await;
    let source = LocalCommandSource::ServerDevice;
    let value = PropertyValue::Real(40.0);
    session
        .write_local(&ch1, P_PV, None, value, Some(8), source)
        .await
        .unwrap();
    assert_eq!(
        read(&session, ch1, PropertyIdentifier::WRITE_STATUS).await,
        PropertyValue::Enumerated(WriteStatus::FAILED.to_raw())
    );
    assert_eq!(
        read(&session, ch1, PropertyIdentifier::RELIABILITY).await,
        PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw())
    );
    assert_eq!(
        read(&session, ch1, PropertyIdentifier::STATUS_FLAGS).await,
        flags(0b0100)
    );
    assert_ao1_unwritten(&session).await;
    // A Channel with no members starts no run: nothing fails.
    let ch2 = cases::oid(ObjectType::CHANNEL, 2);
    let empty = ChannelObject::new(2, "CH-2", 8).unwrap();
    session
        .database
        .as_ref()
        .unwrap()
        .write()
        .await
        .add(Box::new(empty))
        .unwrap();
    let value = PropertyValue::Real(40.0);
    session
        .write_local(&ch2, P_PV, None, value, Some(8), source)
        .await
        .unwrap();
    assert_eq!(
        read(&session, ch2, PropertyIdentifier::WRITE_STATUS).await,
        PropertyValue::Enumerated(WriteStatus::IDLE.to_raw())
    );
    assert_eq!(
        read(&session, ch2, PropertyIdentifier::RELIABILITY).await,
        PropertyValue::Enumerated(Reliability::NO_FAULT_DETECTED.to_raw())
    );
    session.stop().await.unwrap();
}

/// Pinned to differ from the server by design: a Timer started by a local
/// write runs, but the State_Change_Values write its start owes ends at
/// once, unmade (Reliability PROCESS_ERROR with FAULT, AO-1 unwritten), and
/// with no Timer task on the endpoint it never expires; stopped, it goes
/// IDLE with that write unmade too. The server writes AO-1 and expires the
/// Timer at its time.
#[tokio::test(start_paused = true)]
async fn a_timer_started_locally_writes_nothing_and_never_expires() {
    let tmr1 = cases::oid(ObjectType::TIMER, 1);
    let mut timer = TimerObject::new(1, "TMR-1").unwrap();
    timer.set_pres_value_range(100, 600_000).unwrap();
    let real = |value| BACnetTimerStateChangeValue::Primitive(PropertyValue::Real(value));
    let target = BACnetDeviceObjectPropertyReference::new_local(ao1(), P_PV.to_raw());
    timer
        .set_state_change(core::array::from_fn(|_| real(10.0)), vec![target], 9)
        .unwrap();
    let (mut session, _) = endpoint(with_output(Box::new(timer))).await;
    let source = LocalCommandSource::ServerDevice;
    let time = PropertyValue::Unsigned(300);
    session
        .write_local(&tmr1, P_PV, None, time, None, source)
        .await
        .unwrap();
    let running = PropertyValue::Enumerated(TimerState::RUNNING.to_raw());
    assert_eq!(
        read(&session, tmr1, PropertyIdentifier::TIMER_STATE).await,
        running
    );
    assert_eq!(
        read(&session, tmr1, PropertyIdentifier::RELIABILITY).await,
        PropertyValue::Enumerated(Reliability::PROCESS_ERROR.to_raw())
    );
    assert_eq!(
        read(&session, tmr1, PropertyIdentifier::STATUS_FLAGS).await,
        flags(0b0100)
    );
    assert_ao1_unwritten(&session).await;
    // Well past its 300 ms, it still runs.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(
        read(&session, tmr1, PropertyIdentifier::TIMER_STATE).await,
        running
    );
    assert_ao1_unwritten(&session).await;
    // Stopped, it goes IDLE, and that transition's write isn't made either.
    let idle = PropertyValue::Enumerated(TimerState::IDLE.to_raw());
    let state = PropertyIdentifier::TIMER_STATE;
    session
        .write_local(&tmr1, state, None, idle.clone(), None, source)
        .await
        .unwrap();
    assert_eq!(read(&session, tmr1, state).await, idle);
    assert_ao1_unwritten(&session).await;
    session.stop().await.unwrap();
}

fn encoding(error: Error) -> String {
    match error {
        Error::Encoding(message) => message,
        other => panic!("not a session refusal: {other:?}"),
    }
}

/// The three calls need a running session with a database: before start,
/// after stop and in a client-only session with none, each is refused with
/// nothing changed.
#[tokio::test(start_paused = true)]
async fn the_calls_need_a_running_session_with_a_database() {
    let ai = cases::ai();
    let pv = PropertyValue::Real(30.0);
    let not_running = "endpoint session is not running";
    let mut session =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(fixture())
            .build_session_over(BusPort::new())
            .unwrap();
    let before = session.set_present_value_local(&ai, pv.clone()).await;
    assert_eq!(encoding(before.unwrap_err()), not_running);
    session.start().await.unwrap();
    session.stop().await.unwrap();
    let after = session.set_reliability_local(&ai, bacnet_types::enums::Reliability::NO_SENSOR);
    assert_eq!(encoding(after.await.unwrap_err()), not_running);
    let source = LocalCommandSource::ServerDevice;
    let after = session.write_local(&ai, P_PV, None, pv.clone(), Some(8), source);
    assert_eq!(encoding(after.await.unwrap_err()), not_running);
    assert_eq!(read(&session, ai, P_PV).await, PropertyValue::Real(21.5));
    assert_eq!(
        read(&session, ai, PropertyIdentifier::RELIABILITY).await,
        PropertyValue::Enumerated(0)
    );

    let mut client_only =
        crate::bip::BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ClientOnly)
            .build_session_over(BusPort::new())
            .unwrap();
    client_only.start().await.unwrap();
    let refused = client_only.set_present_value_local(&ai, pv).await;
    assert_eq!(
        encoding(refused.unwrap_err()),
        "endpoint session has no local database"
    );
    client_only.stop().await.unwrap();
}

/// Without a source Reporter the Device has no Audit_Notification_Recipient
/// for a local write to reach, as for a client's: PROPERTY /
/// UNKNOWN_PROPERTY.
#[tokio::test(start_paused = true)]
async fn an_audit_recipient_without_a_source_reporter_is_unknown() {
    let (mut session, _) = endpoint(fixture()).await;
    let device = cases::oid(ObjectType::DEVICE, DEVICE);
    let mut recipient = bytes::BytesMut::new();
    bacnet_encoding::constructed::encode_recipient(
        &mut recipient,
        &bacnet_types::constructed::BACnetRecipient::Device(cases::oid(ObjectType::DEVICE, 9)),
    )
    .unwrap();
    let value = PropertyValue::ApplicationData(recipient.to_vec());
    let property = PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT;
    let source = LocalCommandSource::ServerDevice;
    let refused = session
        .write_local(&device, property, None, value, None, source)
        .await;
    assert_eq!(
        cases::outcome("recipient", refused),
        Err((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
    session.stop().await.unwrap();
}

/// A client's ReadProperty against `mac`'s `property` of `object`.
async fn wire_read(
    client: &bacnet_client::client::BACnetClient<BusPort>,
    mac: &[u8],
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let ack = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client.read_property(mac, object, property, None),
    )
    .await
    .unwrap()
    .unwrap();
    let (value, end) =
        bacnet_encoding::primitives::decode_application_value(&ack.property_value, 0).unwrap();
    assert_eq!(end, ack.property_value.len());
    value
}

/// What a client reads of `object`: Present_Value, Status_Flags and
/// Reliability.
async fn seen(
    client: &bacnet_client::client::BACnetClient<BusPort>,
    mac: &[u8],
    object: ObjectIdentifier,
) -> Vec<PropertyValue> {
    let mut values = Vec::new();
    for property in [
        P_PV,
        PropertyIdentifier::STATUS_FLAGS,
        PropertyIdentifier::RELIABILITY,
    ] {
        values.push(wire_read(client, mac, object, property).await);
    }
    values
}

/// A client reading the endpoint sees each call's change: the value, the
/// Status_Flags and the Reliability. A refused call changes none of them.
#[tokio::test(start_paused = true)]
async fn a_client_reads_each_local_change_and_nothing_from_a_refused_one() {
    let (mut session, mac) = endpoint(fixture()).await;
    let mut client = bus_client().await;
    let (ai, av) = (cases::ai(), cases::av2());
    let no_fault = PropertyValue::Enumerated(0);

    session
        .set_present_value_local(&ai, PropertyValue::Real(30.0))
        .await
        .unwrap();
    let input = [PropertyValue::Real(30.0), flags(0), no_fault.clone()];
    assert_eq!(seen(&client, &mac, ai).await, input);

    let fault = bacnet_types::enums::Reliability::NO_SENSOR;
    session.set_reliability_local(&ai, fault).await.unwrap();
    let faulted = [
        PropertyValue::Real(30.0),
        flags(0b0100),
        PropertyValue::Enumerated(1),
    ];
    assert_eq!(seen(&client, &mac, ai).await, faulted);

    // AV-2 is read-only in service; out of service a write simulates it.
    let source = LocalCommandSource::ServerDevice;
    let oos = PropertyValue::Boolean(true);
    let property = PropertyIdentifier::OUT_OF_SERVICE;
    session
        .write_local(&av, property, None, oos, None, source)
        .await
        .unwrap();
    session
        .write_local(&av, P_PV, None, PropertyValue::Real(5.0), Some(8), source)
        .await
        .unwrap();
    let simulated = [PropertyValue::Real(5.0), flags(0b0001), no_fault];
    assert_eq!(seen(&client, &mac, av).await, simulated);

    // Refused: nothing a client reads moves.
    let outside = bacnet_types::enums::Reliability::from_raw(26);
    assert!(session.set_reliability_local(&ai, outside).await.is_err());
    let mistyped = PropertyValue::Boolean(true);
    assert!(
        session
            .set_present_value_local(&ai, mistyped)
            .await
            .is_err()
    );
    assert_eq!(seen(&client, &mac, ai).await, faulted);
    let input_oos = PropertyValue::Real(6.0);
    assert!(
        session
            .set_present_value_local(&av, input_oos)
            .await
            .is_err()
    );
    assert_eq!(seen(&client, &mac, av).await, simulated);

    client.stop().await.unwrap();
    session.stop().await.unwrap();
}
