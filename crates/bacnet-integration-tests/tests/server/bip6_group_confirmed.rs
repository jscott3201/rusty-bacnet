//! A confirmed request that a B/IPv6 server receives as a group delivery is
//! neither executed nor answered (#1257, Clause 5.4.5.1).
//!
//! A BBMD hands a foreign device each multicast as a Forwarded-NPDU, which the
//! B/IPv6 port reports as a group delivery. That is the group path a loopback
//! test can drive: an Original-Broadcast only counts when it arrives on the
//! BACnet multicast group itself.

use bacnet_encoding::apdu::{
    decode_apdu, encode_apdu, Apdu, ConfirmedRequest as ConfirmedRequestPdu, UnconfirmedRequest,
};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_encoding::primitives::encode_app_real;
use bacnet_objects::analog::AnalogValueObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::device::{DeviceConfig, DeviceObject};
use bacnet_server::server::{BACnetServer, DiscoveryPolicy};
use bacnet_services::who_is::WhoIsRequest;
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_transport::bip6::{
    decode_bvll6, encode_bvll6, Bip6ForeignDeviceConfig, Bip6Transport, Bip6Vmac, Bvll6,
};
use bacnet_types::enums::{
    ConfirmedServiceChoice, ObjectType, PropertyIdentifier, UnconfirmedServiceChoice,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use bytes::{Bytes, BytesMut};
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use tokio::net::UdpSocket;
use tokio::time::Duration;

pub(super) const DEVICE: u32 = 6257;

/// A Device and AV-1, whose Present_Value starts at 0.0.
pub(super) fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        DeviceObject::new(DeviceConfig {
            identity: bacnet_objects::DeviceIdentity::new(DEVICE, 0)
                .expect("valid Device identity")
                .with_name("B/IPv6 server"),
            ..DeviceConfig::default()
        })
        .unwrap(),
    ))
    .unwrap();
    db.add(Box::new(
        AnalogValueObject::new(
            1,
            "AV-1",
            bacnet_types::enums::EngineeringUnits::DEGREES_CELSIUS,
        )
        .unwrap(),
    ))
    .unwrap();
    db
}

pub(super) fn npdu(apdu: &Apdu, expecting_reply: bool) -> Vec<u8> {
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
    npdu.to_vec()
}

/// `npdu` as a BBMD forwards a multicast to its foreign devices, from a node
/// at a routable documentation address.
fn forwarded(npdu: &[u8]) -> BytesMut {
    let origin = SocketAddrV6::new(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 9), 47808, 0, 0);
    let mut frame = BytesMut::new();
    encode_bvll6(
        &mut frame,
        &Bvll6::ForwardedNpdu {
            original_source: Bip6Vmac::new([0xAA, 0xAA, 0xAA]),
            original_address: origin.into(),
            npdu,
        },
    )
    .unwrap();
    frame
}

pub(super) fn write_present_value(value: f32) -> Apdu {
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
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
        service_request: request.freeze(),
    })
}

pub(super) fn who_is() -> Apdu {
    let mut request = BytesMut::new();
    WhoIsRequest { range: None }.encode(&mut request);
    Apdu::UnconfirmedRequest(UnconfirmedRequest {
        service_choice: UnconfirmedServiceChoice::WHO_IS,
        service_request: request.freeze(),
    })
}

/// Whether one datagram the server sent the BBMD carries an I-Am.
fn is_i_am(datagram: &[u8]) -> bool {
    let npdu = match decode_bvll6(datagram) {
        Ok(
            Bvll6::DistributeBroadcastToNetwork { npdu, .. }
            | Bvll6::OriginalUnicastNpdu { npdu, .. },
        ) => npdu,
        _ => return false,
    };
    let Ok(npdu) = decode_npdu(Bytes::copy_from_slice(npdu)) else {
        return false;
    };
    matches!(
        decode_apdu(npdu.payload),
        Ok(Apdu::UnconfirmedRequest(UnconfirmedRequest { service_choice, .. }))
            if service_choice == UnconfirmedServiceChoice::I_AM
    )
}

#[tokio::test]
async fn a_confirmed_request_forwarded_to_a_bip6_foreign_device_is_ignored() {
    let bbmd = UdpSocket::bind("[::1]:0").await.unwrap();
    let SocketAddr::V6(bbmd_address) = bbmd.local_addr().unwrap() else {
        unreachable!("bound to an IPv6 address");
    };
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(DEVICE));
    transport.register_as_foreign_device(Bip6ForeignDeviceConfig {
        bbmd_ip: *bbmd_address.ip(),
        bbmd_port: bbmd_address.port(),
        ttl: 60,
    });
    // The I-Am fence goes by broadcast, which a foreign device hands its BBMD.
    let mut server = BACnetServer::generic_builder()
        .device(bacnet_objects::DeviceIdentity::new(DEVICE, 0).unwrap())
        .database(database())
        .discovery_policy(DiscoveryPolicy {
            prefer_directed_responses: false,
            ..DiscoveryPolicy::default()
        })
        .transport(transport)
        .build()
        .await
        .unwrap();
    let port = server.transport().unwrap().local_address().unwrap().port();
    let server_address = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);

    // The WriteProperty goes first; the Who-Is behind it is the fence.
    for apdu in [(write_present_value(42.0), true), (who_is(), false)] {
        bbmd.send_to(&forwarded(&npdu(&apdu.0, apdu.1)), server_address)
            .await
            .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut datagram = [0u8; 1500];
        loop {
            let (length, _) = bbmd.recv_from(&mut datagram).await.unwrap();
            if is_i_am(&datagram[..length]) {
                break;
            }
        }
    })
    .await
    .expect("the server answers the forwarded Who-Is");

    // The write was dropped, not executed: the value never moves.
    let analog_value = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 1).unwrap();
    for _ in 0..20 {
        let value = server
            .database()
            .read()
            .await
            .get(&analog_value)
            .unwrap()
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(value, PropertyValue::Real(0.0));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    server.stop().await.unwrap();
}
