//! MCP (Model Context Protocol) server implementation.
//!
//! Exposes BACnet operations as MCP tools and network state as MCP resources.

pub mod discovery;
pub mod objects;
pub mod properties;
pub mod reference;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::ErrorData as McpError;
use rmcp::model::ResourceContents;
use rmcp::model::{
    ListResourceTemplatesResult, ListResourcesResult, PaginatedRequestParams,
    ReadResourceRequestParams, ReadResourceResult, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_handler, tool_router, RoleServer, ServerHandler};

use crate::state::GatewayState;

/// MCP server handler for the BACnet gateway.
#[derive(Clone)]
pub struct GatewayMcp {
    pub state: GatewayState,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl GatewayMcp {
    pub fn new(state: GatewayState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }

    // --- Discovery tools ---

    #[tool(
        description = "Manually register a remote BACnet device by instance and IP:port address, without requiring WhoIs/IAm discovery."
    )]
    async fn register_device(
        &self,
        params: Parameters<discovery::RegisterDeviceParams>,
    ) -> Result<String, String> {
        discovery::register_device_impl(&self.state, params.0).await
    }

    #[tool(
        description = "Discover BACnet devices on the network by sending a WhoIs broadcast. Returns a list of devices that respond with IAm."
    )]
    async fn discover_devices(
        &self,
        params: Parameters<discovery::DiscoverParams>,
    ) -> Result<String, String> {
        discovery::discover_devices_impl(&self.state, params.0).await
    }

    #[tool(
        description = "List all previously discovered BACnet devices from the device table. No network traffic is generated."
    )]
    async fn list_known_devices(&self) -> Result<String, String> {
        discovery::list_known_devices_impl(&self.state).await
    }

    #[tool(
        description = "Get detailed information about a specific BACnet device by reading its Device object properties (name, vendor, model, firmware, etc.)."
    )]
    async fn get_device_info(
        &self,
        params: Parameters<discovery::DeviceInfoParams>,
    ) -> Result<String, String> {
        discovery::get_device_info_impl(&self.state, params.0).await
    }

    // --- Property tools ---

    #[tool(
        description = "Read a property from a remote BACnet device. Specify the device instance, object type and instance, and property name."
    )]
    async fn read_property(
        &self,
        params: Parameters<properties::ReadPropertyParams>,
    ) -> Result<String, String> {
        properties::read_property_impl(&self.state, params.0).await
    }

    #[tool(
        description = "Write a value to a property on a remote BACnet device. Specify the device, object, property, value, and optionally a command priority (1-16)."
    )]
    async fn write_property(
        &self,
        params: Parameters<properties::WritePropertyParams>,
    ) -> Result<String, String> {
        properties::write_property_impl(&self.state, params.0).await
    }

    // --- Local object tools ---

    #[tool(
        description = "List objects in the gateway's local BACnet object database. Optionally filter by object type."
    )]
    async fn list_local_objects(
        &self,
        params: Parameters<objects::ListObjectsParams>,
    ) -> Result<String, String> {
        objects::list_local_objects_impl(&self.state, params.0).await
    }

    #[tool(
        description = "Read a property from the gateway's local object database. No network traffic."
    )]
    async fn read_local_property(
        &self,
        params: Parameters<objects::ReadLocalPropertyParams>,
    ) -> Result<String, String> {
        objects::read_local_property_impl(&self.state, params.0).await
    }

    #[tool(
        description = "Write a value to a property in the gateway's local object database. No network traffic."
    )]
    async fn write_local_property(
        &self,
        params: Parameters<objects::WriteLocalPropertyParams>,
    ) -> Result<String, String> {
        objects::write_local_property_impl(&self.state, params.0).await
    }

    #[tool(
        description = "Create a new object in the gateway's local BACnet database. Supports analog, binary, multi-state, and value types."
    )]
    async fn create_local_object(
        &self,
        params: Parameters<objects::CreateLocalObjectParams>,
    ) -> Result<String, String> {
        objects::create_local_object_impl(&self.state, params.0).await
    }

    #[tool(
        description = "Delete an object from the gateway's local BACnet database. Cannot delete the Device object."
    )]
    async fn delete_local_object(
        &self,
        params: Parameters<objects::DeleteLocalObjectParams>,
    ) -> Result<String, String> {
        objects::delete_local_object_impl(&self.state, params.0).await
    }
}

