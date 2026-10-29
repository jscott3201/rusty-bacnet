//! Malformed and unsupported inputs never become discovery observations.
use super::*;
fn wire(message: Bvll<'_>) -> Vec<u8> {
    let mut bytes = vec![0; 512];
    let len = message.encode_into(&mut bytes).unwrap();
    bytes.truncate(len);
    bytes
}
fn peer() -> SocketAddrV4 {
    "127.0.0.1:47808".parse().unwrap()
}
#[test]
fn passive_native_router_reports_validate_payload_and_delivery_kind() {
    let router = [1, 0x80, 1, 0, 2, 0, 3];
    assert_eq!(
        decode(&wire(Bvll::OriginalBroadcastNpdu(&router)), peer()),
        Some(DiscoveryEvent::Router {
            router: peer().to_string(),
            networks: vec![2, 3]
        })
    );
    // A routed announcement's station and immediate next hop remain distinct.
    let routed = [1, 0x88, 0, 7, 1, 5, 1, 0, 2];
    assert_eq!(
        decode(&wire(Bvll::OriginalUnicastNpdu(&routed)), peer()),
        Some(DiscoveryEvent::Router {
            router: "7:0x05 via 127.0.0.1:47808".into(),
            networks: vec![2]
        })
    );
    let number = [1, 0x80, 0x13, 0, 77, 1];
    assert_eq!(
        decode(&wire(Bvll::OriginalBroadcastNpdu(&number)), peer()),
        Some(DiscoveryEvent::NetworkNumber {
            network: 77,
            configured: true
        })
    );
    assert_eq!(
        decode(&wire(Bvll::OriginalUnicastNpdu(&number)), peer()),
        None
    );
    for payload in [
        &[1, 0x80, 1][..],
        &[1, 0x80, 1, 0],
        &[1, 0x80, 1, 0, 0],
        &[1, 0x80, 1, 255, 255],
        &[1, 0x80, 0x13, 0, 77, 2],
        &[1, 0x88, 0, 7, 1, 5, 0x13, 0, 77, 1],
        &[1, 0x80, 0],
        &[1, 0, 0x10, 1],
        &[2, 0],
        &[1],
    ] {
        assert_eq!(
            decode(&wire(Bvll::OriginalBroadcastNpdu(payload)), peer()),
            None,
            "{payload:?}"
        );
    }
}
#[test]
fn passive_rejects_truncation_wrong_bvll_and_management_wrapped_data() {
    let router = [1, 0x80, 1, 0, 2];
    let valid = wire(Bvll::ForwardedNpdu {
        origin: peer(),
        npdu: &router,
    });
    for length in 0..valid.len() {
        assert_eq!(decode(&valid[..length], peer()), None);
    }
    for (offset, value) in [(0, 0x82), (1, 0xFF), (2, 0xFF), (3, 0)] {
        let mut changed = valid.clone();
        changed[offset] = value;
        assert_eq!(decode(&changed, peer()), None);
    }
    assert_eq!(
        decode(&wire(Bvll::DistributeBroadcastToNetwork(&router)), peer()),
        None
    );
    assert_eq!(
        decode(&wire(Bvll::ReadBroadcastDistributionTable), peer()),
        None
    );
}
#[test]
fn passive_native_ihave_requires_complete_service_and_device_identifier() {
    use bacnet_encoding::{
        apdu::{UnconfirmedRequest, encode_apdu},
        npdu::{Npdu, encode_npdu},
    };
    use bacnet_types::primitives::ObjectIdentifier;
    use bytes::BytesMut;
    let mut service = BytesMut::new();
    IHaveRequest {
        device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 12).unwrap(),
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        object_name: "Supply".into(),
    }
    .encode(&mut service)
    .unwrap();
    let encoded = |body: &[u8]| {
        let mut apdu = BytesMut::new();
        encode_apdu(
            &mut apdu,
            &Apdu::UnconfirmedRequest(UnconfirmedRequest {
                service_choice: UnconfirmedServiceChoice::I_HAVE,
                service_request: Bytes::copy_from_slice(body),
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
        wire(Bvll::OriginalBroadcastNpdu(&npdu))
    };
    assert!(
        matches!(decode(&encoded(&service), peer()), Some(DiscoveryEvent::IHave(row)) if row.name == "Supply" && row.device == 12)
    );
    for length in 0..service.len() {
        assert_eq!(decode(&encoded(&service[..length]), peer()), None);
    }
    service.extend_from_slice(&[0]);
    assert_eq!(decode(&encoded(&service), peer()), None, "trailing member");
    service.truncate(service.len() - 1);
    service[1] = 0; // Device object type becomes Analog Input.
    assert_eq!(decode(&encoded(&service), peer()), None);
}

// Explicit local subnet fixtures; unrelated remote subnet masks remain unknown.
fn decode_on(
    bytes: &[u8],
    peer: SocketAddrV4,
    own: &str,
    broadcast: &str,
) -> Option<DiscoveryEvent> {
    let own: SocketAddrV4 = own.parse().unwrap();
    let groups = bacnet_bip_core::node::Groups::new(
        SocketAddrV4::new(broadcast.parse().unwrap(), own.port()),
        *own.ip(),
    );
    super::decode(bytes, peer, &groups)
}
fn decode(bytes: &[u8], peer: SocketAddrV4) -> Option<DiscoveryEvent> {
    decode_on(bytes, peer, "127.0.0.1:47808", "255.255.255.255")
}
fn observation_payloads() -> Vec<Vec<u8>> {
    vec![vec![1, 0x80, 1, 0, 2], valid_ihave()]
}
#[test]
fn passive_repair_rejects_group_origins_for_router_and_ihave() {
    for npdu in observation_payloads() {
        for ip in ["239.1.2.3", "255.255.255.255"] {
            for port in [47808, 47809] {
                let origin = format!("{ip}:{port}").parse().unwrap();
                assert_eq!(
                    decode(
                        &wire(Bvll::ForwardedNpdu {
                            origin,
                            npdu: &npdu
                        }),
                        peer()
                    ),
                    None,
                    "{origin}"
                );
            }
        }
    }
}
#[test]
fn passive_repair_rejects_group_udp_peers_for_router_and_ihave() {
    for npdu in observation_payloads() {
        for ip in ["239.1.2.3", "255.255.255.255"] {
            for port in [47808, 47809] {
                let source = format!("{ip}:{port}").parse().unwrap();
                for frame in [
                    Bvll::OriginalBroadcastNpdu(&npdu),
                    Bvll::ForwardedNpdu {
                        origin: peer(),
                        npdu: &npdu,
                    },
                ] {
                    assert_eq!(decode(&wire(frame), source), None, "{source}");
                }
            }
        }
    }
}
#[test]
fn passive_repair_known_subnet_broadcast_rejected_without_guessing_remote_masks() {
    for npdu in observation_payloads() {
        for port in [47808, 47809] {
            let group = format!("192.0.2.255:{port}").parse().unwrap();
            assert_eq!(
                decode_on(
                    &wire(Bvll::ForwardedNpdu {
                        origin: group,
                        npdu: &npdu
                    }),
                    peer(),
                    "192.0.2.10:47808",
                    "192.0.2.255"
                ),
                None
            );
            assert_eq!(
                decode_on(
                    &wire(Bvll::OriginalBroadcastNpdu(&npdu)),
                    group,
                    "192.0.2.10:47808",
                    "192.0.2.255"
                ),
                None
            );
        }
        for origin in ["192.0.2.11:47808", "198.51.100.255:47808"] {
            assert!(
                decode_on(
                    &wire(Bvll::ForwardedNpdu {
                        origin: origin.parse().unwrap(),
                        npdu: &npdu
                    }),
                    peer(),
                    "192.0.2.10:47808",
                    "192.0.2.255"
                )
                .is_some(),
                "remote .255 is not a known group"
            );
        }
    }
}

fn valid_ihave() -> Vec<u8> {
    use bacnet_types::primitives::ObjectIdentifier;
    let mut service = bytes::BytesMut::new();
    IHaveRequest {
        device_identifier: ObjectIdentifier::new(ObjectType::DEVICE, 12).unwrap(),
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 3).unwrap(),
        object_name: "Supply".into(),
    }
    .encode(&mut service)
    .unwrap();
    let mut npdu = vec![1, 0, 0x10, UnconfirmedServiceChoice::I_HAVE.to_raw()];
    npdu.extend_from_slice(&service);
    npdu
}

#[test]
fn passive_native_source_admission_preserves_own_interface_and_loopback_exceptions() {
    for npdu in observation_payloads() {
        for own in ["192.0.2.255:47808", "127.0.0.1:47808"] {
            let address: SocketAddrV4 = own.parse().unwrap();
            let ip = address.ip().to_string();
            // Native UDP-source admission exempts this interface even when its
            // broadcast setting equals it. Forwarded origin uses full address.
            assert!(
                decode_on(&wire(Bvll::OriginalBroadcastNpdu(&npdu)), address, own, &ip).is_some()
            );
            let mut origin = address;
            origin.set_port(47809);
            assert!(
                decode_on(
                    &wire(Bvll::ForwardedNpdu {
                        origin,
                        npdu: &npdu
                    }),
                    peer(),
                    own,
                    &ip
                )
                .is_some()
            );
            assert_eq!(
                decode_on(
                    &wire(Bvll::ForwardedNpdu {
                        origin: address,
                        npdu: &npdu
                    }),
                    peer(),
                    own,
                    &ip
                ),
                None
            );
        }
    }
}
