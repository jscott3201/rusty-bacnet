//! The standalone PICS reads its executed services from the selected Device
//! (#1204).

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_types::enums::ServiceSupported;

use super::{PicsConfig, generate_pics};
use crate::server::ServerConfig;

fn device(instance: u32, services: &[ServiceSupported]) -> Box<DeviceObject> {
    let mut device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(instance, 0)
            .expect("valid Device identity")
            .with_name(format!("Device {instance}")),
        ..DeviceConfig::default()
    })
    .unwrap();
    device.set_services_supported(services);
    Box::new(device)
}

/// Device 100 declares ReadProperty only and Device 200 adds WriteProperty.
/// The PICS follows the lowest, Device 100, whatever order the Devices were
/// added in and however a fresh database's hash order visits them.
#[test]
fn standalone_pics_reads_the_lowest_of_several_devices() {
    let read = [ServiceSupported::READ_PROPERTY];
    let read_write = [
        ServiceSupported::READ_PROPERTY,
        ServiceSupported::WRITE_PROPERTY,
    ];
    for _ in 0..16 {
        for lowest_first in [true, false] {
            let mut db = ObjectDatabase::new();
            let (first, second) = if lowest_first {
                (device(100, &read), device(200, &read_write))
            } else {
                (device(200, &read_write), device(100, &read))
            };
            db.add(first).unwrap();
            db.add(second).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            let executes = |name: &str| {
                pics.supported_services
                    .iter()
                    .any(|service| service.service_name == name && service.executor)
            };
            assert!(executes("ReadProperty"), "lowest first: {lowest_first}");
            assert!(!executes("WriteProperty"), "lowest first: {lowest_first}");
        }
    }
}
