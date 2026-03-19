//! MCP server integration tests.
//! Requires: `cargo test -p bacnet-gateway --features mcp`

#![cfg(feature = "mcp")]

use bacnet_gateway::config::{DeviceConfig, GatewayConfig, ServerConfig, TransportsConfig};
use bacnet_gateway::mcp::discovery;
use bacnet_gateway::mcp::objects;
use bacnet_gateway::mcp::reference;
use bacnet_gateway::mcp::GatewayMcp;
use bacnet_gateway::state::GatewayState;

use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig as BacnetDeviceConfig, DeviceObject};

use rmcp::ServerHandler;

fn test_config() -> GatewayConfig {
    GatewayConfig {
        server: ServerConfig::default(),
        device: DeviceConfig {
            instance: 1234,
            name: "Test Gateway".to_string(),
            vendor_id: 999,
            description: "Test".to_string(),
        },
        transports: TransportsConfig::default(),
        bbmd: None,
        foreign_device: None,
        routes: vec![],
        objects: vec![],
    }
}

fn test_state() -> GatewayState {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(BacnetDeviceConfig {
        instance: 1234,
        name: "Test Gateway".into(),
        vendor_id: 999,
        ..BacnetDeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();
    let av = AnalogValueObject::new(1, "Test AV", 95).unwrap();
    db.add(Box::new(av)).unwrap();
    GatewayState::new(db, test_config())
}

#[test]
fn mcp_server_has_tool_capabilities() {
    let state = test_state();
    let mcp = GatewayMcp::new(state);
    let info = mcp.get_info();
    assert!(info.capabilities.tools.is_some());
}

#[tokio::test]
async fn mcp_list_local_objects() {
    let state = test_state();
    let result =
        objects::list_local_objects_impl(&state, objects::ListObjectsParams { object_type: None })
            .await;

    assert!(result.contains("2 local object(s)"));
    assert!(result.contains("device:1234"));
    assert!(result.contains("analog-value:1"));
}

#[tokio::test]
async fn mcp_list_local_objects_filtered() {
    let state = test_state();
    let result = objects::list_local_objects_impl(
        &state,
        objects::ListObjectsParams {
            object_type: Some("analog-value".to_string()),
        },
    )
    .await;

    assert!(result.contains("1 local object(s)"));
    assert!(result.contains("analog-value:1"));
    assert!(!result.contains("device:1234"));
}

#[tokio::test]
async fn mcp_read_local_property() {
    let state = test_state();
    let result = objects::read_local_property_impl(
        &state,
        objects::ReadLocalPropertyParams {
            object_type: "analog-value".to_string(),
            object_instance: 1,
            property: "object-name".to_string(),
        },
    )
    .await;

    assert!(result.contains("Test AV"));
}

#[tokio::test]
async fn mcp_write_and_read_local_property() {
    let state = test_state();

    let result = objects::write_local_property_impl(
        &state,
        objects::WriteLocalPropertyParams {
            object_type: "analog-value".to_string(),
            object_instance: 1,
            property: "present-value".to_string(),
            value: serde_json::json!(42.0),
        },
    )
    .await;

    assert!(result.contains("Successfully wrote"));

    let result = objects::read_local_property_impl(
        &state,
        objects::ReadLocalPropertyParams {
            object_type: "analog-value".to_string(),
            object_instance: 1,
            property: "present-value".to_string(),
        },
    )
    .await;

    assert!(result.contains("42"));
}

#[tokio::test]
async fn mcp_read_nonexistent_object() {
    let state = test_state();
    let result = objects::read_local_property_impl(
        &state,
        objects::ReadLocalPropertyParams {
            object_type: "analog-input".to_string(),
            object_instance: 999,
            property: "present-value".to_string(),
        },
    )
    .await;

    assert!(result.contains("not found"));
}

#[tokio::test]
async fn mcp_list_known_devices_no_client() {
    let state = test_state();
    let result = discovery::list_known_devices_impl(&state).await;
    assert!(result.contains("not started"));
}

#[tokio::test]
async fn mcp_discover_devices_no_client() {
    let state = test_state();
    let result = discovery::discover_devices_impl(
        &state,
        discovery::DiscoverParams {
            low_instance: None,
            high_instance: None,
            timeout_seconds: Some(1),
        },
    )
    .await;

    assert!(result.contains("not started"));
}

// --- Create/Delete local objects ---

#[tokio::test]
async fn mcp_create_local_object() {
    let state = test_state();
    let result = objects::create_local_object_impl(
        &state,
        objects::CreateLocalObjectParams {
            object_type: "multi-state-value".to_string(),
            object_instance: 1,
            object_name: "Test MSV".to_string(),
            number_of_states: Some(4),
        },
    )
    .await;

    assert!(result.contains("Created"));
    assert!(result.contains("multi-state-value:1"));

    // Verify it exists.
    let list = objects::list_local_objects_impl(
        &state,
        objects::ListObjectsParams {
            object_type: Some("multi-state-value".to_string()),
        },
    )
    .await;
    assert!(list.contains("Test MSV"));
}

#[tokio::test]
async fn mcp_create_integer_value() {
    let state = test_state();
    let result = objects::create_local_object_impl(
        &state,
        objects::CreateLocalObjectParams {
            object_type: "integer-value".to_string(),
            object_instance: 1,
            object_name: "Test IV".to_string(),
            number_of_states: None,
        },
    )
    .await;

    assert!(result.contains("Created"));
}

#[tokio::test]
async fn mcp_delete_local_object() {
    let state = test_state();
    let result = objects::delete_local_object_impl(
        &state,
        objects::DeleteLocalObjectParams {
            object_type: "analog-value".to_string(),
            object_instance: 1,
        },
    )
    .await;

    assert!(result.contains("Deleted"));

    // Verify it's gone.
    let read = objects::read_local_property_impl(
        &state,
        objects::ReadLocalPropertyParams {
            object_type: "analog-value".to_string(),
            object_instance: 1,
            property: "present-value".to_string(),
        },
    )
    .await;
    assert!(read.contains("not found"));
}

#[tokio::test]
async fn mcp_delete_device_object_rejected() {
    let state = test_state();
    let result = objects::delete_local_object_impl(
        &state,
        objects::DeleteLocalObjectParams {
            object_type: "device".to_string(),
            object_instance: 1234,
        },
    )
    .await;

    assert!(result.contains("cannot delete"));
}

// --- Reference resources ---

#[test]
fn reference_resources_list() {
    let resources = reference::reference_resources();
    assert_eq!(resources.len(), 9);
    for r in &resources {
        assert!(
            r.uri.starts_with("bacnet://reference/"),
            "bad URI: {}",
            r.uri
        );
    }
}

#[test]
fn state_resources_list() {
    let resources = reference::state_resources();
    assert_eq!(resources.len(), 3);
    for r in &resources {
        assert!(r.uri.starts_with("bacnet://state/"), "bad URI: {}", r.uri);
    }
}

#[test]
fn reference_templates_list() {
    let templates = reference::reference_templates();
    assert_eq!(templates.len(), 1);
    assert!(templates[0].uri_template.contains("{type}"));
}

#[test]
fn reference_read_object_types_index() {
    let content = reference::read_reference("bacnet://reference/object-types").unwrap();
    assert!(content.contains("analog-input"));
    assert!(content.contains("device"));
    assert!(content.contains("binary-value"));
}

#[test]
fn reference_read_properties() {
    let content = reference::read_reference("bacnet://reference/properties").unwrap();
    assert!(content.contains("present-value"));
    assert!(content.contains("status-flags"));
}

#[test]
fn reference_read_networking() {
    let content = reference::read_reference("bacnet://reference/networking").unwrap();
    assert!(content.contains("BBMD"));
    assert!(content.contains("router"));
}

#[test]
fn reference_read_object_type_detail_analog_input() {
    let content =
        reference::read_reference("bacnet://reference/object-types/analog-input").unwrap();
    assert!(content.contains("Sensor"));
    assert!(content.contains("present-value"));
    assert!(content.contains("cov-increment"));
}

#[test]
fn reference_read_object_type_detail_device() {
    let content = reference::read_reference("bacnet://reference/object-types/device").unwrap();
    assert!(content.contains("vendor-name"));
    assert!(content.contains("object-list"));
}

#[test]
fn reference_read_unknown_returns_none() {
    assert!(reference::read_reference("bacnet://reference/nonexistent").is_none());
}

#[test]
fn reference_server_has_resource_capabilities() {
    let state = test_state();
    let mcp = GatewayMcp::new(state);
    let info = mcp.get_info();
    assert!(info.capabilities.resources.is_some());
}
