//! `DeviceIdentity`: its defaults and builders, the checks it runs on itself
//! and on a supplied Device, the database it builds, and the registered-port
//! hooks the endpoint drives (#2183).
use super::*;
use crate::analog::AnalogInputObject;
use bacnet_types::enums::EngineeringUnits;
use core::net::{SocketAddrV4, SocketAddrV6};

const LAN: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 10);
const LAN6: Ipv6Addr = Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 0x10);

fn identity() -> DeviceIdentity {
    DeviceIdentity::new(1001, 42).unwrap()
}

/// The `Error::Encoding` message of `result`, or a panic naming what came back.
fn encoding_message<T>(result: Result<T, Error>) -> String {
    match result {
        Err(Error::Encoding(message)) => message,
        Err(other) => panic!("expected an encoding error, got {other:?}"),
        Ok(_) => panic!("expected an encoding error, got success"),
    }
}

fn port_oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, instance).unwrap()
}

#[test]
fn a_new_identity_takes_the_endpoint_defaults() {
    let identity = identity();
    assert_eq!(identity.instance(), 1001);
    assert_eq!(identity.vendor_id(), 42);
    assert_eq!(identity.max_apdu_length(), 1476);
    assert_eq!(identity.segmentation(), Segmentation::NO_SEGMENTATION);
    assert_eq!(identity.apdu_segment_timeout_ms(), 5000);
    assert_eq!(identity.services(), &[ServiceSupported::READ_PROPERTY]);
    assert!(identity.network_ports().is_empty());
    assert_eq!(identity.device_uuid(), [0; 16]);
    assert_eq!(identity.name(), "device-1001");
    assert_eq!(
        identity.device_oid(),
        ObjectIdentifier::new(ObjectType::DEVICE, 1001).unwrap()
    );
    identity.validate().unwrap();

    let default = DeviceIdentity::default();
    assert_eq!((default.instance(), default.vendor_id()), (1, 0));
    assert_eq!(default.name(), "BACnet Device");
}

#[test]
fn the_builders_set_what_they_name() {
    let services = [
        ServiceSupported::READ_PROPERTY,
        ServiceSupported::WRITE_PROPERTY,
    ];
    let identity = identity()
        .with_max_apdu(1474)
        .unwrap()
        .with_segmentation(Segmentation::SEGMENTED_BOTH)
        .with_apdu_segment_timeout_ms(7000)
        .with_services(&services)
        .with_device_uuid([7; 16])
        .with_name("boiler-plant");
    // The raw ceiling is kept, 1474 included.
    assert_eq!(identity.max_apdu_length(), 1474);
    assert_eq!(identity.segmentation(), Segmentation::SEGMENTED_BOTH);
    assert_eq!(identity.apdu_segment_timeout_ms(), 7000);
    assert_eq!(identity.services(), &services);
    assert_eq!(identity.device_uuid(), [7; 16]);
    assert_eq!(identity.name(), "boiler-plant");
    identity.validate().unwrap();
}

#[test]
fn a_receive_ceiling_below_fifty_is_refused() {
    assert!(encoding_message(identity().with_max_apdu(49)).contains("at least 50"));
    assert_eq!(identity().with_max_apdu(50).unwrap().max_apdu_length(), 50);
}

