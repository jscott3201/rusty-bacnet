//! TOML configuration parsing and validation.

use serde::Deserialize;
use std::collections::HashSet;

/// Top-level gateway configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    /// HTTP/MCP server settings.
    #[serde(default)]
    pub server: ServerConfig,
    /// Local BACnet device identity.
    pub device: DeviceConfig,
    /// Transport configurations.
    #[serde(default)]
    pub transports: TransportsConfig,
    /// BBMD configuration (mutually exclusive with foreign_device).
    pub bbmd: Option<BbmdConfig>,
    /// Foreign device registration (mutually exclusive with bbmd).
    pub foreign_device: Option<ForeignDeviceConfig>,
    /// Static routing table entries.
    #[serde(default)]
    pub routes: Vec<RouteConfig>,
    /// Pre-populated local objects.
    #[serde(default)]
    pub objects: Vec<ObjectConfig>,
}

/// HTTP/MCP server settings.
#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    /// Bind address for the HTTP server.
    #[serde(default = "default_bind")]
    pub bind: String,
    /// API key for bearer token auth. If omitted, no auth is applied.
    pub api_key: Option<String>,
    /// Read-only mode. When true, all write operations (write_property,
    /// create_object, delete_object) are rejected.
    #[serde(default)]
    pub read_only: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            api_key: None,
            read_only: false,
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:3000".to_string()
}

/// Local BACnet device identity.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceConfig {
    /// Device instance number (0–4194302).
    pub instance: u32,
    /// Device object name.
    pub name: String,
    /// Vendor identifier.
    #[serde(default = "default_vendor_id")]
    pub vendor_id: u16,
    /// Device description.
    #[serde(default)]
    pub description: String,
}

fn default_vendor_id() -> u16 {
    999
}

/// Transport configurations.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TransportsConfig {
    /// BACnet/IP transport.
    pub bip: Option<BipConfig>,
    /// BACnet/SC transport.
    pub sc: Option<ScConfig>,
    /// MS/TP transport (Linux only).
    pub mstp: Option<MstpConfig>,
}

/// BACnet/IP transport configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct BipConfig {
    /// Bind interface address.
    #[serde(default = "default_interface")]
    pub interface: String,
    /// UDP port.
    #[serde(default = "default_bip_port")]
    pub port: u16,
    /// Subnet broadcast address.
    pub broadcast: String,
    /// Network number for this transport.
    pub network_number: u16,
}

fn default_interface() -> String {
    "0.0.0.0".to_string()
}

fn default_bip_port() -> u16 {
    47808
}

/// BACnet/SC transport configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ScConfig {
    /// WebSocket hub URI.
    pub hub_uri: String,
    /// TLS client certificate path.
    pub cert: String,
    /// TLS private key path.
    pub key: String,
    /// CA certificate path.
    #[serde(default)]
    pub ca: Option<String>,
    /// Network number for this transport.
    pub network_number: u16,
}

/// MS/TP transport configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct MstpConfig {
    /// Serial port path.
    pub serial_port: String,
    /// Baud rate.
    #[serde(default = "default_baud_rate")]
    pub baud_rate: u32,
    /// Station address (0–254).
    pub station_address: u8,
    /// Max master station address.
    #[serde(default = "default_max_master")]
    pub max_master: u8,
    /// Network number for this transport.
    pub network_number: u16,
}

fn default_baud_rate() -> u32 {
    76800
}

fn default_max_master() -> u8 {
    127
}

/// BBMD configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct BbmdConfig {
    /// Enable BBMD on the BIP transport.
    #[serde(default)]
    pub enabled: bool,
    /// Initial Broadcast Distribution Table entries (IP:port strings).
    #[serde(default)]
    pub bdt: Vec<String>,
}

/// Foreign device registration configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ForeignDeviceConfig {
    /// BBMD address to register with (IP:port).
    pub bbmd: String,
    /// Time-to-live in seconds.
    pub ttl: u16,
}

/// Static route entry.
#[derive(Debug, Clone, Deserialize)]
pub struct RouteConfig {
    /// Destination network number.
    pub network: u16,
    /// Transport to route through ("bip", "sc", "mstp").
    pub via_transport: String,
    /// Next hop address (optional, for routed networks).
    pub next_hop: Option<String>,
}

/// Pre-populated local object.
#[derive(Debug, Clone, Deserialize)]
pub struct ObjectConfig {
    /// Object type name (e.g., "analog-value").
    #[serde(rename = "type")]
    pub object_type: String,
    /// Object instance number.
    pub instance: u32,
    /// Object name.
    pub name: String,
    /// Engineering units (optional).
    pub units: Option<String>,
    /// Number of states for multi-state objects (default: 2).
    pub number_of_states: Option<u32>,
}

/// Configuration validation error.
#[derive(Debug, Clone)]
pub struct ConfigError {
    pub message: String,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config error: {}", self.message)
    }
}

impl std::error::Error for ConfigError {}

