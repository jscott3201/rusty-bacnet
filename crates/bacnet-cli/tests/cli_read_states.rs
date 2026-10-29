//! `bacnet read` and `readm` naming binary and multi-state states with
//! their objects' state texts (#2132): what they print, and what the texts
//! cost, counted by a relay between the CLI and the lab device that keeps
//! each confirmed request it passes on and can stop passing them on.
#[allow(dead_code)]
mod support;

use std::borrow::Cow;
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::process::Output;
use std::sync::{Arc, Mutex};

use bacnet_encoding::apdu::{Apdu, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_objects::binary::BinaryValueObject;
use bacnet_objects::multistate::MultiStateValueObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_server::server::BACnetServer;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::rpm::ReadPropertyMultipleRequest;
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::Bytes;
use serde_json::{Value, json};
use support::lab::{lab_database, lab_identity};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// A binary or multi-state object that has a state but refuses every read
/// of its state texts.
struct Textless {
    oid: ObjectIdentifier,
    name: String,
    state: PropertyValue,
}

impl BACnetObject for Textless {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            PropertyIdentifier::OBJECT_IDENTIFIER => Ok(PropertyValue::ObjectIdentifier(self.oid)),
            PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(self.oid.object_type().to_raw()))
            }
            PropertyIdentifier::PRESENT_VALUE => Ok(self.state.clone()),
            _ => Err(Error::protocol(
                ErrorClass::PROPERTY,
                ErrorCode::UNKNOWN_PROPERTY,
            )),
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
        ])
    }
}

/// Multi-State Value 2 at state 2, its State_Text `Off`, `Low` (behind a
/// right-to-left override) and `High`; Binary Value 2, inactive, its texts
/// `Running` and `Stopped`; and Multi-State Value 3 at state 2 and Binary
/// Value 3 active, which refuse their texts.
fn state_objects() -> Vec<Box<dyn BACnetObject>> {
    let text = |text: &str| PropertyValue::CharacterString(text.into());
    let mut speed = MultiStateValueObject::new(2, "Fan Speed", 3).unwrap();
    speed
        .write_property(
            PropertyIdentifier::STATE_TEXT,
            None,
            PropertyValue::List(vec![text("Off"), text("\u{202E}Low"), text("High")]),
            None,
        )
        .unwrap();
    speed.set_relinquish_default(2).unwrap();
    let mut fan = BinaryValueObject::new(2, "Fan").unwrap();
    for (property, value) in [
        (PropertyIdentifier::ACTIVE_TEXT, "Running"),
        (PropertyIdentifier::INACTIVE_TEXT, "Stopped"),
    ] {
        fan.write_property(property, None, text(value), None)
            .unwrap();
    }
    let textless = |object_type, state| {
        Box::new(Textless {
            oid: ObjectIdentifier::new(object_type, 3).unwrap(),
            name: format!("Textless {object_type}"),
            state,
        }) as Box<dyn BACnetObject>
    };
    vec![
        Box::new(speed),
        Box::new(fan),
        textless(ObjectType::MULTI_STATE_VALUE, PropertyValue::Unsigned(2)),
        textless(ObjectType::BINARY_VALUE, PropertyValue::Enumerated(1)),
    ]
}

/// The lab device with the [`state_objects`], and its address.
async fn server() -> (BACnetServer<BipTransport>, SocketAddr) {
    let mut db = lab_database(1234);
    for object in state_objects() {
        db.add(object).unwrap();
    }
    let server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .device(lab_identity(1234))
        .database(db)
        .build()
        .await
        .unwrap();
    let mac = server.local_mac();
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, u16::from_be_bytes([mac[4], mac[5]])));
    (server, address)
}

/// A confirmed request the relay saw: its service and the properties it
/// names. A retry, with the same invoke ID, is kept once.
type Asked = Arc<Mutex<Vec<(ConfirmedServiceChoice, Vec<PropertyIdentifier>)>>>;