#[test]
fn ports_are_kept_in_order_and_a_duplicate_or_invalid_one_is_refused() {
    let identity = identity()
        .with_bip_port(1, 10, LAN, 47808)
        .unwrap()
        .with_bip6_port(2, 20, LAN6, 47809)
        .unwrap()
        .with_sc_port(3, 30, [1, 2, 3, 4, 5, 6])
        .unwrap();
    let ports = identity.network_ports();
    assert_eq!(
        ports.iter().map(|entry| entry.instance).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(ports[0], NetworkPortEntry::bip(1, 10, LAN, 47808));
    assert_eq!(ports[1], NetworkPortEntry::bip6(2, 20, LAN6, 47809));
    assert_eq!(ports[2], NetworkPortEntry::sc(3, 30, [1, 2, 3, 4, 5, 6]));

    let duplicate = identity.clone().with_sc_port(1, 40, [9; 6]);
    assert!(encoding_message(duplicate).contains("duplicate Network-Port instance 1"));
    // A network number past 16 bits never reaches the list.
    assert!(
        identity
            .clone()
            .with_bip_port(4, 70_000, LAN, 47808)
            .is_err()
    );
    // Nor does a B/IP instance outside the local Port ID range.
    assert!(identity.clone().with_bip_port(256, 1, LAN, 47808).is_err());
    assert_eq!(identity.network_ports().len(), 3);
}

#[test]
fn resolve_takes_the_lower_ceiling_and_leaves_the_request_alone() {
    let requested = identity();
    let resolved = requested.resolve(480).unwrap();
    assert_eq!(resolved.max_apdu_length(), 480);
    assert_eq!(requested.max_apdu_length(), 1476);
    // A larger transport keeps the requested value.
    assert_eq!(requested.resolve(u16::MAX).unwrap().max_apdu_length(), 1476);
    // A transport that can't take 50 octets leaves no valid Device.
    assert!(encoding_message(requested.resolve(49)).contains("at least 50"));
}

#[test]
fn validate_refuses_an_unknown_segmentation_and_a_zero_segment_timeout() {
    let mut unknown = identity();
    unknown.segmentation_supported = Segmentation::from_raw(4);
    assert!(encoding_message(unknown.validate()).contains("segmentation"));

    let zero = identity()
        .with_segmentation(Segmentation::SEGMENTED_TRANSMIT)
        .with_apdu_segment_timeout_ms(0);
    assert!(encoding_message(zero.validate()).contains("APDU_Segment_Timeout"));
    // Without segmentation the timeout isn't served, so zero is fine.
    identity()
        .with_apdu_segment_timeout_ms(0)
        .validate()
        .unwrap();

    // The fields are private to the crate, but a stale one is still caught.
    let mut small = identity();
    small.max_apdu_length = 49;
    assert!(encoding_message(small.validate()).contains("at least 50"));
}

/// The database `identity` builds.
fn built_device(identity: &DeviceIdentity) -> ObjectDatabase {
    identity.build_database().unwrap()
}

#[test]
fn validate_device_accepts_the_devices_own_build_and_names_each_mismatch() {
    let segmented = identity()
        .with_segmentation(Segmentation::SEGMENTED_BOTH)
        .with_apdu_segment_timeout_ms(6000);
    let db = built_device(&segmented);
    let device = db.get(&segmented.device_oid()).unwrap();
    segmented.validate_device(device).unwrap();

    // A Device of another instance.
    let other = DeviceIdentity::new(1002, 42)
        .unwrap()
        .with_segmentation(Segmentation::SEGMENTED_BOTH)
        .with_apdu_segment_timeout_ms(6000);
    assert!(encoding_message(other.validate_device(device)).contains("identifier"));

    for (expected, name) in [
        (segmented.clone().with_name("renamed"), None),
        (
            DeviceIdentity::new(1001, 43)
                .unwrap()
                .with_segmentation(Segmentation::SEGMENTED_BOTH)
                .with_apdu_segment_timeout_ms(6000),
            Some("VENDOR_IDENTIFIER"),
        ),
        (
            segmented.clone().with_max_apdu(480).unwrap(),
            Some("MAX_APDU_LENGTH_ACCEPTED"),
        ),
        (
            segmented
                .clone()
                .with_segmentation(Segmentation::SEGMENTED_TRANSMIT),
            Some("SEGMENTATION_SUPPORTED"),
        ),
        (
            segmented.clone().with_apdu_segment_timeout_ms(6001),
            Some("APDU_Segment_Timeout"),
        ),
    ] {
        match name {
            // The name isn't one of the checked declarations.
            None => expected.validate_device(device).unwrap(),
            Some(name) => {
                let message = encoding_message(expected.validate_device(device));
                assert!(message.contains(name), "{name}: {message}");
            }
        }
    }

    // An invalid identity is refused before the Device is looked at.
    let mut invalid = segmented.clone();
    invalid.apdu_segment_timeout_ms = 0;
    assert!(encoding_message(invalid.validate_device(device)).contains("APDU_Segment_Timeout"));

    // An object that is no Device at all.
    let point = AnalogInputObject::new(1, "ai-1", EngineeringUnits::NO_UNITS).unwrap();
    assert!(encoding_message(segmented.validate_device(&point)).contains("identifier"));
}

#[test]
fn a_device_without_segmentation_is_not_asked_for_a_segment_timeout() {
    let identity = identity();
    let db = built_device(&identity);
    let device = db.get(&identity.device_oid()).unwrap();
    // The stock Device serves no APDU_Segment_Timeout without segmentation,
    // so a differing timeout isn't compared.
    identity
        .clone()
        .with_apdu_segment_timeout_ms(1234)
        .validate_device(device)
        .unwrap();
}

/// The Device's Object_List in `db`.
fn object_list(db: &ObjectDatabase, device: ObjectIdentifier) -> PropertyValue {
    db.get(&device)
        .unwrap()
        .read_property(P::OBJECT_LIST, None)
        .unwrap()
}

#[test]
fn the_database_holds_the_device_and_its_ports_in_object_list_order() {
    let identity = identity()
        .with_services(&[
            ServiceSupported::READ_PROPERTY,
            ServiceSupported::REINITIALIZE_DEVICE,
        ])
        .with_device_uuid([0xA5; 16])
        .with_bip_port(1, 10, LAN, 47808)
        .unwrap()
        .with_sc_port(2, 20, [1, 2, 3, 4, 5, 6])
        .unwrap();
    let db = identity.build_database().unwrap();
    let device_oid = identity.device_oid();
    assert_eq!(db.len(), 3);
    assert_eq!(
        object_list(&db, device_oid),
        PropertyValue::List(
            [device_oid, port_oid(1), port_oid(2)]
                .into_iter()
                .map(PropertyValue::ObjectIdentifier)
                .collect()
        )
    );
    let device = db.get(&device_oid).unwrap();
    assert_eq!(
        device.read_property(P::DEVICE_UUID, None).unwrap(),
        PropertyValue::OctetString(vec![0xA5; 16])
    );
    // The ReadProperty and ReinitializeDevice bits, and no others.
    let PropertyValue::BitString { data, .. } = device
        .read_property(P::PROTOCOL_SERVICES_SUPPORTED, None)
        .unwrap()
    else {
        panic!("Protocol_Services_Supported must be a bit string");
    };
    let set: Vec<usize> = (0..data.len() * 8)
        .filter(|bit| data[bit / 8] & (0x80 >> (bit % 8)) != 0)
        .collect();
    assert_eq!(
        set,
        [
            ServiceSupported::READ_PROPERTY.to_raw() as usize,
            ServiceSupported::REINITIALIZE_DEVICE.to_raw() as usize
        ]
    );
    for instance in [1, 2] {
        let port = db.get(&port_oid(instance)).unwrap();
        assert_eq!(
            port.read_property(P::OBJECT_NAME, None).unwrap(),
            PropertyValue::CharacterString(format!("port-{instance}"))
        );
    }
    identity.validate_device(device).unwrap();
}

#[test]
fn extra_objects_join_the_object_list_after_the_ports() {
    let identity = identity().with_bip_port(1, 10, LAN, 47808).unwrap();
    let point = AnalogInputObject::new(7, "ai-7", EngineeringUnits::NO_UNITS).unwrap();
    let point_oid = point.object_identifier();
    let db = build_database_with_extra(&identity, vec![Box::new(point)]).unwrap();
    let device_oid = identity.device_oid();
    assert_eq!(
        object_list(&db, device_oid),
        PropertyValue::List(
            [device_oid, port_oid(1), point_oid]
                .into_iter()
                .map(PropertyValue::ObjectIdentifier)
                .collect()
        )
    );
    assert!(db.get(&point_oid).is_some());

    // An extra named like a declared port is refused by the database.
    let clash = AnalogInputObject::new(8, "port-1", EngineeringUnits::NO_UNITS).unwrap();
    match build_database_with_extra(&identity, vec![Box::new(clash)]) {
        Err(Error::Remote(bacnet_types::error::RemoteError::Error {
            class: bacnet_types::enums::ErrorClass::PROPERTY,
            code: bacnet_types::enums::ErrorCode::DUPLICATE_NAME,
        })) => {}
        Err(other) => panic!("expected PROPERTY / DUPLICATE_NAME, got {other:?}"),
        Ok(_) => panic!("expected PROPERTY / DUPLICATE_NAME, got a database"),
    }
}

fn assert_identifier_exists(result: Result<ObjectDatabase, Error>) {
    match result {
        Err(Error::Remote(bacnet_types::error::RemoteError::Error {
            class: bacnet_types::enums::ErrorClass::OBJECT,
            code: bacnet_types::enums::ErrorCode::OBJECT_IDENTIFIER_ALREADY_EXISTS,
        })) => {}
        Err(other) => panic!("expected OBJECT_IDENTIFIER_ALREADY_EXISTS, got {other:?}"),
        Ok(_) => panic!("expected OBJECT_IDENTIFIER_ALREADY_EXISTS, got a database"),
    }
}

/// An extra under the Device's identifier would replace the Device (#2250).
#[test]
fn an_extra_reusing_the_device_identifier_is_refused() {
    let identity = identity();
    let twin = DeviceObject::new(DeviceConfig {
        identity: identity.clone().with_name("twin"),
        ..DeviceConfig::default()
    })
    .unwrap();
    assert_identifier_exists(build_database_with_extra(&identity, vec![Box::new(twin)]));
}

/// An extra under a declared port's identifier would replace the port (#2250).
#[test]
fn an_extra_reusing_a_declared_port_identifier_is_refused() {
    let identity = identity().with_bip_port(1, 10, LAN, 47808).unwrap();
    let port = crate::network_port::NetworkPortObject::new_non_bip(
        1,
        "other-port",
        NetworkType::VIRTUAL,
        1,
        bacnet_types::MacAddr::from_slice(&[1]),
        1476,
    )
    .unwrap();
    assert_identifier_exists(build_database_with_extra(&identity, vec![Box::new(port)]));
}

/// Two extras under one identifier would keep only the second (#2250).
#[test]
fn two_extras_sharing_an_identifier_are_refused() {
    let first = AnalogInputObject::new(9, "ai-9-a", EngineeringUnits::NO_UNITS).unwrap();
    let second = AnalogInputObject::new(9, "ai-9-b", EngineeringUnits::NO_UNITS).unwrap();
    assert_identifier_exists(build_database_with_extra(
        &identity(),
        vec![Box::new(first), Box::new(second)],
    ));
}

#[test]
fn a_stale_port_entry_fails_both_database_builders() {
    // with_network_port can't store this entry, so place it by hand.
    let mut identity = identity();
    let mut entry = NetworkPortEntry::bip(1, 10, LAN, 47808);
    entry.udp_port = 47809;
    identity.network_ports.push(entry);
    assert!(encoding_message(identity.build_database()).contains("B/IP MAC"));
    assert!(
        encoding_message(build_database_with_extra(&identity, Vec::new())).contains("B/IP MAC")
    );
}

fn bip_config(network_number: u16, ip: Ipv4Addr, udp_port: u16) -> BipPortConfig {
    BipPortConfig {
        network_number,
        ip_address: ip.octets(),
        udp_port,
        ..Default::default()
    }
}

fn bip6_config(network_number: u16, ip: Ipv6Addr, udp_port: u16) -> Bip6PortConfig {
    Bip6PortConfig {
        network_number,
        ip_address: ip.octets(),
        udp_port,
        ..Default::default()
    }
}

#[test]
fn a_registered_bip_port_must_match_the_database_configuration() {
    let identity = identity()
        .with_bip_port(1, 10, LAN, 47808)
        .unwrap()
        .with_bip6_port(2, 20, LAN6, 47808)
        .unwrap();
    identity
        .validate_registered_bip_internal(port_oid(1), &bip_config(10, LAN, 47808))
        .unwrap();
    for config in [
        bip_config(10, Ipv4Addr::new(10, 0, 0, 1), 47808),
        bip_config(10, LAN, 47809),
        bip_config(11, LAN, 47808),
    ] {
        let message =
            encoding_message(identity.validate_registered_bip_internal(port_oid(1), &config));
        assert!(message.contains("differs"), "{config:?}: {message}");
    }
    // The B/IPv6 entry is the wrong type for a B/IP check.
    assert!(
        encoding_message(
            identity.validate_registered_bip_internal(port_oid(2), &bip_config(20, LAN, 47808))
        )
        .contains("differs")
    );
    assert!(
        encoding_message(
            identity.validate_registered_bip_internal(port_oid(9), &bip_config(10, LAN, 47808))
        )
        .contains("missing")
    );
}

#[test]
fn a_registered_bip6_port_must_match_the_database_configuration() {
    let identity = identity()
        .with_bip_port(1, 10, LAN, 47808)
        .unwrap()
        .with_bip6_port(2, 20, LAN6, 47808)
        .unwrap();
    // The VMAC isn't compared: both sides hold a placeholder.
    let mut config = bip6_config(20, LAN6, 47808);
    config.vmac = [9, 9, 9];
    identity
        .validate_registered_bip6_internal(port_oid(2), &config)
        .unwrap();
    for config in [
        bip6_config(20, Ipv6Addr::LOCALHOST, 47808),
        bip6_config(20, LAN6, 47809),
        bip6_config(21, LAN6, 47808),
    ] {
        let message =
            encoding_message(identity.validate_registered_bip6_internal(port_oid(2), &config));
        assert!(message.contains("differs"), "{config:?}: {message}");
    }
    assert!(
        encoding_message(
            identity.validate_registered_bip6_internal(port_oid(1), &bip6_config(10, LAN6, 47808))
        )
        .contains("differs")
    );
    assert!(
        encoding_message(
            identity.validate_registered_bip6_internal(port_oid(3), &bip6_config(20, LAN6, 47808))
        )
        .contains("missing")
    );
}

#[test]
fn publishing_a_started_port_records_its_bound_address() {
    let mut identity = identity()
        .with_bip_port(1, 10, Ipv4Addr::UNSPECIFIED, 0)
        .unwrap()
        .with_bip6_port(2, 20, Ipv6Addr::UNSPECIFIED, 0)
        .unwrap();
    identity.publish_registered_bip_internal(port_oid(1), SocketAddrV4::new(LAN, 50000));
    identity.publish_registered_bip6_internal(
        port_oid(2),
        SocketAddrV6::new(LAN6, 50001, 0, 0),
        [0xA, 0xB, 0xC],
    );
    let ports = identity.network_ports();
    assert_eq!(ports[0], NetworkPortEntry::bip(1, 10, LAN, 50000));
    assert_eq!(ports[1].ipv6, LAN6.octets());
    assert_eq!(ports[1].udp_port, 50001);
    assert_eq!(ports[1].network_number, 20);
    assert_eq!(ports[1].mac.as_slice(), &[0xA, 0xB, 0xC]);
    // The published entries are what the next checks compare against.
    identity
        .validate_registered_bip_internal(port_oid(1), &bip_config(10, LAN, 50000))
        .unwrap();
    identity
        .validate_registered_bip6_internal(port_oid(2), &bip6_config(20, LAN6, 50001))
        .unwrap();
    let db = identity.build_database().unwrap();
    assert_eq!(
        db.get(&port_oid(2))
            .unwrap()
            .read_property(P::MAC_ADDRESS, None)
            .unwrap(),
        PropertyValue::OctetString(vec![0xA, 0xB, 0xC])
    );
}
