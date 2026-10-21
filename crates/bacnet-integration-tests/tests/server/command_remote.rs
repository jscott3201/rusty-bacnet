use super::*;
use bacnet_objects::analog::AnalogOutputObject;
use bacnet_objects::command::CommandObject;
use bacnet_server::server::{DeviceBinding, ServerConfig};
use bacnet_transport::bbmd::BdtEntry;
use bacnet_transport::bvll::decode_bip_mac;
use bacnet_types::constructed::{BACnetActionCommand, BACnetActionList};
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::primitives::PropertyValue;
use tokio::sync::RwLock;

// ---------------------------------------------------------------------------
// Command actions naming another device (#1180, Clause 12.10.8)
//
// Two servers on loopback B/IP. Device 10 holds CMD-1 and reaches Device 20
// through a configured binding, or finds it with a Who-Is (#1322). Device 20
// holds AO-1, whose Present_Value takes a priority write, and AI-1, whose
// Present_Value refuses a write while it is in service.
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

/// A command for Device 20's `object` Present_Value.
fn remote(object: ObjectIdentifier, value: f32, quit_on_failure: bool) -> BACnetActionCommand {
    BACnetActionCommand {
        device_identifier: Some(oid(ObjectType::DEVICE, 20)),
        object_identifier: object,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: PropertyValue::Real(value),
        priority: Some(8),
        post_delay: None,
        quit_on_failure,
        write_successful: true,
    }
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

/// Write `list` to CMD-1's Present_Value, wait for the run to end, and return
/// All_Writes_Successful and each command's write-successful flag.
async fn run(server: &BACnetServer<BipTransport>, list: u64) -> (bool, Vec<bool>) {
    let cmd = oid(ObjectType::COMMAND, 1);
    server
        .write_local(
            &cmd,
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(list),
            None,
            bacnet_server::LocalCommandSource::ServerDevice,
        )
        .await
        .unwrap();
    let db = server.database();
    tokio::time::timeout(Duration::from_secs(10), async {
        while read(db, cmd, PropertyIdentifier::IN_PROCESS, None).await
            != PropertyValue::Boolean(false)
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the run ended with In_Process FALSE");
    let all = read(db, cmd, PropertyIdentifier::ALL_WRITES_SUCCESSFUL, None).await;
    let PropertyValue::ApplicationData(element) = read(
        db,
        cmd,
        PropertyIdentifier::ACTION,
        Some(u32::try_from(list).unwrap()),
    )
    .await
    else {
        panic!("an encoded action list");
    };
    let (decoded, _) = bacnet_encoding::constructed::decode_action_list(&element, 0).unwrap();
    let flags = decoded
        .commands
        .iter()
        .map(|command| command.write_successful)
        .collect();
    (all == PropertyValue::Boolean(true), flags)
}

/// Device 20 with AO-1 and AI-1, on its own port.
async fn start_target() -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device(20))).unwrap();
    db.add(Box::new(
        AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogInputObject::new(1, "AI-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    start(db, None).await
}

/// Device 10 with CMD-1, whose list 1 writes AO-1 in Device 20.
fn commander_db() -> ObjectDatabase {
    let ao = oid(ObjectType::ANALOG_OUTPUT, 1);
    let ai = oid(ObjectType::ANALOG_INPUT, 1);
    let mut command = CommandObject::new(1, "CMD-1").unwrap();
    command
        .set_action(vec![
            BACnetActionList {
                commands: vec![remote(ao, 42.0, false)],
            },
            // AI-1 refuses; the failure quits before AO-1 is written again.
            BACnetActionList {
                commands: vec![remote(ai, 1.0, true), remote(ao, 7.0, false)],
            },
        ])
        .unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(device(10))).unwrap();
    db.add(Box::new(command)).unwrap();
    db
}

#[tokio::test]
async fn command_writes_a_target_in_another_device_over_bip() {
    let ao = oid(ObjectType::ANALOG_OUTPUT, 1);
    let mut target = start_target().await;
    let binding = DeviceBinding::local(oid(ObjectType::DEVICE, 20), target.local_mac()).unwrap();
    let mut commander = start(commander_db(), Some(binding)).await;

    assert_eq!(run(&commander, 1).await, (true, vec![true]));
    let slot8 = read(
        target.database(),
        ao,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
    )
    .await;
    assert_eq!(slot8, PropertyValue::Real(42.0));

    assert_eq!(run(&commander, 2).await, (false, vec![false, false]));
    let slot8 = read(
        target.database(),
        ao,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
    )
    .await;
    assert_eq!(slot8, PropertyValue::Real(42.0));

    commander.stop().await.unwrap();
    target.stop().await.unwrap();
}

#[tokio::test]
async fn command_finds_an_unbound_target_with_a_who_is_over_bip() {
    let ao = oid(ObjectType::ANALOG_OUTPUT, 1);
    let mut target = start_target().await;
    // The commander has no binding for Device 20. On loopback each server has
    // its own port, so a local broadcast reaches only the sender; making the
    // commander a BBMD whose table lists the target carries its broadcasts,
    // the global Who-Is included, to the target as Forwarded-NPDUs. The
    // target answers with a directed I-Am.
    let (ip, port) = decode_bip_mac(target.local_mac()).unwrap();
    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    transport.enable_bbmd(vec![BdtEntry {
        ip,
        port,
        broadcast_mask: [255; 4],
    }]);
    let mut commander = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(10, 0).unwrap(),
            ..Default::default()
        },
        commander_db(),
        transport,
    )
    .await
    .unwrap();

    assert_eq!(run(&commander, 1).await, (true, vec![true]));
    let slot8 = read(
        target.database(),
        ao,
        PropertyIdentifier::PRIORITY_ARRAY,
        Some(8),
    )
    .await;
    assert_eq!(slot8, PropertyValue::Real(42.0));

    commander.stop().await.unwrap();
    target.stop().await.unwrap();
}
