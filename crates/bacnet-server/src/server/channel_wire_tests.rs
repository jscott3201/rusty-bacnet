//! A Channel object's Present_Value write reaches its members, on a running
//! server (#1151, Clause 12.53).
//!
//! CH-1 (channel 11) has four members, each with no delay: the Present_Value
//! of AO-1 (REAL), BO-1 (ENUMERATED) and MSO-1 (Unsigned, three states), and
//! CH-2's Channel_Number, an Unsigned that isn't commandable.
//!
//! CH-3 (channel 12) has three: AO-2's Present_Value after 200 ms, AV-1's at
//! once and AO-9's, which doesn't exist, after 100 ms.
//!
//! Requests and the Channel reads go over the wire; members are read from
//! the database. The clock is paused, so delays pass only when a test sleeps
//! through them.
use super::command_action_wire_tests::{ao, outputs, read_db, read_wire, slot8};
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::apdu::ErrorPdu;
use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::channel::ChannelObject;
use bacnet_objects::multistate::MultiStateOutputObject;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{ObjectType, WriteStatus};

const LIST: PropertyIdentifier = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;

pub(super) fn ch(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::CHANNEL, instance).unwrap()
}

fn bo1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap()
}

fn mso1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::MULTI_STATE_OUTPUT, 1).unwrap()
}

pub(super) fn member(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference::new_local(object, property.to_raw())
}

pub(super) fn channel(
    instance: u32,
    number: u16,
    members: Vec<(BACnetDeviceObjectPropertyReference, u32)>,
) -> ChannelObject {
    let mut channel = ChannelObject::new(instance, format!("CH-{instance}"), number).unwrap();
    let (references, delays) = members.into_iter().unzip();
    channel.set_members(references).unwrap();
    channel.set_execution_delay(delays).unwrap();
    channel
}

fn objects(db: &mut ObjectDatabase) {
    outputs(db);
    db.add(Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()))
        .unwrap();
    db.add(Box::new(
        MultiStateOutputObject::new(1, "MSO-1", 3).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(channel(2, 5, vec![]))).unwrap();
    db.add(Box::new(channel(
        1,
        11,
        vec![
            (member(ao(1), PV), 0),
            (member(bo1(), PV), 0),
            (member(mso1(), PV), 0),
            (member(ch(2), PropertyIdentifier::CHANNEL_NUMBER), 0),
        ],
    )))
    .unwrap();
    db.add(Box::new(channel(
        3,
        12,
        vec![
            (member(ao(2), PV), 200),
            (member(av1(), PV), 0),
            (member(ao(9), PV), 100),
        ],
    )))
    .unwrap();
}

pub(super) async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), objects).await
}

pub(super) fn encoded(value: &PropertyValue) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_property_value(&mut bytes, value).unwrap();
    bytes.to_vec()
}

/// WriteProperty over the wire.
pub(super) async fn write_wire(
    h: &mut Harness,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    value: Vec<u8>,
    priority: Option<u8>,
) -> Result<(), ErrorPdu> {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: object,
        property_identifier: property,
        property_array_index: index,
        property_value: value,
        priority,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    error_response(h).await
}

/// WriteProperty of `value` to CH-`instance`'s Present_Value.
pub(super) async fn write_channel(
    h: &mut Harness,
    instance: u32,
    value: &PropertyValue,
    priority: Option<u8>,
) -> Result<(), ErrorPdu> {
    write_wire(h, ch(instance), PV, None, encoded(value), priority).await
}

/// CH-`instance`'s Write_Status, read over the wire.
pub(super) async fn write_status(h: &mut Harness, instance: u32) -> WriteStatus {
    match read_wire(h, ch(instance), PropertyIdentifier::WRITE_STATUS, None)
        .await
        .unwrap()[..]
    {
        [0x91, raw] => WriteStatus::from_raw(raw.into()),
        ref other => panic!("Write_Status read {other:?}"),
    }
}

