//! The B/IPv6 profile (#1579): the IPV6 rows a configured snapshot serves,
//! its validation, and registration and publication through the database.

use super::*;
use crate::database::ObjectDatabase;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use bacnet_types::bip_port::BipPortMode;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use std::net::Ipv6Addr;

// Independent oracle from Tables 12-71 and 12-72, not copied from production
// metadata.
const ALL: &[P] = &[
    P::OBJECT_IDENTIFIER,
    P::OBJECT_NAME,
    P::DESCRIPTION,
    P::OBJECT_TYPE,
    P::STATUS_FLAGS,
    P::OUT_OF_SERVICE,
    P::RELIABILITY,
    P::NETWORK_TYPE,
    P::PROTOCOL_LEVEL,
    P::NETWORK_NUMBER,
    P::NETWORK_NUMBER_QUALITY,
    P::MAC_ADDRESS,
    P::APDU_LENGTH,
    P::LINK_SPEED,
    P::CHANGES_PENDING,
    P::BACNET_IPV6_MODE,
    P::IPV6_ADDRESS,
    P::IPV6_PREFIX_LENGTH,
    P::BACNET_IPV6_UDP_PORT,
    P::IPV6_DEFAULT_GATEWAY,
    P::BACNET_IPV6_MULTICAST_ADDRESS,
    P::IPV6_DNS_SERVER,
];
const IPV4_ROWS: [P; 6] = [
    P::BACNET_IP_MODE,
    P::IP_ADDRESS,
    P::IP_SUBNET_MASK,
    P::IP_DEFAULT_GATEWAY,
    P::BACNET_IP_UDP_PORT,
    P::IP_DNS_SERVER,
];

fn ip(last: u16) -> [u8; 16] {
    Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, last).octets()
}

fn config() -> Bip6PortConfig {
    Bip6PortConfig {
        network_number: 12,
        ip_address: ip(5),
        prefix_length: 48,
        udp_port: 0,
        default_gateway: ip(1),
        dns_servers: vec![ip(0x53), ip(0x54)],
        vmac: [0x0A, 0x0B, 0x0C],
        ..Default::default()
    }
}

fn read(object: &(impl BACnetObject + ?Sized), property: P) -> PropertyValue {
    object.read_property(property, None).unwrap()
}

fn octets(value: &[u8]) -> PropertyValue {
    PropertyValue::OctetString(value.to_vec())
}

fn error_code(result: Result<impl std::fmt::Debug, Error>) -> ErrorCode {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY => {
            code
        }
        other => panic!("expected a property error: {other:?}"),
    }
}

#[test]
fn a_configured_snapshot_serves_the_ipv6_rows_read_only() {
    let mut object = NetworkPortObject::new_bip6(1, "NP6", config()).unwrap();
    assert_eq!(object.property_list().as_ref(), ALL);
    for row in object.property_metadata().iter() {
        let expected = match row.property_identifier {
            P::DESCRIPTION | P::LINK_SPEED => PropertyConformance::Optional,
            _ => PropertyConformance::RequiredRead,
        };
        assert_eq!(row.conformance, expected, "{}", row.property_identifier);
        if ALL[15..].contains(&row.property_identifier) {
            assert_eq!(row.write_capability, PropertyWriteCapability::ReadOnly);
        }
        object.read_property(row.property_identifier, None).unwrap();
    }
    assert_eq!(read(&object, P::NETWORK_TYPE), PropertyValue::Enumerated(9));
    assert_eq!(
        read(&object, P::BACNET_IPV6_MODE),
        PropertyValue::Enumerated(0)
    );
    assert_eq!(read(&object, P::MAC_ADDRESS), octets(&[0x0A, 0x0B, 0x0C]));
    assert_eq!(read(&object, P::IPV6_ADDRESS), octets(&ip(5)));
    assert_eq!(
        read(&object, P::IPV6_PREFIX_LENGTH),
        PropertyValue::Unsigned(48)
    );
    assert_eq!(
        read(&object, P::BACNET_IPV6_UDP_PORT),
        PropertyValue::Unsigned(0)
    );
    assert_eq!(read(&object, P::IPV6_DEFAULT_GATEWAY), octets(&ip(1)));
    // FF05::BAC0, then the UDP port.
    let mut group = Ipv6Addr::new(0xFF05, 0, 0, 0, 0, 0, 0, 0xBAC0)
        .octets()
        .to_vec();
    group.extend_from_slice(&[0, 0]);
    assert_eq!(
        read(&object, P::BACNET_IPV6_MULTICAST_ADDRESS),
        octets(&group)
    );
    assert_eq!(
        read(&object, P::NETWORK_NUMBER),
        PropertyValue::Unsigned(12)
    );
    assert_eq!(
        read(&object, P::IPV6_DNS_SERVER),
        PropertyValue::List(vec![octets(&ip(0x53)), octets(&ip(0x54))])
    );
    assert_eq!(
        object.read_property(P::IPV6_DNS_SERVER, Some(0)).unwrap(),
        PropertyValue::Unsigned(2)
    );
    assert_eq!(
        object.read_property(P::IPV6_DNS_SERVER, Some(2)).unwrap(),
        octets(&ip(0x54))
    );
    assert_eq!(
        error_code(object.read_property(P::IPV6_DNS_SERVER, Some(3))),
        ErrorCode::INVALID_ARRAY_INDEX
    );
    for property in IPV4_ROWS {
        assert_eq!(
            error_code(object.read_property(property, None)),
            ErrorCode::UNKNOWN_PROPERTY,
            "{property}"
        );
    }
    for &property in &ALL[15..] {
        let before = read(&object, property);
        assert_eq!(
            error_code(object.write_property(property, None, before.clone(), None)),
            ErrorCode::WRITE_ACCESS_DENIED,
            "{property}"
        );
        assert_eq!(read(&object, property), before);
    }
    assert_eq!(
        read(&object, P::CHANGES_PENDING),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn the_snapshot_refuses_a_configuration_outside_the_profile() {
    assert!(NetworkPortObject::new_bip6(1, "ok", Bip6PortConfig::default()).is_ok());
    for instance in [0, 256] {
        assert!(NetworkPortObject::new_bip6(instance, "bad", config()).is_err());
    }
    for bad in [
        Bip6PortConfig {
            prefix_length: 0,
            ..config()
        },
        Bip6PortConfig {
            prefix_length: 129,
            ..config()
        },
        Bip6PortConfig {
            multicast_address: ip(9),
            ..config()
        },
        Bip6PortConfig {
            dns_servers: Vec::new(),
            ..config()
        },
        Bip6PortConfig {
            apdu_length: 49,
            ..config()
        },
        Bip6PortConfig {
            network_number: u16::MAX,
            ..config()
        },
    ] {
        assert!(
            NetworkPortObject::new_bip6(1, "bad", bad.clone()).is_err(),
            "{bad:?}"
        );
    }
}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

fn database() -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip6(1, "NP6", config()).unwrap(),
    ))
    .unwrap();
    db
}

