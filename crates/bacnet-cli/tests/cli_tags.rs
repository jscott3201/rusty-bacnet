//! Real CLI processes reading provisioned Tags over loopback.
#[allow(dead_code)]
mod support;

use bacnet_objects::color::ColorObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::object_profile::ObjectProfile;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BipTransport;
use bacnet_types::constructed::BACnetNameValue;
use bacnet_types::primitives::PropertyValue;
use serde_json::json;
use std::net::Ipv4Addr;

async fn server() -> BACnetServer<BipTransport> {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            instance: 1,
            name: "tags-reader".into(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    for (instance, name, tags) in [
        (
            1,
            "color",
            vec![
                BACnetNameValue::semantic("exhaust"),
                BACnetNameValue::valued("floor", PropertyValue::Unsigned(3)),
            ],
        ),
        (2, "empty", vec![]),
    ] {
        let mut object = ColorObject::new(instance, name).unwrap();
        object
            .set_profile(ObjectProfile {
                tags: Some(tags),
                ..ObjectProfile::default()
            })
            .unwrap();
        db.add(Box::new(object)).unwrap();
    }
    BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .build()
        .await
        .unwrap()
}

fn target(server: &BACnetServer<BipTransport>) -> String {
    let mac = server.local_mac();
    format!("127.0.0.1:{}", u16::from_be_bytes([mac[4], mac[5]]))
}

async fn read(target: &str, format: &str, command: &str, object: &str, properties: &str) -> String {
    let output = support::run([
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--format",
        format,
        command,
        target,
        object,
        properties,
    ])
    .await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
async fn cli_tags_whole_read_has_named_values_in_existing_json_schema() {
    let mut server = server().await;
    let output = read(&target(&server), "json", "read", "color:1", "tags").await;
    server.stop().await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(
        value,
        json!({"object": "COLOR:1", "property": "TAGS", "value": "[\"exhaust\", \"floor\"=3]"})
    );
}

#[tokio::test]
async fn cli_tags_rp_and_rpm_table_json_preserve_shapes_and_property_errors() {
    let mut server = server().await;
    let address = target(&server);
    let mut outputs = Vec::new();
    for (object, properties) in [
        ("color:1", "tags"),
        ("color:1", "tags[1]"),
        ("color:1", "tags[2]"),
        ("color:1", "tags[0]"),
        ("color:2", "tags"),
        ("color:2", "tags[0]"),
    ] {
        for format in ["json", "table"] {
            for command in ["read", "readm"] {
                outputs.push((
                    object,
                    properties,
                    format,
                    command,
                    read(&address, format, command, object, properties).await,
                ));
            }
        }
    }
    let mixed = read(
        &address,
        "json",
        "readm",
        "color:1",
        "tags,tags[1],tags[0],object-name,profile-name",
    )
    .await;
    let mixed_table = read(
        &address,
        "table",
        "readm",
        "color:1",
        "tags,object-name,profile-name",
    )
    .await;
    server.stop().await.unwrap();
    for (object, property, format, command, output) in outputs {
        let expected = match (object, property) {
            ("color:1", "tags") => "[\"exhaust\", \"floor\"=3]",
            (_, "tags[1]") => "\"exhaust\"",
            (_, "tags[2]") => "\"floor\"=3",
            ("color:1", "tags[0]") => "2",
            (_, "tags[0]") => "0",
            _ => "[]",
        };
        if format == "json" {
            let mut value: serde_json::Value = serde_json::from_str(&output).unwrap();
            if command == "readm" {
                value = value.as_array().unwrap()[0].clone();
            }
            assert_eq!(
                value,
                json!({"object": object.to_uppercase(), "property": property.to_uppercase(), "value": expected})
            );
        } else {
            assert!(output.contains(expected), "{output}");
            assert!(output.contains(&property.to_uppercase()), "{output}");
            assert!(!output.contains("raw:"), "{output}");
        }
    }
    let mixed: serde_json::Value = serde_json::from_str(&mixed).unwrap();
    assert_eq!(
        mixed,
        json!([
            {"object":"COLOR:1","property":"TAGS","value":"[\"exhaust\", \"floor\"=3]"},
            {"object":"COLOR:1","property":"TAGS[1]","value":"\"exhaust\""},
            {"object":"COLOR:1","property":"TAGS[0]","value":"2"},
            {"object":"COLOR:1","property":"OBJECT_NAME","value":"\"color\""},
            {"object":"COLOR:1","property":"PROFILE_NAME","value":"ERROR: PROPERTY:UNKNOWN_PROPERTY"}
        ])
    );
    assert!(
        mixed_table.contains("ERROR: PROPERTY:UNKNOWN_PROPERTY"),
        "{mixed_table}"
    );
    assert!(mixed_table.contains("\"color\""), "{mixed_table}");
}
