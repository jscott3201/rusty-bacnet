//! The frame tap through a running client's transport (#957): a
//! ReadProperty to a real server shows up as an outbound request and an
//! inbound answer, whose bytes decode with the link's own codec, over B/IP,
//! B/IPv6 and MS/TP. Control traffic shows up too, and a subscriber that
//! never reads loses frames, counted, without holding the exchange up.
//! BACnet/SC is covered in `bacnet-transport` (scripted hub, paused time)
//! and `bacnet-client`'s `sc_frame_tap` (real hub over TLS).

use std::net::Ipv4Addr;

use bacnet_bip_codec::{BvlcResultCode, Bvll};
use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_encoding::npdu::decode_npdu;
use bacnet_mstp_codec::{decode_frame, FrameType};
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_objects::DeviceIdentity;
use bacnet_server::server::BACnetServer;
use bacnet_transport::bip::BvlcExchangeError;
use bacnet_transport::mstp::{LoopbackSerial, MstpConfig, MstpTransport};
use bacnet_transport::tap::{FrameTapReceiver, TapDirection, TapPeer, TappedFrame};
use bacnet_types::data_link::DataLink;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;
use tokio::time::{timeout, Duration};

const DEVICE: u32 = 1234;
/// Real sockets on a shared runner: generous, so load cannot fail a run
/// that takes milliseconds (#1811).
const WAIT: Duration = Duration::from_secs(30);

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let device = DeviceObject::new(DeviceConfig {
        identity: DeviceIdentity::new(DEVICE, 0).unwrap().with_name("Tapped"),
        ..DeviceConfig::default()
    })
    .unwrap();
    db.add(Box::new(device)).unwrap();
    db
}

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, DEVICE).unwrap()
}

/// Every frame queued so far.
fn drain(frames: &mut FrameTapReceiver) -> Vec<TappedFrame> {
    std::iter::from_fn(|| frames.try_recv()).collect()
}

/// What an NPDU carries, if it is a ReadProperty request or answer:
/// whether it is the request, and its invoke ID.
fn read_property(npdu: &[u8]) -> Option<(bool, u8)> {
    let npdu = decode_npdu(Bytes::copy_from_slice(npdu)).ok()?;
    match decode_apdu(npdu.payload).ok()? {
        Apdu::ConfirmedRequest(request)
            if request.service_choice == ConfirmedServiceChoice::READ_PROPERTY =>
        {
            Some((true, request.invoke_id))
        }
        Apdu::ComplexAck(ack) if ack.service_choice == ConfirmedServiceChoice::READ_PROPERTY => {
            Some((false, ack.invoke_id))
        }
        _ => None,
    }
}

/// The request went out and its answer came in, in that order, with one
/// invoke ID; `npdu` takes a frame to its NPDU with the link's codec.
fn assert_exchange(frames: &[TappedFrame], link: DataLink, npdu: impl Fn(&[u8]) -> Option<&[u8]>) {
    let exchange: Vec<_> = frames
        .iter()
        .filter_map(|frame| {
            assert_eq!(frame.link, link);
            let found = read_property(npdu(&frame.bytes)?)?;
            Some((frame.direction, found))
        })
        .collect();
    let [(TapDirection::Outbound, (true, sent)), (TapDirection::Inbound, (false, answered))] =
        exchange[..]
    else {
        panic!("one ReadProperty out and its answer in, got {exchange:?}");
    };
    assert_eq!(sent, answered);
}

fn bip_npdu(bytes: &[u8]) -> Option<&[u8]> {
    bacnet_bip_codec::decode(bytes).ok()?.npdu()
}

