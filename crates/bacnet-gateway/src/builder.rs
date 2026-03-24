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
use crate::parse::{construct_object, parse_object_type};
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

        // Pre-populate local objects from config.
        for obj_cfg in &self.config.objects {
            let obj_type = parse_object_type(&obj_cfg.object_type)
                .map_err(|e| Error::Encoding(format!("object config: {e}")))?;
            let obj = construct_object(
                obj_type,
                obj_cfg.instance,
                &obj_cfg.name,
                obj_cfg.number_of_states,
            )
            .map_err(|e| Error::Encoding(format!("object config: {e}")))?;
            db.add(obj)
                .map_err(|e| Error::Encoding(format!("object config: {e}")))?;
            tracing::info!(
                "Pre-populated object {}:{} ({})",
                obj_cfg.object_type,
                obj_cfg.instance,
                obj_cfg.name
            );
        }

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

        // Parse foreign device config once (used for both server and client transports).
        let fd_config = if let Some(fd) = &self.config.foreign_device {
            let addr: std::net::SocketAddrV4 = fd.bbmd.parse().map_err(|e| {
                Error::Encoding(format!("invalid foreign_device.bbmd '{}': {e}", fd.bbmd))
            })?;
            Some(bacnet_transport::bip::ForeignDeviceConfig {
                bbmd_ip: *addr.ip(),
                bbmd_port: addr.port(),
                ttl: fd.ttl,
            })
        } else {
            None
        };

        // Create and configure BIP transport for the server.
        let mut server_transport = BipTransport::new(interface, bip.port, broadcast);

        if let Some(ref fdc) = fd_config {
            server_transport.register_as_foreign_device(fdc.clone());
            tracing::info!(
                "Registered as foreign device with BBMD {}:{}",
                fdc.bbmd_ip,
                fdc.bbmd_port
            );
        }

        if let Some(bbmd_cfg) = &self.config.bbmd {
            if bbmd_cfg.enabled {
                let mut bdt_entries = Vec::new();
                for entry_str in &bbmd_cfg.bdt {
                    let addr: std::net::SocketAddrV4 = entry_str.parse().map_err(|e| {
                        Error::Encoding(format!("invalid BDT entry '{entry_str}': {e}"))
                    })?;
                    bdt_entries.push(bacnet_transport::bbmd::BdtEntry {
                        ip: addr.ip().octets(),
                        port: addr.port(),
                        broadcast_mask: [0xff, 0xff, 0xff, 0xff],
                    });
                }
                server_transport.enable_bbmd(bdt_entries);
                tracing::info!("BBMD enabled with {} BDT entries", bbmd_cfg.bdt.len());
            }
        }

        // Start server with the pre-configured transport.
        let server = BACnetServer::generic_builder()
            .transport(server_transport)
            .database(db)
            .vendor_id(self.config.device.vendor_id)
            .build()
            .await?;

        let server_mac = server.local_mac().to_vec();
        let db = server.database().clone();

        // Create client transport (also needs foreign device config for broadcast routing).
        let mut client_transport = BipTransport::new(interface, 0, broadcast);
        if let Some(ref fdc) = fd_config {
            client_transport.register_as_foreign_device(fdc.clone());
        }

        let client = BACnetClient::generic_builder()
            .transport(client_transport)
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
