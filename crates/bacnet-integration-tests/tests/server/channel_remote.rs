use super::*;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::binary::BinaryOutputObject;
use bacnet_objects::channel::ChannelObject;
use bacnet_server::server::DeviceBinding;
use bacnet_types::constructed::BACnetDeviceObjectPropertyReference;
use bacnet_types::enums::{EngineeringUnits, Reliability, WriteStatus};
use bacnet_types::primitives::PropertyValue;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Channel members in another device (#1264, #1323, Clause 12.53.11)
//
// Two servers on loopback B/IP. Device 10 holds the Channels and reaches
// Device 20 through a configured binding. Device 20 holds AO-1, whose
// Present_Value takes a priority write; AI-1, whose Present_Value refuses a
// write while it is in service; and CH-5, whose Channel_Number refuses NULL
// as the wrong datatype. The test polls each Channel's Write_Status every
// 5 ms until the distribution ends.
// ---------------------------------------------------------------------------

fn oid(object_type: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(object_type, instance).unwrap()
}

fn device(instance: u32) -> DeviceObject {
    DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(instance, 0)
            .expect("valid Device identity")
            .with_name(format!("Device {instance}")),
        ..DeviceConfig::default()
    })
    .unwrap()
}

/// `property` of `object` in Device 20.
fn remote(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
) -> BACnetDeviceObjectPropertyReference {
    BACnetDeviceObjectPropertyReference {
        device_identifier: Some(oid(ObjectType::DEVICE, 20)),
        ..BACnetDeviceObjectPropertyReference::new_local(object, property.to_raw())
    }
}

fn channel(instance: u32, members: Vec<BACnetDeviceObjectPropertyReference>) -> ChannelObject {
    let mut channel = ChannelObject::new(instance, format!("CH-{instance}"), 1).unwrap();
    channel.set_members(members).unwrap();
    channel
}

async fn start(db: ObjectDatabase, binding: Option<DeviceBinding>) -> BACnetServer<BipTransport> {
    let identity =
        bacnet_objects::DeviceIdentity::new(db.selected_device().unwrap().instance_number(), 0)
            .unwrap();
    let mut builder = BACnetServer::bip_builder()
        .device(identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db);
    if let Some(binding) = binding {
        builder = builder.device_binding(binding).unwrap();
    }
    builder.build().await.unwrap()
}

