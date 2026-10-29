//! A Command object's Present_Value write runs the action list it selects,
//! on a running server (#1150, Clause 12.10).
//!
//! CMD-1 holds seven lists, written into the harness's own Device 856:
//!
//! 1. AO-1 to 50.0 at priority 8, then AV-1 to 21.5 at priority 9.
//! 2. AO-2 to 30.0 at priority 8 with a 5-second post delay, then AO-1 to
//!    70.0 at priority 8.
//! 3. AO-9, which doesn't exist, quitting on failure; then AO-1 and AO-2 to
//!    10.0, never reached.
//! 4. AO-9 again without quitting, then AO-1 to 60.0.
//! 5. AO-1 to 80.0 in Device 9, which this server isn't and has no binding
//!    for, then AO-2 to 80.0 naming Device 856.
//! 6. Nothing.
//! 7. CMD-1's own Present_Value to 6.
//!
//! Every command starts with its write-successful flag TRUE, so a FALSE read
//! back from Action is the run's doing. Requests and the reads of In_Process,
//! All_Writes_Successful and Action go over the wire; targets are read from
//! the database. The clock is paused, so post delays pass only when a test
//! sleeps through them.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::decode_action_list;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::command::CommandObject;
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use bacnet_types::enums::{EngineeringUnits, ObjectType};

pub(super) fn cmd(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::COMMAND, instance).unwrap()
}

pub(super) fn ao(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, instance).unwrap()
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

/// A write of `value` to `object`'s Present_Value at `priority`.
pub(super) fn write(
    object: ObjectIdentifier,
    value: PropertyValue,
    priority: u8,
) -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: None,
        object_identifier: object,
        property_identifier: PV,
        property_array_index: None,
        property_value: value,
        priority: Some(priority),
        post_delay: None,
        quit_on_failure: false,
        write_successful: true,
    }
}

fn list(commands: Vec<BACnetActionCommand>) -> BACnetActionList {
    BACnetActionList { commands }
}

fn lists() -> Vec<BACnetActionList> {
    let real = PropertyValue::Real;
    let delayed = BACnetActionCommand {
        post_delay: Some(5),
        ..write(ao(2), real(30.0), 8)
    };
    let missing = write(ao(9), real(1.0), 8);
    let quitting = BACnetActionCommand {
        quit_on_failure: true,
        ..missing.clone()
    };
    let in_device = |instance, command: BACnetActionCommand| BACnetActionCommand {
        device_identifier: Some(device(instance)),
        ..command
    };
    vec![
        list(vec![
            write(ao(1), real(50.0), 8),
            write(av1(), real(21.5), 9),
        ]),
        list(vec![delayed, write(ao(1), real(70.0), 8)]),
        list(vec![
            quitting,
            write(ao(1), real(10.0), 8),
            write(ao(2), real(10.0), 8),
        ]),
        list(vec![missing, write(ao(1), real(60.0), 8)]),
        list(vec![
            in_device(9, write(ao(1), real(80.0), 8)),
            in_device(856, write(ao(2), real(80.0), 8)),
        ]),
        list(vec![]),
        list(vec![BACnetActionCommand {
            priority: None,
            ..write(cmd(1), PropertyValue::Unsigned(6), 16)
        }]),
    ]
}

/// Add AO-1 and AO-2.
pub(super) fn outputs(db: &mut ObjectDatabase) {
    for instance in [1, 2] {
        db.add(Box::new(
            AnalogOutputObject::new(
                instance,
                format!("AO-{instance}"),
                EngineeringUnits::DEGREES_CELSIUS,
            )
            .unwrap(),
        ))
        .unwrap();
    }
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        outputs(db);
        let mut command = CommandObject::new(1, "CMD-1").unwrap();
        command.set_action(lists()).unwrap();
        db.add(Box::new(command)).unwrap();
    })
    .await
}

fn unsigned(value: u64) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &PropertyValue::Unsigned(value)).unwrap();
    encoded.to_vec()
}

/// WriteProperty of `value` to `object`'s Present_Value.
pub(super) async fn write_property(
    h: &mut Harness,
    object: ObjectIdentifier,
    value: Vec<u8>,
    priority: Option<u8>,
) -> Result<(), ErrorPdu> {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: PV,
        property_array_index: None,
        property_value: value,
        priority,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    error_response(h).await
}

/// WriteProperty of `value` to CMD-`instance`'s Present_Value.
pub(super) async fn write_pv(h: &mut Harness, instance: u32, value: u64) -> Result<(), ErrorPdu> {
    write_property(h, cmd(instance), unsigned(value), None).await
}