/// Wait, in paused time, until CH-`instance` has written its members, and
/// return its Write_Status.
pub(super) async fn settled(h: &mut Harness, instance: u32) -> WriteStatus {
    tokio::time::timeout(Duration::from_secs(60), async {
        while read_db(h, ch(instance), PropertyIdentifier::WRITE_STATUS, None).await
            == PropertyValue::Enumerated(WriteStatus::IN_PROGRESS.to_raw())
        {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the Channel distribution finished");
    h.settle().await;
    write_status(h, instance).await
}

/// Slot `priority` of `target`'s Priority_Array.
pub(super) async fn slot(h: &Harness, target: ObjectIdentifier, priority: u32) -> PropertyValue {
    read_db(
        h,
        target,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(priority),
    )
    .await
}

async fn channel_number(h: &Harness, instance: u32) -> PropertyValue {
    read_db(h, ch(instance), PropertyIdentifier::CHANNEL_NUMBER, None).await
}

fn assert_error(result: Result<(), ErrorPdu>, class: ErrorClass, code: ErrorCode) {
    let error = result.expect_err("an Error PDU");
    assert_eq!((error.error_class, error.error_code), (class, code));
}

#[tokio::test(start_paused = true)]
async fn channel_value_reaches_members_of_each_datatype_coerced_at_the_written_priority() {
    let mut h = start().await;
    assert_eq!(write_status(&mut h, 1).await, WriteStatus::IDLE);
    write_channel(&mut h, 1, &PropertyValue::Real(1.0), Some(8))
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 1).await, WriteStatus::SUCCESSFUL);
    // REAL 1.0 as each member's own datatype (Table 12-63, Rules 1 and 5).
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(1.0));
    assert_eq!(slot8(&h, bo1()).await, PropertyValue::Enumerated(1));
    assert_eq!(slot8(&h, mso1()).await, PropertyValue::Unsigned(1));
    assert_eq!(channel_number(&h, 2).await, PropertyValue::Unsigned(1));
    assert_eq!(
        read_wire(&mut h, ch(1), PV, None).await,
        Ok(encoded(&PropertyValue::Real(1.0)))
    );
    assert_eq!(
        read_wire(&mut h, ch(1), PropertyIdentifier::LAST_PRIORITY, None).await,
        Ok(vec![0x21, 8])
    );

    // A BOOLEAN goes out as 0 or 1 (Rule 2), here at priority 5: slot 8
    // keeps the earlier write.
    write_channel(&mut h, 1, &PropertyValue::Boolean(false), Some(5))
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 1).await, WriteStatus::FAILED);
    assert_eq!(slot(&h, ao(1), 5).await, PropertyValue::Real(0.0));
    assert_eq!(slot(&h, bo1(), 5).await, PropertyValue::Enumerated(0));
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(1.0));
    // MSO-1 has no state 0, so that member failed and the others didn't
    // stop for it.
    assert_eq!(slot(&h, mso1(), 5).await, PropertyValue::Null);
    assert_eq!(channel_number(&h, 2).await, PropertyValue::Unsigned(0));

    // With no priority the members get none: slot 16 for the commandable
    // ones, and Last_Priority reads 16.
    write_channel(&mut h, 1, &PropertyValue::Unsigned(2), None)
        .await
        .unwrap();
    // BO-1 refuses 2, as a WriteProperty of ENUMERATED 2 would be refused.
    assert_eq!(settled(&mut h, 1).await, WriteStatus::FAILED);
    assert_eq!(slot(&h, ao(1), 16).await, PropertyValue::Real(2.0));
    assert_eq!(slot(&h, mso1(), 16).await, PropertyValue::Unsigned(2));
    assert_eq!(slot(&h, bo1(), 16).await, PropertyValue::Null);
    assert_eq!(channel_number(&h, 2).await, PropertyValue::Unsigned(2));
    assert_eq!(
        read_wire(&mut h, ch(1), PropertyIdentifier::LAST_PRIORITY, None).await,
        Ok(vec![0x21, 16])
    );
}

#[tokio::test(start_paused = true)]
async fn channel_null_relinquishes_commandable_members_and_is_not_failed_by_the_rest() {
    let mut h = start().await;
    write_channel(&mut h, 1, &PropertyValue::Real(1.0), Some(8))
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 1).await, WriteStatus::SUCCESSFUL);
    write_channel(&mut h, 1, &PropertyValue::Null, Some(8))
        .await
        .unwrap();
    // CH-2's Channel_Number refuses NULL as an invalid datatype; that isn't a
    // failure (Clause 12.53.7).
    assert_eq!(settled(&mut h, 1).await, WriteStatus::SUCCESSFUL);
    for target in [ao(1), bo1(), mso1()] {
        assert_eq!(slot8(&h, target).await, PropertyValue::Null, "{target}");
    }
    assert_eq!(channel_number(&h, 2).await, PropertyValue::Unsigned(1));
    assert_eq!(read_wire(&mut h, ch(1), PV, None).await, Ok(vec![0x00]));
}

