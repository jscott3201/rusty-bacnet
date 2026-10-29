//! A B/IPv6 server neither executes nor answers a confirmed request carried
//! in an Original-Unicast-NPDU that was sent to the BACnet multicast group
//! (#1301). The same request sent to the server's own address is answered.
//!
//! The test sends to FF02::BAC0 on the loopback interface. macOS loops that
//! back to a local socket; a Linux loopback without the multicast flag does
//! not. macOS is where this test is the B/IPv6 real-socket evidence, so there
//! it fails if the probe does not get through; elsewhere it skips.

use super::bip6_group_confirmed::{DEVICE, database, npdu, who_is, write_present_value};
use bacnet_encoding::apdu::{Apdu, SimpleAck, decode_apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip6::{
    BACNET_IPV6_MULTICAST_LINK_LOCAL, Bip6Transport, Bip6Vmac, Bvll6, decode_bvll6, encode_bvll6,
    vmac_from_device_instance,
};
use bacnet_types::enums::{ObjectType, PropertyIdentifier, UnconfirmedServiceChoice};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};
use socket2::{Domain, Protocol, Socket, Type};
use std::net::{Ipv6Addr, SocketAddrV6};
use tokio::net::UdpSocket;
use tokio::time::{Duration, timeout};

const CLIENT_VMAC: Bip6Vmac = Bip6Vmac::new([0x0A, 0x0B, 0x0C]);
/// The OS whose loopback carries FF02::BAC0, where the test has to run.
const EVIDENCE_OS: bool = cfg!(target_os = "macos");

/// The loopback interface's index, looked up by its usual names.
fn loopback_index() -> Option<u32> {
    ["lo0", "lo"]
        .into_iter()
        .find_map(|name| nix::net::if_::if_nametoindex(name).ok())
}

/// An IPv6 UDP socket bound to `address` that sends multicast out the
/// interface `index` and hears its own.
fn socket(address: Ipv6Addr, index: u32) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_only_v6(true)?;
    socket.set_multicast_if_v6(index)?;
    socket.set_multicast_loop_v6(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&SocketAddrV6::new(address, 0, 0, 0).into())?;
    UdpSocket::from_std(socket.into())
}

/// Whether a datagram sent to FF02::BAC0 on the loopback interface reaches a
/// local socket that joined the group there. On the evidence OS the probe
/// keeps trying for ten seconds, so a loaded runner does not read as one that
/// cannot.
async fn loopback_multicast_is_delivered(index: u32) -> bool {
    let Ok(receiver) = socket(Ipv6Addr::UNSPECIFIED, index) else {
        return false;
    };
    if receiver
        .join_multicast_v6(&BACNET_IPV6_MULTICAST_LINK_LOCAL, index)
        .is_err()
    {
        return false;
    }
    let port = receiver.local_addr().unwrap().port();
    let Ok(sender) = socket(Ipv6Addr::LOCALHOST, index) else {
        return false;
    };
    let group = SocketAddrV6::new(BACNET_IPV6_MULTICAST_LINK_LOCAL, port, 0, index);
    let mut datagram = [0u8; 8];
    for _ in 0..if EVIDENCE_OS { 10 } else { 1 } {
        if sender.send_to(b"probe", group).await.is_err() {
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

/// The APDU in one datagram the server sent, if it carries one.
fn apdu_of(datagram: &[u8]) -> Option<Apdu> {
    let frame = decode_bvll6(datagram).ok()?;
    let npdu = decode_npdu(Bytes::copy_from_slice(frame.npdu()?)).ok()?;
    decode_apdu(npdu.payload).ok()
}

async fn present_value(server: &BACnetServer<Bip6Transport>) -> PropertyValue {
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
async fn an_original_unicast_sent_to_the_multicast_group_is_not_answered() {
    let delivered = match loopback_index() {
        Some(index) => loopback_multicast_is_delivered(index)
            .await
            .then_some(index),
        None => None,
    };
    let Some(index) = delivered else {
        if EVIDENCE_OS {
            panic!("macOS loops FF02::BAC0 back on lo0, and this test needs it");
        }
        eprintln!("skipping: this host does not loop FF02::BAC0 back on the loopback interface");
        return;
    };
    let mut server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(DEVICE, 0).unwrap())
        .database(database())
        .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(DEVICE)))
        .build()
        .await
        .unwrap();
    let port = server.transport().unwrap().local_address().unwrap().port();
    let server_vmac = vmac_from_device_instance(DEVICE).unwrap();
    assert_eq!(
        server.local_mac(),
        server_vmac.octets(),
        "the MAC is the VMAC"
    );
    let client = socket(Ipv6Addr::LOCALHOST, index).unwrap();
    let to_group = SocketAddrV6::new(BACNET_IPV6_MULTICAST_LINK_LOCAL, port, 0, index);
    let to_server = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
    let mut request = BytesMut::new();
    encode_bvll6(
        &mut request,
        &Bvll6::OriginalUnicastNpdu {
            source: CLIENT_VMAC,
            destination: server_vmac,
            npdu: &npdu(&write_present_value(42.0), true),
        },
    )
    .unwrap();

    // The WriteProperty goes first; the Who-Is behind it, multicast the
    // proper way, is the fence: its I-Am comes back once the server has
    // taken both datagrams off the socket.
    client.send_to(&request, to_group).await.unwrap();
    let mut fence = BytesMut::new();
    encode_bvll6(
        &mut fence,
        &Bvll6::OriginalBroadcastNpdu {
            source: CLIENT_VMAC,
            npdu: &npdu(&who_is(), false),
        },
    )
    .unwrap();
    client.send_to(&fence, to_group).await.unwrap();
    timeout(Duration::from_secs(5), async {
        let mut datagram = [0u8; 1500];
        loop {
            let (length, _) = client.recv_from(&mut datagram).await.unwrap();
            match apdu_of(&datagram[..length]) {
                Some(Apdu::UnconfirmedRequest(request))
                    if request.service_choice == UnconfirmedServiceChoice::I_AM =>
                {
                    break;
                }
                Some(Apdu::UnconfirmedRequest(_)) | None => {}
                Some(answer) => panic!("the multicast request drew an answer: {answer:?}"),
            }
        }
    })
    .await
    .expect("the server answers the multicast Who-Is");
    for _ in 0..20 {
        assert_eq!(present_value(&server).await, PropertyValue::Real(0.0));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    // Sent to the server's own address, the same request is carried out.
    client.send_to(&request, to_server).await.unwrap();
    timeout(Duration::from_secs(5), async {
        let mut datagram = [0u8; 1500];
        loop {
            let (length, _) = client.recv_from(&mut datagram).await.unwrap();
            if let Some(Apdu::SimpleAck(SimpleAck { invoke_id: 1, .. })) =
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
