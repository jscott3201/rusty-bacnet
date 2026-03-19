//! Integration tests for the REST API.
//! Requires: `cargo test -p bacnet-gateway --features http`

#![cfg(feature = "http")]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use bacnet_gateway::api::api_router;
use bacnet_gateway::auth::bearer::BearerTokenAuth;
use bacnet_gateway::auth::Authenticator;
use bacnet_gateway::builder::GatewayBuilder;
use bacnet_gateway::config::{
    BipConfig, DeviceConfig, GatewayConfig, ServerConfig, TransportsConfig,
};
use bacnet_gateway::state::GatewayState;

use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig as BacnetDeviceConfig, DeviceObject};

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

fn test_state_with_objects() -> GatewayState {
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

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

// --- Health endpoint ---

#[tokio::test]
async fn health_returns_ok() {
    let state = test_state_with_objects();
    let app = api_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["status"], "ok");
}

// --- List objects ---

#[tokio::test]
async fn list_objects_returns_all() {
    let state = test_state_with_objects();
    let app = api_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    let objects = json["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 2); // Device + AnalogValue
}

#[tokio::test]
async fn list_objects_with_type_filter() {
    let state = test_state_with_objects();
    let app = api_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects?type=analog-value")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    let objects = json["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["type"], "analog-value");
}

// --- Get object ---

#[tokio::test]
async fn get_object_returns_properties() {
    let state = test_state_with_objects();
    let app = api_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects/analog-value:1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["name"], "Test AV");
    assert!(!json["properties"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_nonexistent_object_returns_404() {
    let state = test_state_with_objects();
    let app = api_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects/analog-input:999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// --- Read property ---

#[tokio::test]
async fn read_property_present_value() {
    let state = test_state_with_objects();
    let app = api_router(state, None);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects/analog-value:1/properties/present-value")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["property"], "present-value");
    assert!(json["value"].is_object());
}

// --- Write property ---

#[tokio::test]
async fn write_then_read_property() {
    let state = test_state_with_objects();
    let app = api_router(state.clone(), None);

    // Write
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/objects/analog-value:1/properties/present-value")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"value": 72.5}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Read back
    let app2 = api_router(state, None);
    let response = app2
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects/analog-value:1/properties/present-value")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    let val = json["value"]["value"].as_f64().unwrap();
    assert!((val - 72.5).abs() < 0.01);
}

// --- Create object ---

#[tokio::test]
async fn create_and_list_object() {
    let state = test_state_with_objects();
    let app = api_router(state.clone(), None);

    // Create
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/objects")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"type": "binary-value", "instance": 1, "name": "Test BV"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // List
    let app2 = api_router(state, None);
    let response = app2
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects?type=binary-value")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let json = body_json(response).await;
    let objects = json["objects"].as_array().unwrap();
    assert_eq!(objects.len(), 1);
    assert_eq!(objects[0]["name"], "Test BV");
}

// --- Delete object ---

#[tokio::test]
async fn delete_object_removes_it() {
    let state = test_state_with_objects();
    let app = api_router(state.clone(), None);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/objects/analog-value:1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Verify gone
    let app2 = api_router(state, None);
    let response = app2
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects/analog-value:1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// --- Auth ---

#[tokio::test]
async fn auth_rejects_missing_token() {
    let state = test_state_with_objects();
    let auth: Box<dyn Authenticator> = Box::new(BearerTokenAuth::new("secret"));
    let app = api_router(state, Some(auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_rejects_wrong_token() {
    let state = test_state_with_objects();
    let auth: Box<dyn Authenticator> = Box::new(BearerTokenAuth::new("secret"));
    let app = api_router(state, Some(auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects")
                .header("authorization", "Bearer wrong")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn auth_allows_correct_token() {
    let state = test_state_with_objects();
    let auth: Box<dyn Authenticator> = Box::new(BearerTokenAuth::new("secret"));
    let app = api_router(state, Some(auth));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn health_bypasses_auth() {
    let state = test_state_with_objects();
    let auth: Box<dyn Authenticator> = Box::new(BearerTokenAuth::new("secret"));
    let app = api_router(state, Some(auth));

    // Health should work without auth.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

// --- End-to-end with full BACnet stack ---

#[tokio::test]
async fn e2e_gateway_serves_device_object() {
    // Build a gateway with BIP on localhost, ephemeral port.
    let config = GatewayConfig {
        server: ServerConfig::default(),
        device: DeviceConfig {
            instance: 9999,
            name: "E2E Gateway".to_string(),
            vendor_id: 555,
            description: "End-to-end test".to_string(),
        },
        transports: TransportsConfig {
            bip: Some(BipConfig {
                interface: "127.0.0.1".to_string(),
                port: 0,
                broadcast: "127.0.0.1".to_string(),
                network_number: 1,
            }),
            ..TransportsConfig::default()
        },
        bbmd: None,
        foreign_device: None,
        routes: vec![],
        objects: vec![],
    };

    let built = GatewayBuilder::new(config).build().await.unwrap();
    let app = api_router(built.state.clone(), None);

    // Read the device object via REST.
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/objects/device:9999")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["name"], "E2E Gateway");

    // The server is in `built.server` — drop it to clean up.
    drop(built);
}