#[tokio::test(start_paused = true)]
async fn channel_value_no_member_can_take_fails_without_writing() {
    let mut h = start().await;
    write_channel(
        &mut h,
        1,
        &PropertyValue::CharacterString("scene".into()),
        Some(8),
    )
    .await
    .unwrap();
    // A CharacterString coerces to none of the four datatypes.
    assert_eq!(settled(&mut h, 1).await, WriteStatus::FAILED);
    for target in [ao(1), bo1(), mso1()] {
        assert_eq!(slot8(&h, target).await, PropertyValue::Null, "{target}");
    }
    assert_eq!(channel_number(&h, 2).await, PropertyValue::Unsigned(5));
}

#[tokio::test(start_paused = true)]
async fn channel_delays_run_side_by_side_and_writes_meanwhile_are_busy() {
    let mut h = start().await;
    write_channel(&mut h, 3, &PropertyValue::Real(30.0), Some(10))
        .await
        .unwrap();
    h.settle().await;
    // AV-1 has no delay; AO-2 waits 200 ms.
    assert_eq!(slot(&h, av1(), 10).await, PropertyValue::Real(30.0));
    assert_eq!(slot(&h, ao(2), 10).await, PropertyValue::Null);
    assert_eq!(write_status(&mut h, 3).await, WriteStatus::IN_PROGRESS);
    assert_error(
        write_channel(&mut h, 3, &PropertyValue::Real(40.0), Some(10)).await,
        ErrorClass::OBJECT,
        ErrorCode::BUSY,
    );
    assert_eq!(
        read_wire(&mut h, ch(3), PV, None).await,
        Ok(encoded(&PropertyValue::Real(30.0)))
    );

    // AO-9's 100 ms pass; AO-2's 200 ms count from the same start, not from
    // AO-9's write.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(write_status(&mut h, 3).await, WriteStatus::IN_PROGRESS);
    assert_eq!(slot(&h, ao(2), 10).await, PropertyValue::Null);
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(slot(&h, ao(2), 10).await, PropertyValue::Real(30.0));
    // AO-9 doesn't exist: one failed member makes the whole write FAILED.
    assert_eq!(settled(&mut h, 3).await, WriteStatus::FAILED);
    // Done, the Channel takes the next write.
    write_channel(&mut h, 3, &PropertyValue::Real(40.0), Some(10))
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn channel_distribution_starts_from_write_local_and_write_property_multiple() {
    let mut h = start().await;
    h.server
        .write_local(
            &ch(1),
            PV,
            None,
            PropertyValue::Real(1.0),
            Some(9),
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 1).await, WriteStatus::SUCCESSFUL);
    assert_eq!(slot(&h, ao(1), 9).await, PropertyValue::Real(1.0));

    // Twice in one request: the first starts a distribution, so the second
    // finds the Channel busy, and the first still finishes.
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: ch(1),
            list_of_properties: [3.0, 1.0]
                .into_iter()
                .map(|value| BACnetPropertyValue {
                    property_identifier: PV,
                    property_array_index: None,
                    value: encoded(&PropertyValue::Real(value)),
                    priority: Some(9),
                })
                .collect(),
        }],
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
        .await;
    assert_eq!(response(&h).await, Err(ErrorCode::BUSY));
    // BO-1 refuses 3, so the first write ends FAILED.
    assert_eq!(settled(&mut h, 1).await, WriteStatus::FAILED);
    assert_eq!(slot(&h, ao(1), 9).await, PropertyValue::Real(3.0));
    assert_eq!(slot(&h, mso1(), 9).await, PropertyValue::Unsigned(3));
}

