//! A routed device's router through a real `BACnetRouter` (#2823): the
//! client on network 1 finds the server, station 5 on network 2, through
//! the router at MAC 1. Another station on network 1 then announces the
//! same device as though it routed to network 2. The client's writes still
//! go through the router, its own check asks the device through the router
//! and gets the answer, and nothing is ever sent to the other station.

use std::time::Duration;

use bacnet_client::client::{BACnetClient, Broadcast, Station};
use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::{Npdu, NpduAddress, encode_npdu};
use bacnet_network::router::{BACnetRouter, RouterOptions, RouterPort, StartedRouter};
use bacnet_npdu::NetworkMessageType;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::BACnetServer;
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::any::AnyTransport;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_transport::test_transport::{SendMode, StartMode, TestTransport};
use bacnet_types::MacAddr;
use bacnet_types::enums::{
    EngineeringUnits, ObjectType, PropertyIdentifier, Segmentation, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::BytesMut;
use tokio::time::sleep;

const INSTANCE: u32 = 4321;
const ROUTER_MAC: u8 = 0x01;
const REMOTE: u16 = 2;
const STATION: u8 = 0x05;
/// Another station on network 1, posing as a router to network 2.
const OTHER: u8 = 0x66;

async fn server(transport: LoopbackTransport) -> BACnetServer<LoopbackTransport> {
    let identity = bacnet_objects::DeviceIdentity::new(INSTANCE, 0).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: identity.clone().with_name("Air handler"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "Setpoint", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .transport(transport)
        .build()
        .await
        .unwrap()
}

/// The device's I-Am as a station on network 1 would send it posing as a
/// router to network 2: SNET 2 and SADR 5.
fn claimed_i_am() -> ReceivedNpdu {
    let mut service_request = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, INSTANCE).unwrap(),
        max_apdu_length: 50,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 0,
    }
    .encode(&mut service_request);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source: Some(NpduAddress {
                network: REMOTE,
                mac_address: MacAddr::from_slice(&[STATION]),
            }),
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(&[OTHER]),
        link_layer_group: false,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    }
}

async fn write(client: &BACnetClient<TestTransport>, value: f32) {
    let station = client.resolve_device(INSTANCE).await.unwrap();
    assert_eq!(station, Station::routed(&[ROUTER_MAC], REMOTE, &[STATION]));
    let mut encoded = vec![0x44];
    encoded.extend_from_slice(&value.to_be_bytes());
    client
        .write_property(
            &station,
            ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
            encoded,
            Some(8),
        )
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn another_station_announcing_the_device_does_not_carry_its_writes() {
    let (client_end, router_near) = LoopbackTransport::pair(vec![0x0A], vec![ROUTER_MAC]);
    let (router_far, station) = LoopbackTransport::pair(vec![0x02], vec![STATION]);
    let StartedRouter { mut router, .. } = BACnetRouter::start(
        vec![
            RouterPort::new(AnyTransport::Loopback(router_near), 1),
            RouterPort::new(AnyTransport::Loopback(router_far), REMOTE),
        ],
        RouterOptions::new(),
    )
    .await
    .unwrap();
    let mut server = server(station).await;
    let builder = TestTransport::builder()
        .inner(client_end)
        .start(StartMode::Relay(64))
        .unicast(SendMode::Forward)
        .broadcast(SendMode::Forward);
    let (sent, link) = (builder.sent(), builder.handle());
    let mut client = BACnetClient::generic_builder()
        .transport(builder.build())
        .build()
        .await
        .unwrap();
    // Let the router's start-up announcements go by.
    sleep(Duration::from_millis(100)).await;

    client.who_is(Broadcast::Global, None).await.unwrap();
    sleep(Duration::from_secs(1)).await;
    let row = client
        .get_device_at(REMOTE, &[STATION])
        .await
        .expect("the server's I-Am came through the router");
    assert_eq!(row.mac_address.as_slice(), [ROUTER_MAC]);
    write(&client, 21.5).await;

    link.inbound()
        .expect("the client's link is started")
        .send(claimed_i_am())
        .await
        .unwrap();
    sleep(Duration::from_millis(10)).await;
    let row = client.get_device_at(REMOTE, &[STATION]).await.unwrap();
    assert_eq!(row.mac_address.as_slice(), [ROUTER_MAC]);
    assert_eq!(row.max_apdu_length, 1476, "the claim's limits were taken");
    write(&client, 22.5).await;

    // The client's check: a Who-Is to the device through the router, which
    // the device answers through it; no router query follows.
    sleep(Duration::from_secs(30)).await;
    let frames = sent.frames();
    let probes: Vec<_> = frames
        .iter()
        .filter(|frame| {
            let npdu = frame.decode_npdu();
            npdu.message_type.is_none()
                && npdu
                    .destination
                    .as_ref()
                    .is_some_and(|d| d.network == REMOTE)
                && matches!(frame.apdu(), Apdu::UnconfirmedRequest(ref r)
                    if r.service_choice == UnconfirmedServiceChoice::WHO_IS)
        })
        .collect();
    assert_eq!(probes.len(), 1, "{frames:?}");
    assert_eq!(probes[0].mac.as_slice(), [ROUTER_MAC]);
    assert!(
        frames.iter().all(|frame| frame.decode_npdu().message_type
            != Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)),
        "no router query: the device answered through the router"
    );
    assert!(frames.iter().all(|frame| frame.mac.as_slice() != [OTHER]));
    write(&client, 23.5).await;

    client.stop().await.unwrap();
    server.stop().await.unwrap();
    router.stop().await.unwrap();
}