impl GatewayMcp {
    async fn read_state_resource(&self, uri: &str) -> Option<String> {
        match uri {
            "bacnet://state/devices" => {
                let text = match self.state.require_client() {
                    Ok(client) => {
                        let devices = client.discovered_devices().await;
                        if devices.is_empty() {
                            "No discovered devices.".to_string()
                        } else {
                            let mut result = format!("{} discovered device(s):\n", devices.len());
                            for dev in &devices {
                                result.push_str(&format!(
                                    "  Instance {}, vendor {}, MAC {:02x?}\n",
                                    dev.object_identifier.instance_number(),
                                    dev.vendor_id,
                                    dev.mac_address.as_slice(),
                                ));
                            }
                            result
                        }
                    }
                    Err(_) => "No devices (client not started).".to_string(),
                };
                Some(text)
            }
            "bacnet://state/local-objects" => {
                let db = self.state.db.read().await;
                let mut result = format!("{} local object(s):\n", db.len());
                for (oid, obj) in db.iter_objects() {
                    result.push_str(&format!(
                        "  {}:{} \"{}\"\n",
                        crate::parse::object_type_name(oid.object_type()),
                        oid.instance_number(),
                        obj.object_name(),
                    ));
                }
                Some(result)
            }
            "bacnet://state/config" => {
                let config = &self.state.config;
                let mut result = String::new();
                result.push_str(&format!(
                    "Device: {} (instance {})\n",
                    config.device.name, config.device.instance
                ));
                result.push_str(&format!("Server bind: {}\n", config.server.bind));
                result.push_str(&format!(
                    "Auth: {}\n",
                    if config.server.api_key.is_some() {
                        "enabled (bearer token)"
                    } else {
                        "disabled"
                    }
                ));
                if let Some(bip) = &config.transports.bip {
                    result.push_str(&format!(
                        "Transport BIP: {}:{}, network {}\n",
                        bip.interface, bip.port, bip.network_number
                    ));
                }
                if let Some(sc) = &config.transports.sc {
                    result.push_str(&format!(
                        "Transport SC: {}, network {}\n",
                        sc.hub_uri, sc.network_number
                    ));
                }
                if let Some(mstp) = &config.transports.mstp {
                    result.push_str(&format!(
                        "Transport MS/TP: {}, station {}, network {}\n",
                        mstp.serial_port, mstp.station_address, mstp.network_number
                    ));
                }
                Some(result)
            }
            _ => None,
        }
    }
}

#[tool_handler]
impl ServerHandler for GatewayMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_instructions(
            "BACnet gateway MCP server. Use tools to discover devices, read/write properties, \
             and manage the local object database. Read reference resources \
             (bacnet://reference/*) to learn about BACnet object types, properties, \
             networking, and troubleshooting."
                .to_string(),
        )
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let mut resources = reference::reference_resources();
        resources.extend(reference::state_resources());
        std::future::ready(Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        }))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        let templates = reference::reference_templates();
        std::future::ready(Ok(ListResourceTemplatesResult {
            resource_templates: templates,
            next_cursor: None,
            meta: None,
        }))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let uri = request.uri;
        async move {
            // Try static reference resources first.
            if let Some(text) = reference::read_reference(&uri) {
                return Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    text,
                    uri.clone(),
                )]));
            }

            // Try live state resources (async — may read from client/db).
            if let Some(text) = self.read_state_resource(&uri).await {
                return Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    text,
                    uri.clone(),
                )]));
            }

            Err(McpError::resource_not_found(
                "resource not found",
                Some(serde_json::json!({ "uri": uri })),
            ))
        }
    }
}
