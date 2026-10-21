//! A Trend Log Multiple's Log_DeviceObjectProperty resized through
//! `write_local` on a running server (#1234). Writing element 0 of an array
//! whose size writes can change sets that size (Clause 12.1.5.1); the wire
//! cases are in `handlers/tests/log_reference_writes.rs`.

use super::cov_wire_test_support::*;
use super::*;
use bacnet_objects::trend::TrendLogMultipleObject;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::ObjectType;

const LDOP: PropertyIdentifier = PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY;

/// [0] analog-value 1, [1] present-value.
const AV1_PV: [u8; 7] = [0x0C, 0x00, 0x80, 0x00, 0x01, 0x19, 0x55];
/// [0] analog-input 4194303, [1] present-value: an empty element.
const EMPTY: [u8; 7] = [0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55];

fn tlm1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG_MULTIPLE, 1).unwrap()
}

async fn write_local(h: &Harness, index: Option<u32>, value: PropertyValue) -> Result<(), Error> {
    h.server
        .write_local(
            &tlm1(),
            LDOP,
            index,
            value,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

async fn read(h: &Harness, property: PropertyIdentifier, index: Option<u32>) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&tlm1())
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

/// Total_Record_Count: one more for each purge's status record.
async fn total(h: &Harness) -> PropertyValue {
    read(h, PropertyIdentifier::TOTAL_RECORD_COUNT, None).await
}

fn assert_refused(result: Result<(), Error>, class: ErrorClass, code: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Protocol { class: c, code: e })
            if c == class && e == code),
        "expected {class:?} / {code:?}, got {result:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn write_local_of_index_0_resizes_the_reference_array() {
    let h = Harness::start_with(ServerConfig::default(), |db| {
        let mut tlm = TrendLogMultipleObject::new(1, "TLM-1", 8).unwrap();
        tlm.add_property_reference(BACnetDeviceObjectPropertyReference::new_local(
            av1(),
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ))
        .unwrap();
        db.add(Box::new(tlm)).unwrap();
    })
    .await;
    assert_eq!(total(&h).await, PropertyValue::Unsigned(0));

    // Growing appends empty elements and purges the log.
    write_local(&h, Some(0), PropertyValue::Unsigned(3))
        .await
        .unwrap();
    assert_eq!(read(&h, LDOP, Some(0)).await, PropertyValue::Unsigned(3));
    assert_eq!(
        read(&h, LDOP, None).await,
        PropertyValue::List(vec![
            PropertyValue::ApplicationData(AV1_PV.to_vec()),
            PropertyValue::ApplicationData(EMPTY.to_vec()),
            PropertyValue::ApplicationData(EMPTY.to_vec()),
        ])
    );
    assert_eq!(total(&h).await, PropertyValue::Unsigned(1));

    // The size already held is no change: no purge.
    write_local(&h, Some(0), PropertyValue::Unsigned(3))
        .await
        .unwrap();
    assert_eq!(total(&h).await, PropertyValue::Unsigned(1));

    // Shrinking drops the trailing elements and purges again.
    write_local(&h, Some(0), PropertyValue::Unsigned(1))
        .await
        .unwrap();
    assert_eq!(
        read(&h, LDOP, None).await,
        PropertyValue::List(vec![PropertyValue::ApplicationData(AV1_PV.to_vec())])
    );
    assert_eq!(total(&h).await, PropertyValue::Unsigned(2));

    // Past the cap, or not an Unsigned: refused, nothing changes.
    assert_refused(
        write_local(&h, Some(0), PropertyValue::Unsigned(65)).await,
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
    );
    assert_refused(
        write_local(&h, Some(0), PropertyValue::Real(2.0)).await,
        ErrorClass::PROPERTY,
        ErrorCode::INVALID_DATA_TYPE,
    );
    assert_eq!(read(&h, LDOP, Some(0)).await, PropertyValue::Unsigned(1));
    assert_eq!(total(&h).await, PropertyValue::Unsigned(2));
}