impl GatewayConfig {
    /// Parse a TOML string into a GatewayConfig.
    pub fn from_toml(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Device instance range
        if self.device.instance > 4_194_302 {
            return Err(ConfigError {
                message: format!(
                    "device instance {} exceeds maximum 4194302",
                    self.device.instance
                ),
            });
        }

        // BBMD and foreign_device are mutually exclusive
        if self.bbmd.is_some() && self.foreign_device.is_some() {
            return Err(ConfigError {
                message: "[bbmd] and [foreign_device] are mutually exclusive".to_string(),
            });
        }

        // BBMD requires BIP transport
        if let Some(bbmd) = &self.bbmd {
            if bbmd.enabled && self.transports.bip.is_none() {
                return Err(ConfigError {
                    message: "[bbmd] requires [transports.bip] to be configured".to_string(),
                });
            }
        }

        // MS/TP only on Linux
        #[cfg(not(target_os = "linux"))]
        if self.transports.mstp.is_some() {
            return Err(ConfigError {
                message: "[transports.mstp] is only available on Linux".to_string(),
            });
        }

        // Validate and check uniqueness of network numbers.
        let mut network_numbers = HashSet::new();
        let transport_networks: Vec<(u16, &str)> = [
            self.transports
                .bip
                .as_ref()
                .map(|t| (t.network_number, "bip")),
            self.transports
                .sc
                .as_ref()
                .map(|t| (t.network_number, "sc")),
            self.transports
                .mstp
                .as_ref()
                .map(|t| (t.network_number, "mstp")),
        ]
        .into_iter()
        .flatten()
        .collect();

        for (num, name) in &transport_networks {
            if *num == 0 {
                return Err(ConfigError {
                    message: format!(
                        "{name} network_number 0 is reserved (local-only, no routing)"
                    ),
                });
            }
            if *num == 65535 {
                return Err(ConfigError {
                    message: format!("{name} network_number 65535 is reserved (broadcast)"),
                });
            }
            if !network_numbers.insert(*num) {
                return Err(ConfigError {
                    message: format!("duplicate network number {num}"),
                });
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_full_config() {
        let toml = r#"
[server]
bind = "0.0.0.0:3000"
api_key = "test-key"

[device]
instance = 389001
name = "Test Gateway"
vendor_id = 555

[transports.bip]
interface = "0.0.0.0"
port = 47808
broadcast = "192.168.1.255"
network_number = 1

[transports.sc]
hub_uri = "wss://hub.example.com"
cert = "certs/client.pem"
key = "certs/client.key"
network_number = 2

[[routes]]
network = 4
via_transport = "bip"
next_hop = "192.168.1.100:47808"

[[objects]]
type = "analog-value"
instance = 1
name = "Gateway Uptime"
units = "seconds"
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        assert_eq!(config.server.bind, "0.0.0.0:3000");
        assert_eq!(config.server.api_key.as_deref(), Some("test-key"));
        assert_eq!(config.device.instance, 389001);
        assert_eq!(config.device.name, "Test Gateway");
        assert_eq!(config.device.vendor_id, 555);
        assert!(config.transports.bip.is_some());
        assert!(config.transports.sc.is_some());
        assert!(config.transports.mstp.is_none());
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].network, 4);
        assert_eq!(config.objects.len(), 1);
        assert_eq!(config.objects[0].object_type, "analog-value");
        config.validate().unwrap();
    }

    #[test]
    fn parse_minimal_config() {
        let toml = r#"
[device]
instance = 1234
name = "Minimal"
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        assert_eq!(config.server.bind, "127.0.0.1:3000");
        assert!(config.server.api_key.is_none());
        assert_eq!(config.device.vendor_id, 999);
        assert!(config.transports.bip.is_none());
        assert!(config.routes.is_empty());
        assert!(config.objects.is_empty());
        config.validate().unwrap();
    }

    #[test]
    fn validate_device_instance_too_large() {
        let toml = r#"
[device]
instance = 4194303
name = "Bad"
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.message.contains("4194303"));
    }

    #[test]
    fn validate_bbmd_and_foreign_device_mutually_exclusive() {
        let toml = r#"
[device]
instance = 1
name = "Test"

[transports.bip]
broadcast = "192.168.1.255"
network_number = 1

[bbmd]
enabled = true

[foreign_device]
bbmd = "192.168.1.1:47808"
ttl = 300
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.message.contains("mutually exclusive"));
    }

    #[test]
    fn validate_bbmd_requires_bip() {
        let toml = r#"
[device]
instance = 1
name = "Test"

[bbmd]
enabled = true
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.message.contains("requires [transports.bip]"));
    }

    #[test]
    fn validate_duplicate_network_numbers() {
        let toml = r#"
[device]
instance = 1
name = "Test"

[transports.bip]
broadcast = "192.168.1.255"
network_number = 1

[transports.sc]
hub_uri = "wss://hub.example.com"
cert = "c.pem"
key = "k.pem"
network_number = 1
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.message.contains("duplicate network number"));
    }

    #[test]
    fn validate_network_number_zero_rejected() {
        let toml = r#"
[device]
instance = 1
name = "Test"

[transports.bip]
broadcast = "192.168.1.255"
network_number = 0
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.message.contains("reserved"));
    }

    #[test]
    fn validate_network_number_broadcast_rejected() {
        let toml = r#"
[device]
instance = 1
name = "Test"

[transports.bip]
broadcast = "192.168.1.255"
network_number = 65535
"#;
        let config = GatewayConfig::from_toml(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.message.contains("reserved"));
    }
}
