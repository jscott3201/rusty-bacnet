//! A BBMD server's own I-Am reaches its BDT peers and registered foreign
//! devices as a Forwarded-NPDU, so remote devices discover it (#937).

use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::decode_npdu;
use bacnet_server::server::ServerConfig;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::bbmd::{BdtEntry, ForeignDevicePolicy};
use bacnet_transport::bvll::{
    decode_bip_mac, decode_bvll, encode_bvll, BvlcFunction, BvlcResultCode, Bvll,
};
use bacnet_types::enums::UnconfirmedServiceChoice;
use std::net::SocketAddrV4;
use tokio::net::UdpSocket;
use tokio::time::timeout;

const BBMD_DEVICE: u32 = 937;

async fn recv_frame(socket: &UdpSocket) -> bacnet_transport::bvll::BvllMessage {
    let mut buf = [0u8; 2048];
    let (len, _) = timeout(Duration::from_secs(2), socket.recv_from(&mut buf))
        .await
        .expect("timed out waiting for a BVLL frame")
        .unwrap();
    decode_bvll(&buf[..len]).unwrap()
}

#[tokio::test]
async fn bbmd_own_i_am_reaches_bdt_peer_device_and_foreign_device() {
    // A device on a remote subnet hears what its BBMD rebroadcasts there: a
    // Forwarded-NPDU carrying the originator's address. Listing this client in
    // the BDT as a unicast (two-hop) peer delivers exactly that frame to it.
    let mut client = make_client().await;
    let (client_ip, client_port) = decode_bip_mac(client.local_mac()).unwrap();
    let foreign = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();

    let mut transport = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    transport.enable_bbmd(vec![BdtEntry {
        ip: client_ip,
        port: client_port,
        broadcast_mask: [255; 4],
    }]);
    transport.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: bacnet_objects::DeviceIdentity::new(BBMD_DEVICE, 0)
            .expect("valid Device identity")
            .with_name("BBMD Device"),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();
    let mut server = BACnetServer::start(
        ServerConfig {
            device: bacnet_objects::DeviceIdentity::new(BBMD_DEVICE, 0).unwrap(),
            ..Default::default()
        },
        db,
        transport,
    )
    .await
    .unwrap();
    let bbmd_mac = server.local_mac().to_vec();
    let (bbmd_ip, bbmd_port) = decode_bip_mac(&bbmd_mac).unwrap();

    let mut register = BytesMut::new();
    encode_bvll(&mut register, &Bvll::RegisterForeignDevice { ttl: 60 }).unwrap();
    foreign
        .send_to(
            &register,
            SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port),
        )
        .await
        .unwrap();
    let result = recv_frame(&foreign).await;
    assert_eq!(result.function, BvlcFunction::BVLC_RESULT);
    assert_eq!(
        result.payload.as_ref(),
        BvlcResultCode::SUCCESSFUL_COMPLETION.to_raw().to_be_bytes()
    );

    server.broadcast_i_am().await.unwrap();

    // The foreign device gets the I-Am with the BBMD's own address as origin.
    let forwarded = recv_frame(&foreign).await;
    assert_eq!(forwarded.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(
        forwarded.origin(),
        Some(SocketAddrV4::new(Ipv4Addr::from(bbmd_ip), bbmd_port))
    );
    let npdu = decode_npdu(forwarded.npdu().unwrap()).unwrap();
    let Apdu::UnconfirmedRequest(apdu) = decode_apdu(npdu.payload).unwrap() else {
        panic!("expected an unconfirmed request");
    };
    assert_eq!(apdu.service_choice, UnconfirmedServiceChoice::I_AM);
    assert_eq!(
        IAmRequest::decode(&apdu.service_request)
            .unwrap()
            .object_identifier,
        ObjectIdentifier::new(ObjectType::DEVICE, BBMD_DEVICE).unwrap()
    );

    // The remote-subnet client discovers the BBMD's device at the BBMD's address.
    let discovered = timeout(Duration::from_secs(2), async {
        loop {
            if let Some(device) = client.get_device(BBMD_DEVICE).await {
                break device;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("remote client never discovered the BBMD's own device");
    assert_eq!(discovered.mac_address.as_slice(), bbmd_mac.as_slice());

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
