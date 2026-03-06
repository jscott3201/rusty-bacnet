use std::sync::Arc;

use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::ObjectIdentifier;

use crate::errors::BacnetError;

/// BACnet object identifier wrapper for Java/Kotlin.
#[derive(uniffi::Object)]
pub struct BacnetObjectIdentifier {
    pub(crate) inner: ObjectIdentifier,
}

#[uniffi::export]
impl BacnetObjectIdentifier {
    #[uniffi::constructor]
    pub fn new(object_type: u32, instance: u32) -> Result<Arc<Self>, BacnetError> {
        let oid = ObjectIdentifier::new(ObjectType::from_raw(object_type), instance)
            .map_err(BacnetError::from)?;
        Ok(Arc::new(Self { inner: oid }))
    }

    pub fn object_type(&self) -> u32 {
        self.inner.object_type().to_raw()
    }

    pub fn instance(&self) -> u32 {
        self.inner.instance_number()
    }

    pub fn display(&self) -> String {
        format!("{}", self.inner)
    }
}

/// Tagged union representing any BACnet property value.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum BacnetPropertyValue {
    Null,
    Boolean {
        value: bool,
    },
    Unsigned {
        value: u64,
    },
    Signed {
        value: i64,
    },
    Real {
        value: f32,
    },
    Double {
        value: f64,
    },
    CharacterString {
        value: String,
    },
    OctetString {
        value: Vec<u8>,
    },
    BitString {
        value: Vec<u8>,
    },
    Enumerated {
        value: u32,
    },
    ObjectId {
        object_type: u32,
        instance: u32,
    },
    Date {
        year: u8,
        month: u8,
        day: u8,
        day_of_week: u8,
    },
    Time {
        hour: u8,
        minute: u8,
        second: u8,
        hundredths: u8,
    },
}

/// Transport configuration for connecting to a BACnet network.
#[derive(Debug, Clone, uniffi::Enum)]
pub enum TransportConfig {
    Bip {
        address: String,
        port: u16,
        broadcast_address: String,
    },
    BipIpv6 {
        address: String,
        port: u16,
    },
    Sc {
        hub_url: String,
        ca_cert: Option<String>,
        client_cert: Option<String>,
        client_key: Option<String>,
        heartbeat_interval_ms: Option<u64>,
        heartbeat_timeout_ms: Option<u64>,
    },
    Mstp {
        serial_port: String,
        baud_rate: u32,
        mac_address: u8,
    },
}

/// Information about a discovered BACnet device.
#[derive(Debug, Clone, uniffi::Record)]
pub struct DiscoveredDevice {
    pub object_type: u32,
    pub instance: u32,
    pub mac_address: Vec<u8>,
    pub max_apdu_length: u32,
    pub segmentation: u8,
    pub vendor_id: u16,
    pub seconds_since_seen: f64,
}

/// A COV (Change of Value) notification received from a BACnet device.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CovNotification {
    pub process_identifier: u32,
    pub device_instance: u32,
    pub object_type: u32,
    pub object_instance: u32,
    pub time_remaining: u32,
    pub values: Vec<CovValue>,
}

/// A single property value within a COV notification.
#[derive(Debug, Clone, uniffi::Record)]
pub struct CovValue {
    pub property_id: u32,
    pub array_index: Option<u32>,
    pub value: BacnetPropertyValue,
}

/// Specification for reading multiple properties.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReadAccessSpec {
    pub object_type: u32,
    pub instance: u32,
    pub properties: Vec<PropertyRef>,
}

/// Reference to a property with optional array index.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PropertyRef {
    pub property_id: u32,
    pub array_index: Option<u32>,
}

/// Result of reading a single property in a ReadPropertyMultiple response.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ReadResult {
    pub property_id: u32,
    pub array_index: Option<u32>,
    pub value: Option<BacnetPropertyValue>,
    pub error_class: Option<u16>,
    pub error_code: Option<u16>,
}

/// Result for one object in a ReadPropertyMultiple response.
#[derive(Debug, Clone, uniffi::Record)]
pub struct ObjectReadResult {
    pub object_type: u32,
    pub instance: u32,
    pub results: Vec<ReadResult>,
}

/// Specification for writing multiple properties.
#[derive(Debug, Clone, uniffi::Record)]
pub struct WriteAccessSpec {
    pub object_type: u32,
    pub instance: u32,
    pub properties: Vec<PropertyWrite>,
}

/// A single property write within a WritePropertyMultiple request.
#[derive(Debug, Clone, uniffi::Record)]
pub struct PropertyWrite {
    pub property_id: u32,
    pub array_index: Option<u32>,
    pub value: BacnetPropertyValue,
    pub priority: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_identifier_creation() {
        let oid = BacnetObjectIdentifier::new(0, 1).unwrap();
        assert_eq!(oid.object_type(), 0);
        assert_eq!(oid.instance(), 1);
    }

    #[test]
    fn test_object_identifier_to_string() {
        let oid = BacnetObjectIdentifier::new(8, 100).unwrap(); // device:100
        let s = oid.display();
        assert!(s.contains("100"), "should contain instance: {s}");
    }

    #[test]
    fn test_property_value_real() {
        let v = BacnetPropertyValue::Real { value: 72.5 };
        match v {
            BacnetPropertyValue::Real { value } => assert_eq!(value, 72.5),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_property_value_string() {
        let v = BacnetPropertyValue::CharacterString {
            value: "hello".into(),
        };
        match v {
            BacnetPropertyValue::CharacterString { value } => assert_eq!(value, "hello"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_property_value_null() {
        let v = BacnetPropertyValue::Null;
        assert!(matches!(v, BacnetPropertyValue::Null));
    }

    #[test]
    fn test_property_value_object_id() {
        let v = BacnetPropertyValue::ObjectId {
            object_type: 0,
            instance: 42,
        };
        match v {
            BacnetPropertyValue::ObjectId {
                object_type,
                instance,
            } => {
                assert_eq!(object_type, 0);
                assert_eq!(instance, 42);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_transport_config_bip() {
        let cfg = TransportConfig::Bip {
            address: "0.0.0.0".into(),
            port: 0xBAC0,
            broadcast_address: "255.255.255.255".into(),
        };
        assert!(matches!(cfg, TransportConfig::Bip { .. }));
    }

    #[test]
    fn test_transport_config_sc() {
        let cfg = TransportConfig::Sc {
            hub_url: "wss://hub.example.com".into(),
            ca_cert: None,
            client_cert: None,
            client_key: None,
            heartbeat_interval_ms: Some(30000),
            heartbeat_timeout_ms: None,
        };
        assert!(matches!(cfg, TransportConfig::Sc { .. }));
    }

    #[test]
    fn test_discovered_device_record() {
        let dev = DiscoveredDevice {
            object_type: 8,
            instance: 100,
            mac_address: vec![192, 168, 1, 1, 0xBA, 0xC0],
            max_apdu_length: 1476,
            segmentation: 0,
            vendor_id: 555,
            seconds_since_seen: 1.5,
        };
        assert_eq!(dev.instance, 100);
        assert_eq!(dev.vendor_id, 555);
    }

    #[test]
    fn test_cov_notification_record() {
        let notif = CovNotification {
            process_identifier: 1,
            device_instance: 100,
            object_type: 0,
            object_instance: 1,
            time_remaining: 300,
            values: vec![CovValue {
                property_id: 85,
                array_index: None,
                value: BacnetPropertyValue::Real { value: 72.0 },
            }],
        };
        assert_eq!(notif.values.len(), 1);
    }
}
