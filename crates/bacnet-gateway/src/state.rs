//! Gateway shared state.

use std::sync::Arc;
use tokio::sync::RwLock;

use bacnet_client::client::BACnetClient;
use bacnet_client::discovery::DiscoveredDevice;
use bacnet_objects::database::ObjectDatabase;
use bacnet_transport::bip::BipTransport;

use crate::config::GatewayConfig;

/// Shared state for the gateway, accessible by both REST API and MCP handlers.
///
/// This is cheaply cloneable (all fields are Arc-wrapped) and is passed as
/// Axum state and cloned into MCP tool handlers.
#[derive(Clone)]
pub struct GatewayState {
    /// Local BACnet object database (shared with the server).
    pub db: Arc<RwLock<ObjectDatabase>>,
    /// Gateway configuration.
    pub config: Arc<GatewayConfig>,
    /// BACnet client for remote device operations (None in test-only mode).
    client: Option<Arc<BACnetClient<BipTransport>>>,
}

impl GatewayState {
    /// Create a minimal GatewayState (for tests without BACnet stack).
    pub fn new(db: ObjectDatabase, config: GatewayConfig) -> Self {
        Self {
            db: Arc::new(RwLock::new(db)),
            config: Arc::new(config),
            client: None,
        }
    }

    /// Create a GatewayState with the full BACnet stack.
    pub fn new_with_stack(
        db: Arc<RwLock<ObjectDatabase>>,
        config: Arc<GatewayConfig>,
        client: BACnetClient<BipTransport>,
    ) -> Self {
        Self {
            db,
            config,
            client: Some(Arc::new(client)),
        }
    }

    /// Get a reference to the BACnet client (if started).
    pub fn client(&self) -> Option<&BACnetClient<BipTransport>> {
        self.client.as_deref()
    }

    /// Get the BACnet client, returning an error message if not started.
    pub fn require_client(&self) -> Result<&BACnetClient<BipTransport>, String> {
        self.client()
            .ok_or_else(|| "BACnet client not started".to_string())
    }

    /// Resolve a device instance number to a DiscoveredDevice entry.
    pub async fn resolve_device(&self, instance: u32) -> Result<DiscoveredDevice, String> {
        let client = self.require_client()?;
        client
            .get_device(instance)
            .await
            .ok_or_else(|| format!("Device {instance} not found. Use discover_devices first."))
    }

    /// Check if the gateway is in read-only mode.
    pub fn is_read_only(&self) -> bool {
        self.config.server.read_only
    }

    /// Return an error if write operations are disabled.
    pub fn require_writable(&self) -> Result<(), String> {
        if self.config.server.read_only {
            Err("Gateway is in read-only mode. Write operations are disabled.".to_string())
        } else {
            Ok(())
        }
    }
}
