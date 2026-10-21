//! Common harness helpers for stress test scenarios.

use std::net::Ipv4Addr;

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{EngineeringUnits, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// Build an ObjectDatabase with `object_count` AnalogInput objects, plus
/// one AnalogOutput, one BinaryValue, and the required Device object.
pub fn make_large_db(device_instance: u32, object_count: u32) -> ObjectDatabase {
    make_large_db_with(stress_device_config(device_instance), object_count)
}

/// The stress Device's configuration, before any scenario's changes.
pub fn stress_device_config(device_instance: u32) -> DeviceConfig {
    DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(device_instance, 555)
            .expect("valid Device identity")
            .with_name("Stress Device"),
        vendor_name: "Rusty BACnet".into(),
        ..DeviceConfig::default()
    }
}

/// [`make_large_db`] with the Device built from `config`. The server
/// refuses a Device whose Max_APDU_Length_Accepted or
/// Segmentation_Supported differs from its own: 480 octets on an MS/TP port
/// without extended frames, say.
pub fn make_large_db_with(config: DeviceConfig, object_count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let device_instance = config.identity.instance();
    let mut device = DeviceObject::new(config).unwrap();
    // The server leaves Object_List to its caller: list every object.
    let oid = |kind, instance| ObjectIdentifier::new(kind, instance).unwrap();
    let mut objects = vec![oid(ObjectType::DEVICE, device_instance)];
    objects.extend((1..=object_count).map(|i| oid(ObjectType::ANALOG_INPUT, i)));
    objects.push(oid(ObjectType::ANALOG_OUTPUT, 1));
    objects.push(oid(ObjectType::BINARY_VALUE, 1));
    device.set_object_list(objects);
    db.add(Box::new(device)).unwrap();

    for i in 1..=object_count {
        let mut ai =
            AnalogInputObject::new(i, format!("AI-{}", i), EngineeringUnits::DEGREES_CELSIUS)
                .unwrap();
        ai.set_present_value(20.0 + (i as f32 * 0.1));
        db.add(Box::new(ai)).unwrap();
    }

    let ao = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    db.add(Box::new(ao)).unwrap();

    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    db.add(Box::new(bv)).unwrap();

    db
}

/// Build a BIP server on localhost with the given database.
pub async fn make_bip_server_with_db(
    db: ObjectDatabase,
) -> Result<BACnetServer<BipTransport>, Error> {
    let instance = db
        .selected_device()
        .expect("stress Device")
        .instance_number();
    BACnetServer::bip_builder()
        .device(stress_device_config(instance).identity)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await
}

/// Build a BIP client on localhost with an ephemeral port.
pub async fn make_stress_client() -> Result<BACnetClient<BipTransport>, Error> {
    BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(5000)
        .build()
        .await
}

/// Get current process RSS in kilobytes.
pub fn current_rss_kb() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory() / 1024).unwrap_or(0)
}
