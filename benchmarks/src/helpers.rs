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
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::Error;

/// Create a populated ObjectDatabase for benchmarking.
/// Used by all transport benchmarks for consistent object sets.
pub fn make_benchmark_db(device_instance: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();

    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(device_instance, 555)
            .expect("valid Device identity")
            .with_name("Benchmark Device"),
        vendor_name: "Rusty BACnet".into(),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();

    for i in 1..=10 {
        let mut ai =
            AnalogInputObject::new(i, format!("AI-{}", i), EngineeringUnits::DEGREES_CELSIUS)
                .unwrap();
        ai.set_present_value(20.0 + i as f32);
        db.add(Box::new(ai)).unwrap();
    }

    let ao = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS).unwrap();
    db.add(Box::new(ao)).unwrap();

    let bv = BinaryValueObject::new(1, "BV-1").unwrap();
    db.add(Box::new(bv)).unwrap();

    db
}

/// Create a server with a populated database on an ephemeral port.
pub async fn make_bip_server() -> Result<BACnetServer<BipTransport>, Error> {
    let db = make_benchmark_db(1234);

    BACnetServer::bip_builder()
        .device(bacnet_objects::DeviceIdentity::new(1234, 555)?)
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

/// Create a BIP6 server with a populated database on an ephemeral port. Its
/// VMAC is the one its device instance, 2345, gives (U.1.3).
pub async fn make_bip6_server() -> Result<BACnetServer<Bip6Transport>, Error> {
    let db = make_benchmark_db(2345);
    let transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(2345));

    BACnetServer::<Bip6Transport>::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(2345, 555)?)
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

/// The MAC a BIP6 client addresses `server` by: the server's VMAC, which the
/// client finds from the server's B/IPv6 address with
/// Virtual-Address-Resolution (U.5). The client then knows where that VMAC
/// is, so no request waits on resolution.
pub async fn resolve_bip6_server(
    client: &BACnetClient<Bip6Transport>,
    server: &BACnetServer<Bip6Transport>,
) -> Result<Vec<u8>, Error> {
    let address = server
        .transport()
        .and_then(Bip6Transport::local_address)
        .ok_or_else(|| Error::Encoding("the BIP6 server is not running".into()))?;
    let vmac = client.transport().resolve_virtual_address(address).await?;
    Ok(vmac.octets().to_vec())
}

/// Get current process RSS in bytes.
pub fn current_rss_bytes() -> u64 {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let pid = Pid::from(std::process::id() as usize);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[pid]), true);
    sys.process(pid).map(|p| p.memory()).unwrap_or(0)
}
