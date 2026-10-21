//! A B/IP server neither executes nor answers a confirmed request carried in
//! an Original-Unicast-NPDU that was sent to the broadcast address (#1301).
//! The same request sent to the server's own address is answered.
//!
//! Loopback broadcast to 127.255.255.255 reaches a local socket on Linux but
//! not on macOS. Linux is where this test is the B/IP real-socket evidence, so
//! there it fails if the probe does not get through; elsewhere it skips.

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu, SimpleAck,
    UnconfirmedRequest,
};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_encoding::primitives::encode_app_real;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::who_is::WhoIsRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::bip::BipTransport;
use bacnet_transport::bvll::{decode_bip_mac, decode_bvll, BvlcFunction};
use bacnet_types::enums::{
    ConfirmedServiceChoice, EngineeringUnits, ObjectType, PropertyIdentifier,
    UnconfirmedServiceChoice,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::BytesMut;
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

const DEVICE: u32 = 6301;
const LOOPBACK_BROADCAST: Ipv4Addr = Ipv4Addr::new(127, 255, 255, 255);
/// The OS whose loopback delivers 127.255.255.255, where the test has to run.
const EVIDENCE_OS: bool = cfg!(target_os = "linux");

/// `apdu` in an NPDU, in a BVLL frame with `function`.
fn bvll(function: BvlcFunction, apdu: &Apdu, expecting_reply: bool) -> Vec<u8> {
    let mut payload = BytesMut::new();
    encode_apdu(&mut payload, apdu).unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            expecting_reply,
            payload: payload.freeze(),
            ..Default::default()
        },
    )
    .unwrap();
    let mut frame = vec![0x81, function.to_raw()];
    frame.extend_from_slice(&(4 + npdu.len() as u16).to_be_bytes());
    frame.extend_from_slice(&npdu);
    frame
}

fn write_present_value(invoke_id: u8, value: f32) -> Apdu {
    let mut property_value = BytesMut::new();
    encode_app_real(&mut property_value, value);
    let mut request = BytesMut::new();
    WritePropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        property_value: property_value.to_vec(),
        priority: None,
    }
    .encode(&mut request)
    .unwrap();
    Apdu::ConfirmedRequest(ConfirmedRequestPdu {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: false,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: request.freeze(),
    })
}

fn who_is() -> Apdu {
    let mut request = BytesMut::new();
    WhoIsRequest { range: None }.encode(&mut request);
    Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: request.freeze(),
    })
}

/// The APDU in one datagram the server sent, if it carries one.
fn apdu_of(datagram: &[u8]) -> Option<Apdu> {
    let message = decode_bvll(datagram).ok()?;
    let npdu = decode_npdu(message.payload).ok()?;
    decode_apdu(npdu.payload).ok()
}

/// Whether a datagram sent to the loopback broadcast address reaches a socket
/// bound to 0.0.0.0 on this host. On the evidence OS the probe keeps trying
/// for ten seconds, so a loaded runner does not read as one that cannot.
async fn loopback_broadcast_is_delivered() -> bool {
    let receiver = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.unwrap();
    let port = receiver.local_addr().unwrap().port();
    let sender = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    sender.set_broadcast(true).unwrap();
    let mut datagram = [0u8; 8];
    for _ in 0..if EVIDENCE_OS { 10 } else { 1 } {
        if sender
            .send_to(b"probe", (LOOPBACK_BROADCAST, port))
            .await
            .is_err()
        {
            return false;
        }
        if matches!(
            timeout(Duration::from_secs(1), receiver.recv_from(&mut datagram)).await,
            Ok(Ok((5, _))) if &datagram[..5] == b"probe"
        ) {
            return true;
        }
    }
    false
}

async fn server() -> BACnetServer<BipTransport> {
    let identity = bacnet_objects::DeviceIdentity::new(DEVICE, 0)
        .expect("valid Device identity")
        .with_name("B/IP server");
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone(),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .broadcast_address(LOOPBACK_BROADCAST)
        .device(identity)
        .database(db)
        .build()
        .await
        .unwrap()
}

async fn present_value(server: &BACnetServer<BipTransport>) -> PropertyValue {
    server
        .database()
        .read()
        .await
        .get(&ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap())
        .unwrap()
        .read_property(PropertyIdentifier::PRESENT_VALUE, None)
        .unwrap()
}

#[tokio::test]
async fn an_original_unicast_sent_to_the_broadcast_address_is_not_answered() {
    if !loopback_broadcast_is_delivered().await {
        if EVIDENCE_OS {
            panic!("Linux delivers 127.255.255.255 to a local socket, and this test needs it");
        }
        eprintln!("skipping: this host does not deliver 127.255.255.255 to a local socket");
        return;
    }
    let mut server = server().await;
    let (_, port) = decode_bip_mac(server.local_mac()).unwrap();
    let client = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    client.set_broadcast(true).unwrap();
    let to_everyone = SocketAddrV4::new(LOOPBACK_BROADCAST, port);
    let to_server = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);

    // The WriteProperty goes first; the Who-Is behind it, broadcast the
    // proper way, is the fence: its I-Am comes back once the server has
    // taken both datagrams off the socket.
    let unicast = BvlcFunction::ORIGINAL_UNICAST_NPDU;
    let request = bvll(unicast, &write_present_value(1, 42.0), true);
    client.send_to(&request, to_everyone).await.unwrap();
    let fence = bvll(BvlcFunction::ORIGINAL_BROADCAST_NPDU, &who_is(), false);
    client.send_to(&fence, to_everyone).await.unwrap();
    timeout(Duration::from_secs(5), async {
        let mut datagram = [0u8; 1500];
        loop {
            let (length, _) = client.recv_from(&mut datagram).await.unwrap();
            match apdu_of(&datagram[..length]) {
                Some(Apdu::UnconfirmedRequest(request))
                    if request.service_choice == UnconfirmedServiceChoice::I_AM =>
                {
                    break
                }
                Some(Apdu::UnconfirmedRequest(_)) | None => {}
                Some(answer) => panic!("the broadcast request drew an answer: {answer:?}"),
            }
        }
    })
    .await
    .expect("the server answers the broadcast Who-Is");
    for _ in 0..20 {
        assert_eq!(present_value(&server).await, PropertyValue::Real(0.0));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Sent to the server's own address, the same request is carried out.
    let request = bvll(unicast, &write_present_value(2, 42.0), true);
    client.send_to(&request, to_server).await.unwrap();
    timeout(Duration::from_secs(5), async {
        let mut datagram = [0u8; 1500];
        loop {
            let (length, _) = client.recv_from(&mut datagram).await.unwrap();
            if let Some(Apdu::SimpleAck(SimpleAck { invoke_id: 2, .. })) =
                apdu_of(&datagram[..length])
            {
                break;
            }
        }
    })
    .await
    .expect("the directed request is acknowledged");
    assert_eq!(present_value(&server).await, PropertyValue::Real(42.0));
    server.stop().await.unwrap();
}
