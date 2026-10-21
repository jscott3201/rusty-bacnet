//! Schedule reference and priority writes on a running server (#1088):
//! WriteProperty, WritePropertyMultiple and `write_local` of
//! List_Of_Object_Property_References and Priority_For_Writing, the targets
//! commanded and relinquished at once, and the refusals. Also the reference
//! half of Reliability (#1086): a target that refuses the schedule's datatype
//! faults the Schedule, which still writes its other targets, until the
//! target takes a value or leaves the list. A target created after it was
//! refused takes the unchanged value as it is created (#1440), without
//! waiting for the server's next pass (#1436).
//!
//! The harness is the one in `schedule_write_tests`: Tuesday 29 September
//! 2026, 15:00; SCH-5 commands AV-1's Present_Value at priority 16 and
//! defaults to 10.0, written at start-up; the COV subscription watches AV-1.
//! AV-2 (an Analog Value) and BV-2 (a Binary Value) are spare targets, both
//! commandable with nothing commanded.
use super::cov_wire_test_support::*;
use super::schedule_write_tests::{
    assert_commanded, from_three, read, sch5, schedule, write_property, write_property_multiple,
};
use super::*;
use bacnet_encoding::constructed::encode_object_property_reference;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_types::constructed::BACnetObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, ObjectType, Reliability};

pub(super) const LIST: PropertyIdentifier = PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES;
const PRIORITY: PropertyIdentifier = PropertyIdentifier::PRIORITY_FOR_WRITING;
const WEEKLY: PropertyIdentifier = PropertyIdentifier::WEEKLY_SCHEDULE;

pub(super) fn av2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap()
}

pub(super) fn bv2() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, 2).unwrap()
}

/// The encoded list of references to each object's Present_Value.
pub(super) fn present_values(objects: &[ObjectIdentifier]) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    for object in objects {
        encode_object_property_reference(
            &mut bytes,
            &BACnetObjectPropertyReference::new(*object, PV.to_raw()),
        );
    }
    bytes.to_vec()
}

async fn start() -> Harness {
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        schedule(db);
        db.add(Box::new(
            AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
        db.add(Box::new(BinaryValueObject::new(2, "BV-2").unwrap()))
            .unwrap();
    })
    .await;
    h.subscribe_cov().await;
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(10.0)
    );
    h
}

/// One slot of a target's Priority_Array.
pub(super) async fn slot(h: &Harness, oid: ObjectIdentifier, priority: u32) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(priority))
        .unwrap()
}

/// SCH-5's Reliability, and the FAULT flag in its Status_Flags.
async fn assert_reliability(h: &Harness, expected: Reliability) {
    assert_eq!(
        read(h, sch5(), PropertyIdentifier::RELIABILITY).await,
        PropertyValue::Enumerated(expected.to_raw())
    );
    let PropertyValue::BitString { data, .. } =
        read(h, sch5(), PropertyIdentifier::STATUS_FLAGS).await
    else {
        panic!("Status_Flags reads as a bit string");
    };
    let fault = data.first().is_some_and(|octet| octet & 0x40 != 0);
    assert_eq!(fault, expected != Reliability::NO_FAULT_DETECTED);
}

#[tokio::test(start_paused = true)]
async fn write_property_of_priority_for_writing_moves_the_command_at_once() {
    let mut h = start().await;
    for (value, code) in [
        (vec![0x21, 0], ErrorCode::VALUE_OUT_OF_RANGE),
        (vec![0x21, 17], ErrorCode::VALUE_OUT_OF_RANGE),
        (real(9.0), ErrorCode::INVALID_DATA_TYPE),
    ] {
        assert_eq!(
            write_property(&mut h, PRIORITY, None, value).await,
            Err(code)
        );
    }
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));

    // Slot 16 is relinquished and slot 9 commanded, in the same pass, so
    // Present_Value never leaves 10.0 and no notification goes out.
    write_property(&mut h, PRIORITY, None, vec![0x21, 9])
        .await
        .unwrap();
    assert_eq!(read(&h, sch5(), PRIORITY).await, PropertyValue::Unsigned(9));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(slot(&h, av1(), 9).await, PropertyValue::Real(10.0));
    h.no_notification().await;

    // The next change lands in the new slot.
    write_property(&mut h, WEEKLY, Some(2), from_three(21.5))
        .await
        .unwrap();
    assert_commanded(&h, 21.5).await;
    assert_eq!(slot(&h, av1(), 9).await, PropertyValue::Real(21.5));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
}

