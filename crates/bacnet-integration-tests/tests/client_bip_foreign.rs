//! Loopback-only client foreign registration and broadcast discovery wiring.
use std::net::Ipv4Addr;
use std::time::Duration;

use bacnet_client::client::{BACnetClient, DeviceEventKind};
use bacnet_encoding::{
    apdu::{decode_apdu, encode_apdu, Apdu, UnconfirmedRequest},
    npdu::{decode_npdu, encode_npdu, Npdu},
};
use bacnet_services::who_is::IAmRequest;
use bacnet_transport::{
    bbmd::{BdtEntry, ForeignDevicePolicy},
    bip::{BipTransport, ForeignDeviceConfig, ForeignRegistrationOutcome},
    bvll::decode_bvll,
    port::TransportPort,
};
use bacnet_types::{
    enums::{BvlcFunction, ObjectType, Segmentation, UnconfirmedServiceChoice},
    primitives::ObjectIdentifier,
};
use bytes::BytesMut;
use tokio::{net::UdpSocket, time::timeout};

fn i_am() -> BytesMut {
    let mut service = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 2201).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NONE,
        vendor_id: 999,
    }
    .encode(&mut service);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequest {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    npdu
}

#[tokio::test]
async fn bip_foreign_builder_discovers_device_through_bbmd_after_observed_registration() {
    // An independent BDT peer captures the BBMD's Forwarded-NPDU output. All
    // sockets use private loopback ports; no physical broadcast is needed.
    let observer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let mut bbmd = BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST);
    bbmd.enable_bbmd(vec![BdtEntry {
        ip: Ipv4Addr::LOCALHOST.octets(),
        port: observer.local_addr().unwrap().port(),
        broadcast_mask: [255; 4],
    }]);
    bbmd.enable_foreign_device_registration(ForeignDevicePolicy::default());
    let mut bbmd_input = bbmd.start().await.unwrap();
    let bbmd_mac = bbmd.local_mac().to_vec();
    let bbmd_port = u16::from_be_bytes([bbmd_mac[4], bbmd_mac[5]]);
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .foreign_device(ForeignDeviceConfig {
            bbmd_ip: Ipv4Addr::LOCALHOST,
            bbmd_port,
            ttl: 60,
            renewal_interval: Some(Duration::from_secs(20)),
        })
        .build()
        .await
        .unwrap();
    timeout(Duration::from_secs(2), async {
        loop {
            if client
                .transport()
                .bvlc_client_snapshot()
                .foreign_registration
                .last_outcome
                == Some(ForeignRegistrationOutcome::Accepted)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("BBMD never acknowledged registration");
    let snapshot = client.transport().bvlc_client_snapshot();
    assert_eq!(
        (
            snapshot.foreign_registration.attempts,
            snapshot.foreign_registration.accepted
        ),
        (1, 1)
    );
    assert_eq!(snapshot.foreign_registration.last_ttl, Some(60));
    assert_eq!(bbmd.fdt_counters().unwrap().registrations_accepted, 1);

    // Client-level management helpers feed the same per-transport snapshot.
    assert!(!client.read_bdt(&bbmd_mac).await.unwrap().is_empty());
    let foreign_entries = client.read_fdt(&bbmd_mac).await.unwrap();
    assert_eq!(foreign_entries.len(), 1);
    assert_eq!(foreign_entries[0].ttl, 60);
    let snapshot = client.transport().bvlc_client_snapshot();
    assert_eq!(snapshot.read_bdt.acknowledgements, 1);
    assert_eq!(snapshot.read_fdt.acknowledgements, 1);

    let mut discovered = client.device_events();
    client.who_is(None).await.unwrap();
    let received = timeout(Duration::from_secs(2), bbmd_input.recv())
        .await
        .unwrap()
        .unwrap();
    let npdu = decode_npdu(received.npdu).unwrap();
    assert!(
        matches!(decode_apdu(npdu.payload).unwrap(), Apdu::UnconfirmedRequest(r)
        if r.service_choice == UnconfirmedServiceChoice::WHO_IS)
    );
    let mut packet = [0; 2048];
    let (length, _) = timeout(Duration::from_secs(2), observer.recv_from(&mut packet))
        .await
        .unwrap()
        .unwrap();
    let forwarded = decode_bvll(&packet[..length]).unwrap();
    assert_eq!(forwarded.function, BvlcFunction::FORWARDED_NPDU);
    assert_eq!(forwarded.originating_ip, Some(Ipv4Addr::LOCALHOST.octets()));
    let client_mac = client.local_mac();
    assert_eq!(
        forwarded.originating_port,
        Some(u16::from_be_bytes([client_mac[4], client_mac[5]]))
    );

    // Its own I-Am is forwarded to the registered foreign client through the
    // actual BBMD fanout and the client's normal discovery intake.
    bbmd.send_broadcast(&i_am()).await.unwrap();
    let event = timeout(Duration::from_secs(2), discovered.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.kind, DeviceEventKind::Discovered);
    assert_eq!(event.device.object_identifier.instance_number(), 2201);
    assert_eq!(event.device.vendor_id, 999);
    assert_eq!(event.device.mac_address.as_slice(), bbmd_mac);
    client.stop().await.unwrap();
    assert_eq!(
        client
            .transport()
            .bvlc_client_snapshot()
            .foreign_registration
            .next_attempt_in,
        None
    );
    bbmd.stop().await.unwrap();
}