/// A B/IP relay on loopback in front of `device`: it passes the first
/// `answered` confirmed requests on (and their retries) and drops the
/// rest, so the device goes silent after them; replies go back to the
/// CLI. Returns the CLI's target, what was asked, and the relay's task.
async fn relay(device: SocketAddr, answered: usize) -> (String, Asked, JoinHandle<()>) {
    let front = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let back = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let target = front.local_addr().unwrap().to_string();
    let asked: Asked = Arc::default();
    let log = Arc::clone(&asked);
    let task = tokio::spawn(async move {
        let (mut seen, mut passed) = (HashSet::new(), HashSet::new());
        let mut cli = None;
        let (mut up, mut down) = ([0u8; 1500], [0u8; 1500]);
        loop {
            tokio::select! {
                received = front.recv_from(&mut up) => {
                    let Ok((length, from)) = received else { return };
                    cli = Some(from);
                    let frame = &up[..length];
                    if let Some((invoke_id, service, properties)) = confirmed(frame) {
                        if seen.insert(invoke_id) {
                            log.lock().unwrap().push((service, properties));
                        }
                        if !passed.contains(&invoke_id) {
                            if passed.len() == answered {
                                continue;
                            }
                            passed.insert(invoke_id);
                        }
                    }
                    let _ = back.send_to(frame, device).await;
                }
                received = back.recv_from(&mut down) => {
                    let Ok((length, _)) = received else { return };
                    if let Some(cli) = cli {
                        let _ = front.send_to(&down[..length], cli).await;
                    }
                }
            }
        }
    });
    (target, asked, task)
}

/// A B/IP frame's confirmed request: its invoke ID, service, and the
/// properties a ReadProperty or ReadPropertyMultiple names.
fn confirmed(frame: &[u8]) -> Option<(u8, ConfirmedServiceChoice, Vec<PropertyIdentifier>)> {
    let npdu = decode_npdu(Bytes::copy_from_slice(frame.get(4..)?)).ok()?;
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload) else {
        return None;
    };
    let properties = match request.service_choice {
        ConfirmedServiceChoice::READ_PROPERTY => {
            ReadPropertyRequest::decode(&request.service_request)
                .map(|read| vec![read.property_identifier])
                .unwrap_or_default()
        }
        ConfirmedServiceChoice::READ_PROPERTY_MULTIPLE => {
            ReadPropertyMultipleRequest::decode(&request.service_request)
                .map(|read| {
                    read.list_of_read_access_specs
                        .iter()
                        .flat_map(|spec| &spec.list_of_property_references)
                        .map(|reference| reference.property_identifier)
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    };
    Some((request.invoke_id, request.service_choice, properties))
}

/// The CLI with a short APDU timeout, so a silent device fails fast.
async fn run(target: &str, format: &str, args: &[&str]) -> Output {
    let mut all = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--timeout",
        "300",
        "--format",
        format,
    ];
    all.push(args[0]);
    all.push(target);
    all.extend(&args[1..]);
    support::run(all).await
}

async fn cli(target: &str, format: &str, args: &[&str]) -> String {
    let output = run(target, format, args).await;
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// `read` of one property, as JSON: its text value and its typed form.
async fn read(target: &str, object: &str, property: &str) -> (String, Value) {
    let json: Value =
        serde_json::from_str(&cli(target, "json", &["read", object, property]).await).unwrap();
    (
        json["value"].as_str().unwrap().to_string(),
        json["decoded"].clone(),
    )
}

/// Each `readm` result's text value.
fn values(stdout: &[u8]) -> Vec<String> {
    let json: Value = serde_json::from_slice(stdout).unwrap();
    json.as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["value"].as_str().unwrap().to_string())
        .collect()
}

/// A multi-state and a binary value's states print with their objects'
/// texts, cleaned, in JSON beside the unchanged typed form and in the
/// table; an object that refuses its texts prints today's plain states,
/// and its refusal doesn't stop the next object's texts. An array's size
/// is never a state.
#[tokio::test]
async fn states_print_with_their_objects_state_texts() {
    let (mut server, device) = server().await;
    let target = device.to_string();
    for (object, property, text, decoded) in [
        ("msv:2", "present-value", "2 (Low)", json!(2)),
        ("msv:2", "relinquish-default", "2 (Low)", json!(2)),
        ("msv:2", "priority-array[0]", "16", json!(16)),
        (
            "bv:2",
            "present-value",
            "inactive (Stopped)",
            json!({"value": 0, "name": "inactive"}),
        ),
        // Refused: the plain state.
        ("msv:3", "present-value", "2", json!(2)),
        (
            "bv:3",
            "present-value",
            "active",
            json!({"value": 1, "name": "active"}),
        ),
    ] {
        let (value, data) = read(&target, object, property).await;
        assert_eq!(value, text, "{object} {property}");
        assert_eq!(data, decoded, "{object} {property}");
    }
    let table = cli(&target, "table", &["read", "msv:2", "pv"]).await;
    assert!(table.contains("2 (Low)"), "{table}");

    let readm = [
        "readm",
        "msv:3",
        "pv",
        "msv:2",
        "pv,state-text",
        "bv:2",
        "pv",
    ];
    let printed = values(cli(&target, "json", &readm).await.as_bytes());
    assert_eq!(printed[..2], ["2", "2 (Low)"], "{printed:?}");
    assert_eq!(printed[3], "inactive (Stopped)", "{printed:?}");

    let all: Value =
        serde_json::from_str(&cli(&target, "json", &["read", "bv:2", "all"]).await).unwrap();
    let present = all
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["property"] == "PRESENT_VALUE")
        .unwrap_or_else(|| panic!("{all}"));
    assert_eq!(present["value"], "inactive (Stopped)", "{all}");
    server.stop().await.unwrap();
}

