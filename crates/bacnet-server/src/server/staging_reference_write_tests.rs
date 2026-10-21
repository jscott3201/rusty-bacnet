//! Staging Target_References members whose Device names this device, on a
//! running server (#1136).
//!
//! Clause 12.62.14 lets a Staging object limited to its own device refuse
//! only references outside it. A target naming Device 856, the harness's own
//! Device, is stored as the local reference it stands for, through
//! WriteProperty (whole or by index), WritePropertyMultiple and
//! `write_local`; one naming any other device is still refused.
//!
//! STG-1 starts at 15.0, inside its second stage, which drives both targets
//! ACTIVE at priority 8: BO-1 and BV-1. BO-2 and BV-2 are spare targets with
//! nothing commanded. Every accepted change to the array re-applies the
//! current stage, so a new target is commanded at once.
use super::cov_wire_test_support::*;
use super::*;
use bacnet_encoding::constructed::encode_device_object_reference;
use bacnet_objects::binary::{BinaryOutputObject, BinaryValueObject};
use bacnet_objects::staging::{StagingConfig, StagingObject};
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::{BACnetDeviceObjectReference, BACnetStageLimitValue};
use bacnet_types::enums::{EngineeringUnits, ObjectType};

const TARGETS: PropertyIdentifier = PropertyIdentifier::TARGET_REFERENCES;

fn stg1() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::STAGING, 1).unwrap()
}

fn bo(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, instance).unwrap()
}

fn bv(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_VALUE, instance).unwrap()
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

fn reference(object: ObjectIdentifier, device: Option<ObjectIdentifier>) -> Vec<u8> {
    let mut bytes = BytesMut::new();
    encode_device_object_reference(
        &mut bytes,
        &BACnetDeviceObjectReference {
            device_identifier: device,
            object_identifier: object,
        },
    );
    bytes.to_vec()
}

/// The encoded array: each object with the Device identifier given.
fn targets(targets: &[(ObjectIdentifier, Option<ObjectIdentifier>)]) -> Vec<u8> {
    targets
        .iter()
        .flat_map(|(object, device)| reference(*object, *device))
        .collect()
}

fn stage(limit: f32, active: bool) -> BACnetStageLimitValue {
    BACnetStageLimitValue {
        limit,
        values: vec![active; 2],
        deadband: 1.0,
    }
}

async fn start() -> Harness {
    Harness::start_with(ServerConfig::default(), |db| {
        for instance in [1, 2] {
            db.add(Box::new(
                BinaryOutputObject::new(instance, format!("BO-{instance}")).unwrap(),
            ))
            .unwrap();
            db.add(Box::new(
                BinaryValueObject::new(instance, format!("BV-{instance}")).unwrap(),
            ))
            .unwrap();
        }
        let config = StagingConfig {
            present_value: 15.0,
            min_present_value: 0.0,
            units: EngineeringUnits::DEGREES_CELSIUS,
            priority_for_writing: 8,
            stages: vec![stage(10.0, false), stage(20.0, true)],
            target_references: [bo(1), bv(1)]
                .map(|object| BACnetDeviceObjectReference {
                    device_identifier: None,
                    object_identifier: object,
                })
                .to_vec(),
            stage_names: None,
        };
        db.add(Box::new(StagingObject::new(1, "STG-1", config).unwrap()))
            .unwrap();
    })
    .await
}

/// WriteProperty of `value` to STG-1's Target_References.
async fn write_property(
    h: &mut Harness,
    index: Option<u32>,
    value: Vec<u8>,
) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: stg1(),
        property_identifier: TARGETS,
        property_array_index: index,
        property_value: value,
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::WRITE_PROPERTY, body)
        .await;
    response(h).await
}

