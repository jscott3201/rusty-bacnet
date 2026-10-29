//! Common harness helpers for stress test scenarios.

use std::net::Ipv4Addr;

use super::client_diagnostics::FailureKind;
use bacnet_client::client::BACnetClient;
use bacnet_objects::analog::{AnalogInputObject, AnalogOutputObject};
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::ObjectType;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// Build an ObjectDatabase with `object_count` AnalogInput objects, plus
/// one AnalogOutput, one BinaryValue, and the required Device object.
pub fn make_large_db(device_instance: u32, object_count: u32) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();

    let mut device = DeviceObject::new(DeviceConfig {
        instance: device_instance,
        name: "Stress Device".into(),
        vendor_name: "Rusty BACnet".into(),
        vendor_id: 555,
        ..DeviceConfig::default()
    })
    .unwrap();
    let oid = |kind, instance| ObjectIdentifier::new(kind, instance).unwrap();
    let mut objects = vec![oid(ObjectType::DEVICE, device_instance)];
    objects.extend((1..=object_count).map(|i| oid(ObjectType::ANALOG_INPUT, i)));
    objects.push(oid(ObjectType::ANALOG_OUTPUT, 1));
    objects.push(oid(ObjectType::BINARY_VALUE, 1));
    device.set_object_list(objects);
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
        .vendor_id(555)
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

pub fn classify(error: &Error) -> FailureKind {
    match error {
        Error::Timeout(_) => FailureKind::Timeout,
        Error::Abort { reason } => FailureKind::Abort { reason: *reason },
        Error::Reject { reason } => FailureKind::Reject { reason: *reason },
        Error::Protocol { class, code } | Error::Structured { class, code, .. } => {
            FailureKind::Protocol {
                class: *class,
                code: *code,
            }
        }
        Error::Transport(error) => FailureKind::Transport {
            io_kind: format!("{:?}", error.kind()),
        },
        _ => FailureKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn historical_raw_codes_match_the_current_normalized_categories() {
        let cases = [
            (
                Error::Timeout(std::time::Duration::from_secs(5)),
                FailureKind::Timeout,
            ),
            (Error::Abort { reason: 1 }, FailureKind::Abort { reason: 1 }),
            (
                Error::Abort { reason: 65 },
                FailureKind::Abort { reason: 65 },
            ),
            (
                Error::Reject { reason: 9 },
                FailureKind::Reject { reason: 9 },
            ),
            (
                Error::Protocol { class: 2, code: 32 },
                FailureKind::Protocol { class: 2, code: 32 },
            ),
            (
                Error::Transport(std::io::ErrorKind::ConnectionRefused.into()),
                FailureKind::Transport {
                    io_kind: "ConnectionRefused".into(),
                },
            ),
            (Error::Encoding("fixture".into()), FailureKind::Other),
        ];
        for (error, expected) in cases {
            assert_eq!(classify(&error), expected);
        }
    }
}
