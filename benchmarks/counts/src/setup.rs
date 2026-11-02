//! The B/IP loopback pair: a server with the benchmark object set and a
//! client, both on ephemeral ports. A copy of
//! `bacnet_benchmarks::helpers::{make_benchmark_db, make_bip_server,
//! make_bip_client}`, so this package needs neither that crate nor its
//! sc-tls dependencies; keep the object set in step with it, since
//! `bip_latency` measures the same workloads in wall-clock time.

use std::net::Ipv4Addr;

use bacnet_client::client::BACnetClient;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::EngineeringUnits;
use bacnet_types::error::Error;

/// The server's device instance.
pub const DEVICE_INSTANCE: u32 = 1234;

/// The objects every round-trip workload reads or writes: Device 1234,
/// Analog Inputs 1 to 10 with present values 21.0 to 30.0 (20 plus the
/// instance), Analog Output 1 and Binary Value 1.
pub fn benchmark_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: DeviceIdentity::new(DEVICE_INSTANCE, 555)
            .expect("valid Device identity")
            .with_name("Benchmark Device"),
        vendor_name: "Rusty BACnet".into(),
        ..DeviceConfig::default()
    })
    .expect("valid Device");
    db.add(Box::new(device)).expect("add Device");
    for i in 1..=10 {
        let mut ai =
            AnalogInputObject::new(i, format!("AI-{i}"), EngineeringUnits::DEGREES_CELSIUS)
                .expect("valid Analog Input");
        ai.set_present_value(20.0 + i as f32);
        db.add(Box::new(ai)).expect("add Analog Input");
    }
    let ao = AnalogOutputObject::new(1, "AO-1", EngineeringUnits::DEGREES_CELSIUS)
        .expect("valid Analog Output");
    db.add(Box::new(ao)).expect("add Analog Output");
    let bv = BinaryValueObject::new(1, "BV-1").expect("valid Binary Value");
    db.add(Box::new(bv)).expect("add Binary Value");
    db
}

/// A B/IP server on 127.0.0.1, ephemeral port, serving [`benchmark_db`].
pub async fn bip_server() -> Result<BACnetServer<BipTransport>, Error> {
    BACnetServer::bip_builder()
        .device(DeviceIdentity::new(DEVICE_INSTANCE, 555)?)
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .database(benchmark_db())
        .build()
        .await
}

/// A B/IP client on 127.0.0.1, ephemeral port.
pub async fn bip_client() -> Result<BACnetClient<BipTransport>, Error> {
    BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(2000)
        .build()
        .await
}