fn assert_error(result: Result<(), ErrorPdu>, class: ErrorClass, code: ErrorCode) {
    let error = result.expect_err("an Error PDU");
    assert_eq!((error.error_class, error.error_code), (class, code));
}

/// ReadProperty over the wire: the value octets, or the error code.
pub(super) async fn read_wire(
    h: &mut Harness,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<Vec<u8>, ErrorCode> {
    let mut body = BytesMut::new();
    ReadPropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: index,
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::READ_PROPERTY, body).await;
    let invoke_id = h.invoke_id;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let answer = {
                let mut frames = h.frames.lock().unwrap();
                let at = frames.iter().position(|apdu| match apdu {
                    Apdu::ComplexAck(ack) => ack.invoke_id == invoke_id,
                    Apdu::Error(error) => error.invoke_id == invoke_id,
                    _ => false,
                });
                at.map(|at| frames.remove(at))
            };
            match answer {
                Some(Apdu::ComplexAck(ack)) => {
                    return Ok(ReadPropertyACK::decode(&ack.service_ack)
                        .unwrap()
                        .property_value);
                }
                Some(Apdu::Error(error)) => return Err(error.error_code),
                _ => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
    })
    .await
    .expect("a ReadProperty answer")
}

/// CMD-`instance`'s In_Process and All_Writes_Successful, read over the wire.
pub(super) async fn state(h: &mut Harness, instance: u32) -> (bool, bool) {
    (
        read_bool(h, instance, PropertyIdentifier::IN_PROCESS).await,
        read_bool(h, instance, PropertyIdentifier::ALL_WRITES_SUCCESSFUL).await,
    )
}

async fn read_bool(h: &mut Harness, instance: u32, property: PropertyIdentifier) -> bool {
    match read_wire(h, cmd(instance), property, None).await.unwrap()[..] {
        [0x10] => false,
        [0x11] => true,
        ref other => panic!("{property:?} read {other:?}"),
    }
}

/// The write-successful flags of CMD-1's Action element `index`, read over
/// the wire.
async fn flags(h: &mut Harness, index: u32) -> Vec<bool> {
    let element = read_wire(h, cmd(1), PropertyIdentifier::ACTION, Some(index))
        .await
        .unwrap();
    let (list, end) = decode_action_list(&element, 0).unwrap();
    assert_eq!(end, element.len());
    list.commands
        .iter()
        .map(|command| command.write_successful)
        .collect()
}

pub(super) async fn read_db(
    h: &Harness,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&object)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

/// Slot 8 of `target`'s Priority_Array: what a list commanded there.
pub(super) async fn slot8(h: &Harness, target: ObjectIdentifier) -> PropertyValue {
    read_db(h, target, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await
}

/// Wait, in paused time, until CMD-`instance` has finished its run.
pub(super) async fn idle(h: &Harness, instance: u32) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while read_db(h, cmd(instance), PropertyIdentifier::IN_PROCESS, None).await
            == PropertyValue::Boolean(true)
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the Command run finished");
    h.settle().await;
}

#[tokio::test(start_paused = true)]
async fn command_present_value_write_runs_the_selected_list_on_local_targets() {
    let mut h = start().await;
    assert_eq!(state(&mut h, 1).await, (false, true));
    write_pv(&mut h, 1, 1).await.unwrap();
    idle(&h, 1).await;
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(50.0));
    assert_eq!(
        read_db(&h, ao(1), PV, None).await,
        PropertyValue::Real(50.0)
    );
    assert_eq!(
        read_db(&h, av1(), PropertyIdentifier::PRIORITY_ARRAY, Some(9)).await,
        PropertyValue::Real(21.5)
    );
    assert_eq!(state(&mut h, 1).await, (false, true));
    assert_eq!(flags(&mut h, 1).await, [true, true]);
    assert_eq!(read_wire(&mut h, cmd(1), PV, None).await, Ok(unsigned(1)));
}

