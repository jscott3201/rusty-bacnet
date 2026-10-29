//! Run the server guide's device program (#1989) and talk to it as a peer:
//! read the input the application supplies, command the output, and check
//! the application saw the command.
#![forbid(unsafe_code)]

#[path = "support/guide_process.rs"]
mod guide;
#[path = "support/guide_interactive.rs"]
mod interactive;
use bacnet_client::client::BACnetClient;
use bacnet_encoding::primitives::{decode_application_value, encode_property_value};
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use guide::{Process, WAIT};
use interactive::Interactive;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::path::Path;
use tokio::time::timeout;

const READY: &str = "Press Enter to stop (or send EOF).\n";

fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_guide-server-device"))
}

/// Wait for the two ready lines and return the address they name.
async fn ready(child: &Interactive) -> SocketAddrV4 {
    let text = wait_for(child, READY).await;
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    let address: SocketAddrV4 = lines[0]
        .strip_prefix("Serving device 1001 at ")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(*address.ip(), Ipv4Addr::LOCALHOST);
    assert_ne!(address.port(), 0);
    address
}

/// Wait until stdout ends with `tail`, and return all of it.
async fn wait_for(child: &Interactive, tail: &str) -> String {
    let mut stdout = child.stdout();
    let seen = timeout(
        WAIT,
        stdout.wait_for(|bytes| bytes.ends_with(tail.as_bytes())),
    )
    .await
    .unwrap_or_else(|_| panic!("the program printed {tail:?}"))
    .expect("stdout stays open")
    .clone();
    String::from_utf8(seen).unwrap()
}

fn mac(address: SocketAddrV4) -> Vec<u8> {
    let mut mac = address.ip().octets().to_vec();
    mac.extend_from_slice(&address.port().to_be_bytes());
    mac
}

async fn client() -> BACnetClient<BipTransport> {
    BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(Ipv4Addr::LOCALHOST)
        .apdu_timeout_ms(3000)
        .build()
        .await
        .unwrap()
}

fn temperature() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap()
}

fn fan() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 1).unwrap()
}

async fn read(
    client: &BACnetClient<BipTransport>,
    device: &[u8],
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
) -> PropertyValue {
    let ack = client
        .read_property(device, oid, property, None)
        .await
        .unwrap();
    decode_application_value(&ack.property_value, 0).unwrap().0
}

async fn command_fan(client: &BACnetClient<BipTransport>, device: &[u8], value: PropertyValue) {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, &value).unwrap();
    client
        .write_property(
            device,
            fan(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            encoded.to_vec(),
            Some(8),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn a_peer_reads_the_input_and_commands_the_output() {
    let mut child = Interactive::start(binary(), &[]).unwrap();
    let address = ready(&child).await;
    let device = mac(address);
    let mut client = client().await;

    // The value the application supplied at start, and the served names.
    assert_eq!(
        read(
            &client,
            &device,
            temperature(),
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(21.5)
    );
    assert_eq!(
        read(
            &client,
            &device,
            temperature(),
            PropertyIdentifier::OBJECT_NAME
        )
        .await,
        PropertyValue::CharacterString("Supply air temperature".into())
    );
    let device_object = ObjectIdentifier::new(ObjectType::DEVICE, 1001).unwrap();
    assert_eq!(
        read(
            &client,
            &device,
            device_object,
            PropertyIdentifier::OBJECT_NAME
        )
        .await,
        PropertyValue::CharacterString("Example air handler".into())
    );

    // A command at priority 8 reaches the application, which answers by
    // changing the input; relinquishing it turns the fan back off.
    command_fan(&client, &device, PropertyValue::Enumerated(1)).await;
    wait_for(&child, "Supply fan: on\n").await;
    assert_eq!(
        read(
            &client,
            &device,
            temperature(),
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(19.0)
    );
    command_fan(&client, &device, PropertyValue::Null).await;
    wait_for(&child, "Supply fan: off\n").await;
    assert_eq!(
        read(
            &client,
            &device,
            temperature(),
            PropertyIdentifier::PRESENT_VALUE
        )
        .await,
        PropertyValue::Real(21.5)
    );
    client.stop().await.unwrap();

    child.send_input(b"\n");
    let output = child.finish().await;
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "Serving device 1001 at {address}\n{READY}Supply fan: on\nSupply fan: off\nStopped\n"
        )
    );
    // Stopped means the port is free again.
    UdpSocket::bind(address).expect("the stopped server released its port");
}

#[tokio::test]
async fn end_of_input_also_stops_the_server() {
    let child = Interactive::start(binary(), &[]).unwrap();
    let address = ready(&child).await;
    // No input: finish closes stdin.
    let output = child.finish().await;
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("Serving device 1001 at {address}\n{READY}Stopped\n")
    );
    UdpSocket::bind(address).expect("the stopped server released its port");
}

#[tokio::test]
async fn arguments_are_refused_before_a_socket_opens() {
    let output = Process::spawn(binary(), &["47808"]).unwrap().finish().await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("usage: server-device"),
        "{output:?}"
    );
}
