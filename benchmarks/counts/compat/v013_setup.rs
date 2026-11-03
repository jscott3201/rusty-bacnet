//! `src/setup.rs` for a v0.13.0 tree (#1979 S1). `perf_counts.py run
//! --baseline-compat v013` puts this file in place of the harness's
//! `src/setup.rs` in the baseline tree only, because the Device and object
//! constructors and the server builder changed after v0.13.0. It builds the
//! same object set with v0.13.0's API: Device 1234 (vendor 555), Analog
//! Inputs 1 to 10 with present values 21.0 to 30.0 in degrees Celsius
//! (units code 62), Analog Output 1 and Binary Value 1. Keep it in step
//! with `src/setup.rs`; it never builds against the current tree.

use std::net::Ipv4Addr;

use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::error::Error;

/// The server's device instance.
pub const DEVICE_INSTANCE: u32 = 1234;

/// v0.13.0 takes engineering units as their numeric code: degrees Celsius.
const DEGREES_CELSIUS: u32 = 62;

/// The objects every round-trip workload reads or writes (see
/// `src/setup.rs`).
pub fn benchmark_db() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        instance: DEVICE_INSTANCE,
        name: "Benchmark Device".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .expect("valid Device");
    db.add(Box::new(device)).expect("add Device");
    for i in 1..=10 {
        let mut ai = AnalogInputObject::new(i, format!("AI-{i}"), DEGREES_CELSIUS)
            .expect("valid Analog Input");
        ai.set_present_value(20.0 + i as f32);
        db.add(Box::new(ai)).expect("add Analog Input");
    }
    let ao = AnalogOutputObject::new(1, "AO-1", DEGREES_CELSIUS).expect("valid Analog Output");
    db.add(Box::new(ao)).expect("add Analog Output");
    let bv = BinaryValueObject::new(1, "BV-1").expect("valid Binary Value");
    db.add(Box::new(bv)).expect("add Binary Value");
    db
}

/// A B/IP server on 127.0.0.1, ephemeral port, serving [`benchmark_db`].
pub async fn bip_server() -> Result<BACnetServer<BipTransport>, Error> {
    BACnetServer::bip_builder()
        .vendor_id(555)
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