async fn read(
    db: &Arc<RwLock<ObjectDatabase>>,
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> PropertyValue {
    db.read()
        .await
        .get(&object)
        .unwrap()
        .read_property(property, index)
        .unwrap()
}

/// Write `value` at priority 8 to CH-`instance`'s Present_Value, wait for the
/// distribution to end, and return Write_Status and Reliability.
async fn distribute(
    server: &BACnetServer<BipTransport>,
    instance: u32,
    value: PropertyValue,
) -> (WriteStatus, Reliability) {
    let channel = oid(ObjectType::CHANNEL, instance);
    server
        .write_local(
            &channel,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            value,
            Some(8),
            bacnet_server::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    let db = server.database();
    let in_progress = PropertyValue::Enumerated(WriteStatus::IN_PROGRESS.to_raw());
    tokio::time::timeout(Duration::from_secs(10), async {
        while read(db, channel, PropertyIdentifier::WRITE_STATUS, None).await == in_progress {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the distribution ended");
    let enumerated = |value| match value {
        PropertyValue::Enumerated(raw) => raw,
        other => panic!("an enumerated value, not {other:?}"),
    };
    let status = read(db, channel, PropertyIdentifier::WRITE_STATUS, None).await;
    let reliability = read(db, channel, PropertyIdentifier::RELIABILITY, None).await;
    (
        WriteStatus::from_raw(enumerated(status)),
        Reliability::from_raw(enumerated(reliability)),
    )
}

#[tokio::test]
async fn channel_writes_members_in_another_device_over_bip() {
    let ao = oid(ObjectType::ANALOG_OUTPUT, 1);
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let ch5 = oid(ObjectType::CHANNEL, 5);
    let mut target_db = ObjectDatabase::new();
    target_db.add(Box::new(device(20))).unwrap();
    target_db
        .add(Box::new(
            AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    target_db
        .add(Box::new(
            AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    target_db.add(Box::new(channel(5, Vec::new()))).unwrap();
    let mut target = start(target_db, None).await;

    let pv = PropertyIdentifier::PRESENT_VALUE;
    let mut channels_db = ObjectDatabase::new();
    channels_db.add(Box::new(device(10))).unwrap();
    channels_db
        .add(Box::new(channel(1, vec![remote(ao, pv)])))
        .unwrap();
    channels_db
        .add(Box::new(channel(2, vec![remote(ai, pv)])))
        .unwrap();
    let number = PropertyIdentifier::CHANNEL_NUMBER;
    channels_db
        .add(Box::new(channel(
            3,
            vec![remote(ao, pv), remote(ch5, number)],
        )))
        .unwrap();
    let binding = DeviceBinding::local(oid(ObjectType::DEVICE, 20), target.local_mac()).unwrap();
    let mut channels = start(channels_db, Some(binding)).await;

    let slot8 = || {
        read(
            target.database(),
            ao,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
        )
    };
    assert_eq!(
        distribute(&channels, 1, PropertyValue::Real(42.0)).await,
        (WriteStatus::SUCCESSFUL, Reliability::NO_FAULT_DETECTED)
    );
    assert_eq!(slot8().await, PropertyValue::Real(42.0));

    // AI-1 refuses the write in service.
    assert_eq!(
        distribute(&channels, 2, PropertyValue::Real(1.0)).await,
        (WriteStatus::FAILED, Reliability::PROCESS_ERROR)
    );

    // NULL relinquishes AO-1, and CH-5's refusal of NULL as the wrong
    // datatype doesn't fail the distribution.
    assert_eq!(
        distribute(&channels, 3, PropertyValue::Null).await,
        (WriteStatus::SUCCESSFUL, Reliability::NO_FAULT_DETECTED)
    );
    assert_eq!(slot8().await, PropertyValue::Null);
    assert_eq!(
        read(target.database(), ch5, number, None).await,
        PropertyValue::Unsigned(1)
    );

    channels.stop().await.unwrap();
    target.stop().await.unwrap();
}

// A Channel reads a remote member's property to learn its datatype, and
// coerces its value to it (#1342, Table 12-63): Device 20's BO-1 takes a REAL
// 1.0 as ACTIVE.
#[tokio::test]
async fn channel_coerces_its_value_for_a_member_in_another_device_over_bip() {
    let bo = oid(ObjectType::BINARY_OUTPUT, 1);
    let mut target_db = ObjectDatabase::new();
    target_db.add(Box::new(device(20))).unwrap();
    target_db
        .add(Box::new(BinaryOutputObject::new(1, "BO-1").unwrap()))
        .unwrap();
    let mut target = start(target_db, None).await;

    let pv = PropertyIdentifier::PRESENT_VALUE;
    let mut channels_db = ObjectDatabase::new();
    channels_db.add(Box::new(device(10))).unwrap();
    channels_db
        .add(Box::new(channel(1, vec![remote(bo, pv)])))
        .unwrap();
    let binding = DeviceBinding::local(oid(ObjectType::DEVICE, 20), target.local_mac()).unwrap();
    let mut channels = start(channels_db, Some(binding)).await;

    let slot8 = || {
        read(
            target.database(),
            bo,
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(8),
        )
    };
    let active = PropertyValue::Enumerated(1);
    assert_eq!(
        distribute(&channels, 1, PropertyValue::Real(1.0)).await,
        (WriteStatus::SUCCESSFUL, Reliability::NO_FAULT_DETECTED)
    );
    assert_eq!(slot8().await, active);
    // The datatype is kept, and Rule 5 takes 0.0 to INACTIVE.
    assert_eq!(
        distribute(&channels, 1, PropertyValue::Real(0.0)).await,
        (WriteStatus::SUCCESSFUL, Reliability::NO_FAULT_DETECTED)
    );
    assert_eq!(slot8().await, PropertyValue::Enumerated(0));

    channels.stop().await.unwrap();
    target.stop().await.unwrap();
}