#[tokio::test(start_paused = true)]
async fn write_property_of_the_reference_list_commands_new_targets_and_relinquishes_dropped_ones() {
    let mut h = start().await;
    let both = present_values(&[av1(), av2()]);
    write_property(&mut h, LIST, None, both.clone())
        .await
        .unwrap();
    assert_eq!(
        read(&h, sch5(), LIST).await,
        PropertyValue::ApplicationData(both)
    );
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(10.0));
    h.no_notification().await;

    // AV-1 leaves the list: its slot 16 is relinquished and it falls to
    // Relinquish_Default.
    write_property(&mut h, LIST, None, present_values(&[av2()]))
        .await
        .unwrap();
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(10.0));

    // AV-1 no longer follows the Schedule.
    write_property(&mut h, WEEKLY, Some(2), from_three(21.5))
        .await
        .unwrap();
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(21.5));
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(0.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn a_refused_reference_list_changes_neither_the_list_nor_the_targets() {
    let mut h = start().await;
    let before = read(&h, sch5(), LIST).await;
    // AV-2 in Device 9.
    let mut remote = present_values(&[av2()]);
    remote.extend([0x3C, 0x02, 0x00, 0x00, 0x09]);
    for (value, code) in [
        (remote, ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
        (real(1.0), ErrorCode::INVALID_DATA_TYPE),
        // An object identifier without its property.
        (
            vec![0x0C, 0x00, 0x80, 0x00, 0x02],
            ErrorCode::INVALID_DATA_ENCODING,
        ),
    ] {
        assert_eq!(
            write_property(&mut h, LIST, None, value.clone()).await,
            Err(code)
        );
        assert_eq!(
            write_property_multiple(&mut h, vec![(LIST, value)]).await,
            Err(code)
        );
    }
    assert_eq!(
        write_property(&mut h, LIST, Some(1), present_values(&[av2()])).await,
        Err(ErrorCode::PROPERTY_IS_NOT_AN_ARRAY)
    );
    assert_eq!(read(&h, sch5(), LIST).await, before);
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(0.0));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn write_property_multiple_of_both_properties_relinquishes_each_old_slot() {
    let mut h = start().await;
    write_property_multiple(
        &mut h,
        vec![(LIST, present_values(&[av2()])), (PRIORITY, vec![0x21, 9])],
    )
    .await
    .unwrap();
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Null);
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );
    assert_eq!(slot(&h, av2(), 9).await, PropertyValue::Real(10.0));
    assert_eq!(slot(&h, av2(), 16).await, PropertyValue::Null);
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn write_local_of_the_reference_list_commands_a_new_target() {
    let h = start().await;
    h.server
        .write_local(
            &sch5(),
            LIST,
            None,
            PropertyValue::ApplicationData(present_values(&[av1(), av2()])),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_eq!(read(&h, av2(), PV).await, PropertyValue::Real(10.0));
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
}

// --- The reference half of Reliability (#1086) --------------------------------

#[tokio::test(start_paused = true)]
async fn a_target_refusing_the_datatype_faults_the_schedule_which_still_writes_the_others() {
    let mut h = start().await;
    assert_reliability(&h, Reliability::NO_FAULT_DETECTED).await;
    write_property(&mut h, LIST, None, present_values(&[av1(), bv2()]))
        .await
        .unwrap();
    // BV-2 refuses the Real 10.0.
    assert_reliability(&h, Reliability::CONFIGURATION_ERROR).await;
    assert_eq!(read(&h, bv2(), PV).await, PropertyValue::Enumerated(0));

    // The misconfigured Schedule still commands AV-1.
    write_property(&mut h, WEEKLY, Some(2), from_three(21.5))
        .await
        .unwrap();
    assert_commanded(&h, 21.5).await;
    assert_reliability(&h, Reliability::CONFIGURATION_ERROR).await;

    // BV-2 leaves the list, and with it the fault.
    write_property(&mut h, LIST, None, present_values(&[av1()]))
        .await
        .unwrap();
    assert_reliability(&h, Reliability::NO_FAULT_DETECTED).await;
    assert_eq!(read(&h, av1(), PV).await, PropertyValue::Real(21.5));
    h.no_notification().await;
}

#[tokio::test(start_paused = true)]
async fn a_schedule_recovers_once_its_target_takes_the_datatype() {
    let mut h = start().await;
    write_property(&mut h, LIST, None, present_values(&[bv2()]))
        .await
        .unwrap();
    // AV-1, dropped, is relinquished; BV-2 refuses the Real.
    assert_eq!(
        h.cov_notification().await.list_of_values[0].value,
        real(0.0)
    );
    assert_reliability(&h, Reliability::CONFIGURATION_ERROR).await;

    // An Enumerated default: BV-2 takes it at once, and the fault clears.
    write_property(
        &mut h,
        PropertyIdentifier::SCHEDULE_DEFAULT,
        None,
        vec![0x91, 0x01],
    )
    .await
    .unwrap();
    assert_eq!(read(&h, bv2(), PV).await, PropertyValue::Enumerated(1));
    assert_reliability(&h, Reliability::NO_FAULT_DETECTED).await;
}

#[tokio::test(start_paused = true)]
async fn a_target_created_later_takes_the_value_as_it_is_created() {
    use bacnet_services::object_mgmt::{CreateObjectRequest, ObjectSpecifier};

    let mut h = start().await;
    let ao9 = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 9).unwrap();
    write_property(&mut h, LIST, None, present_values(&[av1(), ao9]))
        .await
        .unwrap();
    // AO-9 doesn't exist.
    assert_reliability(&h, Reliability::CONFIGURATION_ERROR).await;

    // A client creates it. The request offers the unchanged value to AO-9
    // under its own guard (#1440), with no pass run: AO-9 takes it and the
    // fault clears. AV-1 keeps its value and its subscriber hears nothing,
    // though a rewrite of the same 10.0 would send nothing either;
    // `schedule::reference_retry_tests` shows that a retry goes to the
    // refused references alone.
    let mut body = BytesMut::new();
    CreateObjectRequest {
        object_specifier: ObjectSpecifier::Identifier(ao9),
        list_of_initial_values: Vec::new(),
    }
    .encode(&mut body);
    h.request(ConfirmedServiceChoice::CREATE_OBJECT, body).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while h.server.database().read().await.get(&ao9).is_none() {
            h.settle().await;
        }
    })
    .await
    .expect("AO-9 was created");
    h.settle().await;
    assert_eq!(slot(&h, ao9, 16).await, PropertyValue::Real(10.0));
    assert_reliability(&h, Reliability::NO_FAULT_DETECTED).await;
    assert_eq!(slot(&h, av1(), 16).await, PropertyValue::Real(10.0));
    h.no_notification().await;
}