#[tokio::test(start_paused = true)]
async fn command_write_during_a_run_is_busy_and_the_post_delay_holds_the_next_write() {
    let mut h = start().await;
    write_pv(&mut h, 1, 2).await.unwrap();
    h.settle().await;
    // The first write is made; the second waits out the 5-second delay.
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(30.0));
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Null);
    assert_eq!(state(&mut h, 1).await, (true, false));
    // Any Present_Value write meanwhile, the running number included.
    for value in [2, 1, 0] {
        assert_error(
            write_pv(&mut h, 1, value).await,
            ErrorClass::OBJECT,
            ErrorCode::BUSY,
        );
    }
    assert_eq!(read_wire(&mut h, cmd(1), PV, None).await, Ok(unsigned(2)));

    tokio::time::sleep(Duration::from_secs(4)).await;
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Null);
    assert_eq!(state(&mut h, 1).await, (true, false));
    tokio::time::sleep(Duration::from_millis(1500)).await;
    idle(&h, 1).await;
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(70.0));
    assert_eq!(state(&mut h, 1).await, (false, true));
    // Idle again, the next write is taken.
    write_pv(&mut h, 1, 1).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn command_quit_on_failure_stops_the_list_and_action_reads_back_the_flags() {
    let mut h = start().await;
    write_pv(&mut h, 1, 3).await.unwrap();
    idle(&h, 1).await;
    // AO-9 failed and quit: neither later write is made, and both read
    // unsuccessful.
    assert_eq!(flags(&mut h, 3).await, [false, false, false]);
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Null);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
    assert_eq!(state(&mut h, 1).await, (false, false));

    // Without the quit flag the list goes on past the failure.
    write_pv(&mut h, 1, 4).await.unwrap();
    idle(&h, 1).await;
    assert_eq!(flags(&mut h, 4).await, [false, true]);
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(60.0));
    assert_eq!(state(&mut h, 1).await, (false, false));
}

#[tokio::test(start_paused = true)]
async fn command_rewriting_the_same_value_runs_the_list_again() {
    let mut h = start().await;
    write_pv(&mut h, 1, 1).await.unwrap();
    idle(&h, 1).await;
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(50.0));
    // Someone else commands AO-1 at the same priority.
    write_property(&mut h, ao(1), real(5.0), Some(8))
        .await
        .unwrap();
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(5.0));
    // Present_Value is already 1; writing 1 again makes the writes again.
    write_pv(&mut h, 1, 1).await.unwrap();
    idle(&h, 1).await;
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(50.0));
    assert_eq!(state(&mut h, 1).await, (false, true));
}

#[tokio::test(start_paused = true)]
async fn command_zero_or_an_empty_list_writes_nothing_and_succeeds_at_once() {
    let mut h = start().await;
    write_pv(&mut h, 1, 4).await.unwrap();
    idle(&h, 1).await;
    assert_eq!(state(&mut h, 1).await, (false, false));
    for value in [0, 6] {
        write_pv(&mut h, 1, value).await.unwrap();
        assert_eq!(state(&mut h, 1).await, (false, true), "{value}");
        assert_eq!(
            read_wire(&mut h, cmd(1), PV, None).await,
            Ok(unsigned(value))
        );
        assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(60.0));
        assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Null);
    }
}

#[tokio::test(start_paused = true)]
async fn command_present_value_above_the_action_size_is_value_out_of_range() {
    let mut h = start().await;
    for value in [8, u64::from(u32::MAX)] {
        assert_error(
            write_pv(&mut h, 1, value).await,
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        );
    }
    assert_eq!(read_wire(&mut h, cmd(1), PV, None).await, Ok(unsigned(0)));
    assert_eq!(state(&mut h, 1).await, (false, true));
    // Seven lists: 7 is the last one in range.
    write_pv(&mut h, 1, 7).await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn command_naming_another_device_fails_and_naming_this_device_is_local() {
    let mut h = start().await;
    write_pv(&mut h, 1, 5).await.unwrap();
    idle(&h, 1).await;
    // Device 9 has no binding and answers no Who-Is, so no request is sent
    // there.
    assert!(
        !h.frames
            .lock()
            .unwrap()
            .iter()
            .any(|apdu| matches!(apdu, Apdu::ConfirmedRequest(_)))
    );
    assert_eq!(flags(&mut h, 5).await, [false, true]);
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Null);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(80.0));
    assert_eq!(state(&mut h, 1).await, (false, false));
}

#[tokio::test(start_paused = true)]
async fn command_writing_its_own_present_value_is_refused_busy() {
    let mut h = start().await;
    write_pv(&mut h, 1, 7).await.unwrap();
    idle(&h, 1).await;
    assert_eq!(flags(&mut h, 7).await, [false]);
    assert_eq!(read_wire(&mut h, cmd(1), PV, None).await, Ok(unsigned(7)));
    assert_eq!(state(&mut h, 1).await, (false, false));
}
