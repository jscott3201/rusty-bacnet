//! Averaging Object_Property_Reference values whose Device names this device,
//! on a running server (#1153).
//!
//! Clause 12.5.13 lets an Averaging object sample only properties in its own
//! device. A reference naming Device 856, the harness's own Device, is stored
//! as the local reference it stands for, through WriteProperty,
//! WritePropertyMultiple and `write_local`; one naming any other device is
//! refused with OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. The property holds a
//! single reference, so neither an array index nor the list services apply.
//!
//! An accepted write empties the sample window (Table 12-5, footnote 1) and a
//! refused one leaves it alone, so Attempted_Samples shows which happened. The
//! server then samples the stored local reference (#1144) one spacing later:
//! 900 s / 15 samples, so 60 s of paused time.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::constructed::encode_device_object_property_reference;
use bacnet_objects::averaging::AveragingObject;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::RemoteError;

const REFERENCE: PropertyIdentifier = PropertyIdentifier::OBJECT_PROPERTY_REFERENCE;
const ATTEMPTED: PropertyIdentifier = PropertyIdentifier::ATTEMPTED_SAMPLES;

fn avg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AVERAGING, 1).unwrap()
}

fn device(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap()
}

/// The harness's own Device.
fn local() -> Option<ObjectIdentifier> {
    Some(device(856))
}

/// A Device this server is not.
fn remote() -> Option<ObjectIdentifier> {
    Some(device(9))
}

/// AV-1's `property`, element `index` when given, in the Device given.
fn reference(
    property: PropertyIdentifier,
    index: Option<u32>,
    device: Option<ObjectIdentifier>,
) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_device_object_property_reference(
        &mut bytes,
        &BACnetDeviceObjectPropertyReference {
            object_identifier: av1(),
            property_identifier: property.to_raw(),
            property_array_index: index,
            device_identifier: device,
        },
    );
    bytes.to_vec()
}

/// AV-1's Present_Value in the Device given.
fn present_value(device: Option<ObjectIdentifier>) -> Vec<u8> {
    reference(PropertyIdentifier::PRESENT_VALUE, None, device)
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(AveragingObject::new(1, "AVG-1").unwrap()))
            .unwrap();
    })
    .await
}

/// WriteProperty of `value` to AVG-1's Object_Property_Reference.
async fn write_property(h: &mut Harness, value: Vec<u8>) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: avg1(),
        property_identifier: REFERENCE,
        property_array_index: None,
        property_value: value,
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    response(h).await
}

/// WritePropertyMultiple of `value` to AVG-1's Object_Property_Reference.
async fn write_property_multiple(h: &mut Harness, value: Vec<u8>) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: avg1(),
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: REFERENCE,
                property_array_index: None,
                value,
                priority: None,
            }],
        }],
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body)
        .await;
    response(h).await
}

/// `write_local` of `value` to AVG-1's Object_Property_Reference.
async fn write_local(h: &Harness, value: PropertyValue) -> Result<(), Error> {
    h.server
        .write_local(
            &avg1(),
            REFERENCE,
            None,
            value,
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
}

async fn read(h: &Harness, property: PropertyIdentifier) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&avg1())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

/// AVG-1's Object_Property_Reference as a read returns it: AV-1's
/// `property`, element `index` when given, with no Device member.
async fn assert_reference(
    h: &Harness,
    property: PropertyIdentifier,
    index: Option<u32>,
    what: &str,
) {
    // The Clause 21 encoding, with no Device member (#1182).
    assert_eq!(
        read(h, REFERENCE).await,
        PropertyValue::ApplicationData(reference(property, index, None)),
        "{what}"
    );
}

/// Record `count` application samples, then check the window holds them.
async fn fill_window(h: &Harness, count: u64) {
    for _ in 0..count {
        h.server
            .add_averaging_sample_local(&avg1(), Some(PropertyValue::Real(1.0)))
            .await
            .unwrap();
    }
    assert_eq!(read(h, ATTEMPTED).await, PropertyValue::Unsigned(count));
}