#[test]
fn registration_publishes_the_bound_port_vmac_and_group() {
    let mut db = database();
    // The other IP version, or another address, is refused.
    assert!(
        db.reserve_bip_port_internal(oid(), [127, 0, 0, 1], 0)
            .is_err()
    );
    assert!(db.reserve_bip6_port_internal(oid(), ip(6), 0).is_err());
    assert!(db.reserve_bip6_port_internal(oid(), ip(5), 47808).is_err());
    let (configured, lease) = db.reserve_bip6_port_internal(oid(), ip(5), 0).unwrap();
    assert_eq!(configured, config());
    let group = Ipv6Addr::new(0xFF02, 0, 0, 0, 0, 0, 0, 0xBAC0).octets();
    let bind = Bip6Bind {
        ip: ip(5),
        udp: 47809,
        vmac: [0x00, 0x04, 0xD2],
        multicast: group,
    };
    // Port zero, another address and a B/IP bind can't be published.
    for refused in [Bip6Bind { udp: 0, ..bind }, Bip6Bind { ip: ip(6), ..bind }] {
        assert!(
            db.publish_bip6_port_internal(
                oid(),
                refused.ip,
                refused.udp,
                refused.vmac,
                refused.multicast,
                1476,
                BipPortMode::Normal
            )
            .is_err()
        );
    }
    assert!(
        db.publish_bip_port_internal(oid(), [127, 0, 0, 1], 47809, 1476, BipPortMode::Normal)
            .is_err()
    );
    db.publish_bip6_port_internal(
        oid(),
        bind.ip,
        bind.udp,
        bind.vmac,
        bind.multicast,
        1400,
        BipPortMode::Normal,
    )
    .unwrap();
    assert_eq!(db.registered_port_internal(), Some(oid()));
    let object = db.get(&oid()).unwrap();
    assert_eq!(read(object, P::MAC_ADDRESS), octets(&[0x00, 0x04, 0xD2]));
    assert_eq!(
        read(object, P::BACNET_IPV6_UDP_PORT),
        PropertyValue::Unsigned(47809)
    );
    assert_eq!(read(object, P::APDU_LENGTH), PropertyValue::Unsigned(1400));
    let mut multicast = group.to_vec();
    multicast.extend_from_slice(&47809u16.to_be_bytes());
    assert_eq!(
        read(object, P::BACNET_IPV6_MULTICAST_ADDRESS),
        octets(&multicast)
    );
    // The configured rows the bind doesn't carry stay as configured.
    assert_eq!(
        read(object, P::IPV6_PREFIX_LENGTH),
        PropertyValue::Unsigned(48)
    );
    assert_eq!(read(object, P::NETWORK_NUMBER), PropertyValue::Unsigned(12));
    // Published once only.
    assert!(
        db.publish_bip6_port_internal(
            oid(),
            bind.ip,
            bind.udp,
            bind.vmac,
            bind.multicast,
            1476,
            BipPortMode::Normal
        )
        .is_err()
    );
    drop(lease);
    assert_eq!(db.registered_port_internal(), None);
}

#[test]
fn a_b_ip_object_refuses_a_b_ipv6_reservation() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "NP", BipPortConfig::default()).unwrap(),
    ))
    .unwrap();
    assert!(db.reserve_bip6_port_internal(oid(), ip(5), 0).is_err());
}
