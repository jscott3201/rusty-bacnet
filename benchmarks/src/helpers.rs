//! Shared helpers for benchmark setup.

use std::net::{Ipv4Addr, Ipv6Addr};

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bip6::Bip6Transport;
use bacnet_types::error::Error;

/// Create a populated ObjectDatabase for benchmarking.
/// Used by all transport benchmarks for consistent object sets.
pub fn make_benchmark_db(device_instance: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();

    let device = DeviceObject::new(DeviceConfig {
        instance: device_instance,
        name: "Benchmark Device".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    for i in 1..=10 {
        let mut ai = AnalogInputObject::new(i, format!("AI-{}", i), 62).unwrap();
        ai.set_present_value(20.0 + i as f32);
        db.add(Box::new(ai)).unwrap();
    }

    let ao = AnalogOutputObject::new(1, "AO-1", 62).unwrap();
    db.add(Box::new(ao)).unwrap();

    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    db.add(Box::new(bv)).unwrap();

    db
}

/// Create a server with a populated database on an ephemeral port.
pub async fn make_bip_server() -> Result<BACnetServer<BipTransport>, Error> {
    let db = make_benchmark_db(1234);

    BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(db)
        .build()
        .await
}

/// Create a BIP client on an ephemeral port.
pub async fn make_bip_client() -> Result<BACnetClient<BipTransport>, Error> {
    BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(2000)
        .build()
        .await
}

/// Create a BIP6 server with a populated database on an ephemeral port.
pub async fn make_bip6_server() -> Result<BACnetServer<Bip6Transport>, Error> {
    let db = make_benchmark_db(2345);
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);

    BACnetServer::<Bip6Transport>::generic_builder()
        .transport(transport)
        .database(db)
        .build()
        .await
}

/// Create a BIP6 client on an ephemeral port.
pub async fn make_bip6_client() -> Result<BACnetClient<Bip6Transport>, Error> {
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, None);

    BACnetClient::<Bip6Transport>::generic_builder()
        .transport(transport)
        .apdu_timeout_ms(2000)
        .build()
        .await
}

/// Get current process RSS in bytes.
pub fn current_rss_bytes() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}