/// WritePropertyMultiple of `value` to STG-1's Target_References.
async fn write_property_multiple(
    h: &mut Harness,
    index: Option<u32>,
    value: Vec<u8>,
) -> Result<(), ErrorCode> {
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: stg1(),
            list_of_properties: vec![BACnetPropertyValue {
                property_identifier: TARGETS,
                property_array_index: index,
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

async fn read(
    h: &Harness,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    h.server
        .database()
        .read()
        .await
        .get(&oid)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

/// STG-1's Target_References as a read returns it: local references to
/// these objects, one element each.
async fn assert_targets(h: &Harness, objects: &[ObjectIdentifier], what: &str) {
    let expected = objects
        .iter()
        .map(|object| PropertyValue::ApplicationData(reference(*object, None)))
        .collect();
    assert_eq!(
        read(h, stg1(), TARGETS, None).await,
        PropertyValue::List(expected),
        "{what}"
    );
}

/// Slot 8 of `target`'s Priority_Array: ACTIVE once STG-1 commands it.
async fn assert_commanded(h: &Harness, target: ObjectIdentifier, commanded: bool) {
    let expected = if commanded {
        PropertyValue::Enumerated(1)
    } else {
        PropertyValue::Null
    };
    assert_eq!(
        read(h, target, PropertyIdentifier::PRIORITY_ARRAY, Some(8)).await,
        expected,
        "slot 8 of {target:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn write_property_takes_a_target_naming_this_device_as_a_local_reference() {
    let mut h = start().await;
    assert_targets(&h, &[bo(1), bv(1)], "at start-up").await;
    assert_commanded(&h, bo(1), true).await;
    assert_commanded(&h, bo(2), false).await;

    // The whole array, BO-2 named with this device.
    write_property(&mut h, None, targets(&[(bo(2), local()), (bv(1), None)]))
        .await
        .unwrap();
    assert_targets(&h, &[bo(2), bv(1)], "whole array").await;
    assert_commanded(&h, bo(2), true).await;

    // One element by index, named the same way.
    write_property(&mut h, Some(2), reference(bv(2), local()))
        .await
        .unwrap();
    assert_targets(&h, &[bo(2), bv(2)], "element 2").await;
    assert_eq!(
        read(&h, stg1(), TARGETS, Some(2)).await,
        PropertyValue::ApplicationData(reference(bv(2), None))
    );
    assert_commanded(&h, bv(2), true).await;
}

#[tokio::test(start_paused = true)]
async fn write_property_multiple_and_write_local_take_a_target_naming_this_device() {
    let mut h = start().await;
    write_property_multiple(&mut h, None, targets(&[(bo(1), local()), (bo(2), local())]))
        .await
        .unwrap();
    assert_targets(&h, &[bo(1), bo(2)], "WritePropertyMultiple, whole").await;
    assert_commanded(&h, bo(2), true).await;

    write_property_multiple(&mut h, Some(1), reference(bv(2), local()))
        .await
        .unwrap();
    assert_targets(&h, &[bv(2), bo(2)], "WritePropertyMultiple, element 1").await;
    assert_commanded(&h, bv(2), true).await;

    // write_local, in the shape a read returns.
    h.server
        .write_local(
            &stg1(),
            TARGETS,
            None,
            PropertyValue::List(vec![
                PropertyValue::ApplicationData(reference(bv(1), local())),
                PropertyValue::ApplicationData(reference(bo(2), None)),
            ]),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    assert_targets(&h, &[bv(1), bo(2)], "write_local").await;
}

#[tokio::test(start_paused = true)]
async fn a_target_in_another_device_is_refused_alone_or_in_a_mixed_array() {
    let mut h = start().await;
    let code = Err(ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED);
    // A local target, then BO-2 in Device 9.
    let mixed = targets(&[(bv(1), local()), (bo(2), remote())]);
    assert_eq!(write_property(&mut h, None, mixed.clone()).await, code);
    assert_eq!(write_property_multiple(&mut h, None, mixed).await, code);
    let alone = reference(bo(2), remote());
    assert_eq!(write_property(&mut h, Some(1), alone.clone()).await, code);
    assert_eq!(write_property_multiple(&mut h, Some(1), alone).await, code);
    let refused = h
        .server
        .write_local(
            &stg1(),
            TARGETS,
            Some(1),
            PropertyValue::ApplicationData(reference(bo(2), remote())),
            None,
            crate::LocalCommandSource::ServerDevice,
        )
        .await;
    assert!(refused.is_err(), "write_local of a remote target");
    assert_targets(&h, &[bo(1), bv(1)], "after the refusals").await;
    assert_commanded(&h, bo(2), false).await;

    // Without the remote member the array is accepted.
    write_property(&mut h, None, targets(&[(bv(1), local()), (bo(2), None)]))
        .await
        .unwrap();
    assert_targets(&h, &[bv(1), bo(2)], "the local targets").await;
    assert_commanded(&h, bo(2), true).await;
}

#[tokio::test(start_paused = true)]
async fn a_target_whose_device_member_is_not_a_device_is_out_of_range() {
    let mut h = start().await;
    let code = Err(ErrorCode::VALUE_OUT_OF_RANGE);
    // Analog Value 856 shares the local Device's instance but is no Device,
    // so it is neither localized nor taken as a remote device (#1285).
    for other in [ObjectType::ANALOG_VALUE, ObjectType::BINARY_OUTPUT] {
        let not_a_device = Some(ObjectIdentifier::new(other, 856).unwrap());
        let mixed = targets(&[(bv(1), local()), (bo(2), not_a_device)]);
        assert_eq!(write_property(&mut h, None, mixed.clone()).await, code);
        assert_eq!(write_property_multiple(&mut h, None, mixed).await, code);
        let alone = reference(bo(2), not_a_device);
        assert_eq!(write_property(&mut h, Some(1), alone.clone()).await, code);
        assert_eq!(write_property_multiple(&mut h, Some(1), alone).await, code);
    }
    assert_targets(&h, &[bo(1), bv(1)], "after the refusals").await;
    assert_commanded(&h, bo(2), false).await;
}