#[tokio::test(start_paused = true)]
async fn channel_member_list_takes_members_here_and_in_other_devices_over_the_wire() {
    let mut h = start().await;
    let reference = |device: Option<u32>| {
        let mut bytes = BytesMut::new();
        encode_device_object_property_reference(
            &mut bytes,
            &BACnetDeviceObjectPropertyReference {
                device_identifier: device
                    .map(|instance| ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()),
                ..member(ao(2), PV)
            },
        );
        bytes.to_vec()
    };
    // Another device keeps its identifier: the Channel writes that member
    // there (Clause 12.53.11, #1264).
    write_wire(&mut h, ch(2), LIST, None, reference(Some(9)), None)
        .await
        .unwrap();
    assert_eq!(
        read_wire(&mut h, ch(2), LIST, Some(1)).await,
        Ok(reference(Some(9)))
    );
    // This device's own identifier is the local member it names.
    write_wire(&mut h, ch(2), LIST, None, reference(Some(856)), None)
        .await
        .unwrap();
    assert_eq!(
        read_wire(&mut h, ch(2), LIST, Some(1)).await,
        Ok(reference(None))
    );
    // Index 0 resizes the list, and Execution_Delay with it.
    write_wire(&mut h, ch(2), LIST, Some(0), vec![0x21, 0x03], None)
        .await
        .unwrap();
    assert_eq!(
        read_wire(&mut h, ch(2), LIST, Some(0)).await,
        Ok(vec![0x21, 3])
    );
    assert_eq!(
        read_wire(&mut h, ch(2), PropertyIdentifier::EXECUTION_DELAY, None).await,
        Ok(vec![0x21, 0, 0x21, 0, 0x21, 0])
    );
    write_wire(
        &mut h,
        ch(2),
        PropertyIdentifier::EXECUTION_DELAY,
        Some(1),
        vec![0x21, 0x32],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        read_wire(&mut h, ch(2), PropertyIdentifier::EXECUTION_DELAY, Some(1)).await,
        Ok(vec![0x21, 0x32])
    );
    // The two new members are empty references, which the Channel skips.
    write_channel(&mut h, 2, &PropertyValue::Real(12.0), Some(8))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(settled(&mut h, 2).await, WriteStatus::SUCCESSFUL);
    assert_eq!(slot8(&h, ao(2)).await, PropertyValue::Real(12.0));
}

#[tokio::test(start_paused = true)]
async fn channel_member_list_refuses_a_non_device_device_identifier_over_the_wire() {
    let mut h = start().await;
    let before = read_wire(&mut h, ch(3), LIST, None).await;
    // Analog Value 856 shares the local Device's instance but is no Device,
    // so the server leaves it for the Channel to refuse as out of range
    // (#1285), whole and by index.
    for other in [ObjectType::ANALOG_VALUE, ObjectType::CHANNEL] {
        let mut bytes = BytesMut::new();
        encode_device_object_property_reference(
            &mut bytes,
            &BACnetDeviceObjectPropertyReference {
                device_identifier: Some(ObjectIdentifier::new(other, 856).unwrap()),
                ..member(ao(2), PV)
            },
        );
        for index in [None, Some(1)] {
            assert_error(
                write_wire(&mut h, ch(3), LIST, index, bytes.to_vec(), None).await,
                ErrorClass::PROPERTY,
                ErrorCode::VALUE_OUT_OF_RANGE,
            );
        }
    }
    assert_eq!(read_wire(&mut h, ch(3), LIST, None).await, before);
}

#[tokio::test(start_paused = true)]
async fn channel_writing_its_own_present_value_is_refused_busy() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(channel(4, 1, vec![(member(ch(4), PV), 0)])))
            .unwrap();
    })
    .await;
    write_channel(&mut h, 4, &PropertyValue::Unsigned(1), None)
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 4).await, WriteStatus::FAILED);
    assert_eq!(
        read_wire(&mut h, ch(4), PV, None).await,
        Ok(encoded(&PropertyValue::Unsigned(1)))
    );
}

/// A member naming the identity's Device 856 is written here once the
/// application adds a lower Device 5 (#2227).
#[tokio::test(start_paused = true)]
async fn channel_member_naming_the_identity_device_stays_local_beside_a_lower_one() {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        objects(db);
        let qualified = BACnetDeviceObjectPropertyReference::new_remote(
            ao(1),
            PV.to_raw(),
            ObjectIdentifier::new(ObjectType::DEVICE, 856).unwrap(),
        );
        db.add(Box::new(channel(4, 13, vec![(qualified, 0)])))
            .unwrap();
    })
    .await;
    let lower = crate::local_device::lower_device(5);
    h.server.database().write().await.add(lower).unwrap();
    write_channel(&mut h, 4, &PropertyValue::Real(3.0), Some(8))
        .await
        .unwrap();
    assert_eq!(settled(&mut h, 4).await, WriteStatus::SUCCESSFUL);
    assert_eq!(slot8(&h, ao(1)).await, PropertyValue::Real(3.0));
}