#[tokio::test]
async fn bip_read_property_and_bvlc_control_are_tapped() {
    let mut server = BACnetServer::bip_builder()
        .device(DeviceIdentity::new(DEVICE, 0).unwrap())
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(database())
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let mut frames = client.transport().frame_tap().subscribe();

    timeout(
        WAIT,
        client.read_property(&server_mac, device(), PropertyIdentifier::OBJECT_NAME, None),
    )
    .await
    .unwrap()
    .unwrap();
    let tapped = drain(&mut frames);
    assert_exchange(&tapped, DataLink::Bip, bip_npdu);
    let server_addr = bacnet_bip_codec::address_from_slice(&server_mac).unwrap();
    for frame in &tapped {
        assert_eq!(frame.peer, TapPeer::Udp(server_addr.into()));
    }

    // The server is no BBMD: a registration is refused with a BVLC-Result.
    let refused = timeout(WAIT, client.register_foreign_device_bvlc(&server_mac, 60))
        .await
        .unwrap();
    assert!(
        matches!(refused, Err(BvlcExchangeError::Nak(_))),
        "{refused:?}"
    );
    let control = drain(&mut frames);
    let control: Vec<_> = control
        .iter()
        .map(|frame| {
            (
                frame.direction,
                bacnet_bip_codec::decode(&frame.bytes).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        control,
        [
            (
                TapDirection::Outbound,
                Bvll::RegisterForeignDevice { ttl: 60 }
            ),
            (
                TapDirection::Inbound,
                Bvll::BvlcResult(BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK)
            ),
        ]
    );

    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// A subscriber that never reads fills its queue and loses the rest,
/// counted; the exchange goes on as if nobody were listening.
#[tokio::test]
async fn a_subscriber_that_never_reads_holds_nothing_up() {
    let mut server = BACnetServer::bip_builder()
        .device(DeviceIdentity::new(DEVICE, 0).unwrap())
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(database())
        .build()
        .await
        .unwrap();
    let server_mac = server.local_mac().to_vec();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    let stalled = client.transport().frame_tap().subscribe_with_capacity(1);
    for _ in 0..3 {
        timeout(
            WAIT,
            client.read_property(&server_mac, device(), PropertyIdentifier::OBJECT_NAME, None),
        )
        .await
        .expect("the tap must not hold a round trip up")
        .unwrap();
    }
    // Six frames at least; the first fills the queue.
    assert!(stalled.dropped() >= 5, "{}", stalled.dropped());
    let counters = client.transport().frame_tap().counters();
    assert_eq!(
        (counters.delivered, counters.dropped),
        (1, stalled.dropped())
    );
    drop(stalled);
    assert_eq!(client.transport().frame_tap().counters().subscribers, 0);
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

#[cfg(feature = "ipv6")]
#[tokio::test]
async fn bip6_read_property_and_address_resolution_are_tapped() {
    use bacnet_bip_codec::ipv6::Bvll6;
    use bacnet_transport::bip6::Bip6Transport;
    use std::net::Ipv6Addr;

    let mut server = BACnetServer::generic_builder()
        .device(DeviceIdentity::new(DEVICE, 0).unwrap())
        .database(database())
        .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(DEVICE)))
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(77)))
        .build()
        .await
        .unwrap();
    let mut frames = client.transport().frame_tap().subscribe();
    let server_address = server.transport().unwrap().local_address().unwrap();
    let vmac = timeout(
        WAIT,
        client.transport().resolve_virtual_address(server_address),
    )
    .await
    .unwrap()
    .unwrap();
    timeout(
        WAIT,
        client.read_property(
            &vmac.octets(),
            device(),
            PropertyIdentifier::OBJECT_NAME,
            None,
        ),
    )
    .await
    .unwrap()
    .unwrap();

    let tapped = drain(&mut frames);
    let resolution: Vec<_> = tapped
        .iter()
        .filter_map(
            |frame| match bacnet_bip_codec::ipv6::decode(&frame.bytes).unwrap() {
                Bvll6::VirtualAddressResolution { .. } => Some((frame.direction, true)),
                Bvll6::VirtualAddressResolutionAck { .. } => Some((frame.direction, false)),
                _ => None,
            },
        )
        .collect();
    assert_eq!(
        resolution,
        [
            (TapDirection::Outbound, true),
            (TapDirection::Inbound, false)
        ]
    );
    assert_exchange(&tapped, DataLink::Bip6, |bytes| {
        bacnet_bip_codec::ipv6::decode(bytes).ok()?.npdu()
    });
    for frame in &tapped {
        assert_eq!(frame.peer, TapPeer::Udp(server_address.into()));
    }
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Two masters on a loopback line: the token ring's control frames and a
/// ReadProperty's data frames all reach the client's tap, preamble to CRC.
#[tokio::test(start_paused = true)]
async fn mstp_token_ring_and_read_property_are_tapped() {
    let (client_line, server_line) = LoopbackSerial::pair();
    let config = |this_station| MstpConfig {
        this_station,
        max_master: 3,
        ..MstpConfig::default()
    };
    let client_transport = MstpTransport::new(client_line, config(1));
    let mut frames = client_transport.frame_tap().subscribe();
    let stalled = client_transport.frame_tap().subscribe_with_capacity(1);
    let mut server = BACnetServer::generic_builder()
        // The server makes its Device object, sized for the link.
        .device(DeviceIdentity::new(DEVICE, 0).unwrap())
        .transport(MstpTransport::new(server_line, config(2)))
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .build()
        .await
        .unwrap();
    timeout(
        WAIT,
        client.read_property(&[2], device(), PropertyIdentifier::OBJECT_NAME, None),
    )
    .await
    .expect("the tap must not hold a round trip up")
    .unwrap();
    assert!(stalled.dropped() > 0);

    let tapped = drain(&mut frames);
    let seen = |direction, frame_type| {
        tapped.iter().any(|frame| {
            let (decoded, consumed) = decode_frame(&frame.bytes).unwrap();
            assert_eq!(consumed, frame.bytes.len());
            let peer = match frame.direction {
                TapDirection::Inbound => decoded.header.source,
                _ => decoded.header.destination,
            };
            assert_eq!(frame.peer, TapPeer::Mstp(peer));
            frame.direction == direction && decoded.header.frame_type == frame_type
        })
    };
    for direction in [TapDirection::Outbound, TapDirection::Inbound] {
        assert!(seen(direction, FrameType::TOKEN), "{direction:?} Token");
        assert!(
            seen(direction, FrameType::POLL_FOR_MASTER),
            "{direction:?} Poll For Master"
        );
    }
    assert_exchange(&tapped, DataLink::Mstp, |bytes| {
        let (frame, _) = decode_frame(bytes).ok()?;
        (!frame.data.is_empty()).then_some(frame.data)
    });
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
