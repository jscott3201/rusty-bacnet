//! Gateway builder for constructing the full BACnet stack.

use std::net::Ipv4Addr;
use std::sync::Arc;

use bacnet_client::client::BACnetClient;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig as BacnetDeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::error::Error;

use crate::config::GatewayConfig;
use crate::state::GatewayState;

/// Builder for constructing a fully wired Gateway.
pub struct GatewayBuilder {
    config: GatewayConfig,
}

impl GatewayBuilder {
    /// Create a new builder from config.
    pub fn new(config: GatewayConfig) -> Self {
        Self { config }
    }

    /// Build the gateway: create transports, start client/server, return state.
    ///
    /// Currently supports BIP transport only. SC and MS/TP will be added
    /// when the router-centric model with LoopbackTransport is wired.
    pub async fn build(self) -> Result<BuiltGateway, Error> {
        // Build object database with Device object.
        let mut db = ObjectDatabase::new();
        let device = DeviceObject::new(BacnetDeviceConfig {
            instance: self.config.device.instance,
            name: self.config.device.name.clone(),
            vendor_id: self.config.device.vendor_id,
            ..BacnetDeviceConfig::default()
        })?;
        db.add(Box::new(device))?;

        // Determine BIP settings (required for now).
        let bip = self.config.transports.bip.as_ref().ok_or_else(|| {
            Error::Encoding("[transports.bip] is required for the gateway".to_string())
        })?;

        let interface: Ipv4Addr = bip
            .interface
            .parse()
            .map_err(|e| Error::Encoding(format!("invalid BIP interface: {e}")))?;
        let broadcast: Ipv4Addr = bip
            .broadcast
            .parse()
            .map_err(|e| Error::Encoding(format!("invalid BIP broadcast: {e}")))?;

        // Start server on the configured BIP port.
        let server = BACnetServer::bip_builder()
            .interface(interface)
            .port(bip.port)
            .broadcast_address(broadcast)
            .database(db)
            .build()
            .await?;

        let server_mac = server.local_mac().to_vec();
        let db = server.database().clone();

        // Start client on an ephemeral port.
        let client = BACnetClient::bip_builder()
            .interface(interface)
            .port(0)
            .broadcast_address(broadcast)
            .apdu_timeout_ms(6000)
            .build()
            .await?;

        let state = GatewayState::new_with_stack(db, Arc::new(self.config), client);

        Ok(BuiltGateway {
            state,
            server_mac,
            server,
        })
    }
}

/// A fully constructed gateway with its state and metadata.
pub struct BuiltGateway {
    /// Shared state for API/MCP handlers.
    pub state: GatewayState,
    /// The server's BIP MAC address.
    pub server_mac: Vec<u8>,
    /// The BACnet server (kept alive; drop to stop).
    pub server: BACnetServer<BipTransport>,
}