/// Move the paused clock on by `seconds` and let the server run, without
/// moving the clock further.
async fn advance(seconds: u64) {
    tokio::time::advance(Duration::from_secs(seconds)).await;
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn write_property_takes_a_reference_naming_this_device_as_a_local_reference() {
    let mut h = start().await;
    // Unset: analog-input 4194303's present-value (#1417).
    assert_eq!(
        read(&h, REFERENCE).await,
        PropertyValue::ApplicationData(vec![0x0C, 0x00, 0x3F, 0xFF, 0xFF, 0x19, 0x55])
    );
    fill_window(&h, 2).await;

    write_property(&mut h, present_value(local()))
        .await
        .unwrap();
    assert_reference(&h, PropertyIdentifier::PRESENT_VALUE, None, "Present_Value").await;
    // Accepted, so the window was emptied.
    assert_eq!(read(&h, ATTEMPTED).await, PropertyValue::Unsigned(0));
    // The server samples AV-1 through the stored local reference: a valid
    // sample of its Present_Value, 0.0, one spacing later.
    advance(60).await;
    assert_eq!(
        read(&h, PropertyIdentifier::VALID_SAMPLES).await,
        PropertyValue::Unsigned(1)
    );
    assert_eq!(
        read(&h, PropertyIdentifier::AVERAGE_VALUE).await,
        PropertyValue::Real(0.0)
    );

    // An element of an array property keeps its index.
    let slot = reference(PropertyIdentifier::PRIORITY_ARRAY, Some(8), local());
    write_property(&mut h, slot).await.unwrap();
    assert_reference(&h, PropertyIdentifier::PRIORITY_ARRAY, Some(8), "slot 8").await;
}

#[tokio::test(start_paused = true)]
async fn write_property_multiple_and_write_local_take_a_reference_naming_this_device() {
    let mut h = start().await;
    fill_window(&h, 2).await;
    write_property_multiple(&mut h, present_value(local()))
        .await
        .unwrap();
    assert_reference(
        &h,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        "WritePropertyMultiple",
    )
    .await;
    assert_eq!(read(&h, ATTEMPTED).await, PropertyValue::Unsigned(0));

    // write_local, with the reference as one chunk of bytes.
    fill_window(&h, 2).await;
    let slot = reference(PropertyIdentifier::PRIORITY_ARRAY, Some(8), local());
    write_local(&h, PropertyValue::ApplicationData(slot))
        .await
        .unwrap();
    assert_reference(
        &h,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
        "write_local",
    )
    .await;
    assert_eq!(read(&h, ATTEMPTED).await, PropertyValue::Unsigned(0));
}

#[tokio::test(start_paused = true)]
async fn a_reference_in_another_device_is_refused_and_changes_nothing() {
    let mut h = start().await;
    write_property(&mut h, present_value(None)).await.unwrap();
    fill_window(&h, 2).await;

    let code = Err(ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED);
    assert_eq!(write_property(&mut h, present_value(remote())).await, code);
    assert_eq!(
        write_property_multiple(&mut h, present_value(remote())).await,
        code
    );
    let refused = write_local(&h, PropertyValue::ApplicationData(present_value(remote()))).await;
    assert!(
        matches!(refused, Err(Error::Remote(RemoteError::Error { code, .. }))
            if code == ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED),
        "write_local of a remote reference: {refused:?}"
    );

    // Two references where the property holds one, whichever Devices they
    // name, are malformed: none is taken.
    let code = Err(ErrorCode::INVALID_DATA_ENCODING);
    let slot = reference(PropertyIdentifier::PRIORITY_ARRAY, Some(8), local());
    for (second, what) in [(remote(), "this device, then another"), (local(), "twice")] {
        let mixed = [slot.clone(), present_value(second)].concat();
        assert_eq!(write_property(&mut h, mixed.clone()).await, code, "{what}");
        assert_eq!(write_property_multiple(&mut h, mixed).await, code, "{what}");
    }

    assert_reference(
        &h,
        PropertyIdentifier::PRESENT_VALUE,
        None,
        "after the refusals",
    )
    .await;
    assert_eq!(
        read(&h, ATTEMPTED).await,
        PropertyValue::Unsigned(2),
        "the window keeps its samples"
    );
}