/// The texts cost one request an object that needs them, after the read,
/// and none when the read already holds them or holds no state.
#[tokio::test]
async fn texts_cost_one_request_an_object_or_none() {
    let (mut server, device) = server().await;
    let cases: [(&[&str], usize); 6] = [
        (&["read", "msv:2", "pv"], 2),
        (&["read", "bv:2", "pv"], 2),
        (&["read", "bv:2", "all"], 1),
        (&["readm", "msv:2", "pv,state-text"], 1),
        // Two of its states, one text read.
        (&["readm", "msv:2", "pv,relinquish-default"], 2),
        (&["read", "av:1", "pv"], 1),
    ];
    for (args, requests) in cases {
        let (target, asked, task) = relay(device, usize::MAX).await;
        cli(&target, "json", args).await;
        let asked = asked.lock().unwrap().clone();
        assert_eq!(asked.len(), requests, "{args:?}: {asked:?}");
        task.abort();
    }
    server.stop().await.unwrap();
}

/// A device that goes silent after answering the read: the first text
/// read times out, no other is sent, and the command succeeds with the
/// plain states and the same stderr as a device without texts would give.
#[tokio::test]
async fn a_silent_device_costs_one_text_timeout() {
    let (mut server, device) = server().await;
    let readm = ["readm", "msv:2", "pv", "bv:2", "pv"];
    let (target, asked, task) = relay(device, 1).await;
    let output = run(&target, "json", &readm).await;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(values(&output.stdout), ["2", "inactive"]);
    let plain = run(&device.to_string(), "json", &["readm", "msv:3", "pv"]).await;
    assert_eq!(output.stderr, plain.stderr);
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2, "the read and one text read: {asked:?}");
    assert_eq!(
        asked[1],
        (
            ConfirmedServiceChoice::READ_PROPERTY,
            vec![PropertyIdentifier::STATE_TEXT]
        )
    );
    task.abort();
    server.stop().await.unwrap();
}
