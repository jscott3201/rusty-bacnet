//! Common harness helpers for stress test scenarios.

use std::net::Ipv4Addr;

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::error::Error;

/// Build an ObjectDatabase with `object_count` AnalogInput objects, plus
/// one AnalogOutput, one BinaryValue, and the required Device object.
pub fn make_large_db(device_instance: u32, object_count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();

    let device = DeviceObject::new(DeviceConfig {
        instance: device_instance,
        name: "Stress Device".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    for i in 1..=object_count {
        let mut ai = AnalogInputObject::new(i, format!("AI-{}", i), 62).unwrap();
        ai.set_present_value(20.0 + (i as f32 * 0.1));
        db.add(Box::new(ai)).unwrap();
    }

    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();

    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    db.add(Box::new(bv)).unwrap();

    db
}

/// Build a BIP server on localhost with the given database.
pub async fn make_bip_server_with_db(
    db: ObjectDatabase,
) -> Result<BACnetServer<BipTransport>, Error> {
    BACnetServer::bip_builder()
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
