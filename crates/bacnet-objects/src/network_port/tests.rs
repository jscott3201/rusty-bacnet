use super::*;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;

// Independent Clause12.56 profile oracle, not copied from production metadata.
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
    P::BACNET_IP_MODE,
    P::IP_ADDRESS,
    P::IP_DEFAULT_GATEWAY,
    P::IP_SUBNET_MASK,
    P::BACNET_IP_UDP_PORT,
    P::IP_DNS_SERVER,
];
fn bip(config: BipPortConfig) -> NetworkPortObject {
    NetworkPortObject::new_bip(1, "NP", config).unwrap()
}
fn error(result: Result<PropertyValue, Error>, expected: ErrorCode) {
    assert!(
        matches!(result, Err(Error::Remote(RemoteError::Error { class, code })) if class == ErrorClass::PROPERTY && code == expected),
        "{result:?}"
    );
}

#[test]
fn configured_bip_contract_requires_application_properties() {
    let object = bip(Default::default());
    for (p, expected) in [
        (P::NETWORK_TYPE, PropertyValue::Enumerated(5)),
        (P::PROTOCOL_LEVEL, PropertyValue::Enumerated(2)),
        (P::BACNET_IP_MODE, PropertyValue::Enumerated(0)),
        (P::APDU_LENGTH, PropertyValue::Unsigned(1476)),
        (P::NETWORK_NUMBER_QUALITY, PropertyValue::Enumerated(0)),
        (P::LINK_SPEED, PropertyValue::Real(0.0)),
        (P::CHANGES_PENDING, PropertyValue::Boolean(false)),
    ] {
        assert_eq!(object.read_property(p, None).unwrap(), expected);
    }
    for p in [P::IP_ADDRESS, P::IP_SUBNET_MASK, P::IP_DEFAULT_GATEWAY] {
        assert_eq!(
            object.read_property(p, None).unwrap(),
            PropertyValue::OctetString(vec![0; 4])
        );
    }
    for p in [P::MAX_APDU_LENGTH_ACCEPTED, P::COMMAND_NP, P::EVENT_STATE] {
        error(object.read_property(p, None), ErrorCode::UNKNOWN_PROPERTY);
    }
}

#[test]
fn configured_bip_metadata_exact_sets_and_indexed_list() {
    let object = bip(Default::default());
    assert_eq!(object.property_list().as_ref(), ALL);
    let metadata = object.property_metadata();
    assert_eq!(metadata.len(), ALL.len() + 1);
    let required: Vec<_> = ALL
        .iter()
        .copied()
        .filter(|p| !matches!(*p, P::DESCRIPTION | P::LINK_SPEED))
        .collect();
    let actual: Vec<_> = object
        .required_properties()
        .iter()
        .copied()
        .filter(|p| *p != P::PROPERTY_LIST)
        .collect();
    assert_eq!(actual, required);
    for row in metadata.iter() {
        assert_eq!(
            row.conformance,
            if matches!(row.property_identifier, P::DESCRIPTION | P::LINK_SPEED) {
                PropertyConformance::Optional
            } else {
                PropertyConformance::RequiredRead
            }
        );
        assert_eq!(
            row.write_capability,
            if matches!(row.property_identifier, P::DESCRIPTION | P::OUT_OF_SERVICE) {
                PropertyWriteCapability::Always
            } else {
                PropertyWriteCapability::ReadOnly
            }
        );
        object.read_property(row.property_identifier, None).unwrap();
    }
    let listed: Vec<_> = ALL
        .iter()
        .filter(|&&p| !matches!(p, P::OBJECT_IDENTIFIER | P::OBJECT_NAME | P::OBJECT_TYPE))
        .map(|p| PropertyValue::Enumerated(p.to_raw()))
        .collect();
    assert_eq!(
        object.read_property(P::PROPERTY_LIST, None).unwrap(),
        PropertyValue::List(listed.clone())
    );
    assert_eq!(
        object.read_property(P::PROPERTY_LIST, Some(0)).unwrap(),
        PropertyValue::Unsigned(18)
    );
    for (i, v) in listed.into_iter().enumerate() {
        assert_eq!(
            object
                .read_property(P::PROPERTY_LIST, Some(i as u32 + 1))
                .unwrap(),
            v
        );
    }
    error(
        object.read_property(P::PROPERTY_LIST, Some(19)),
        ErrorCode::INVALID_ARRAY_INDEX,
    );
    assert!(!object.is_createable());
    assert!(!object.is_deleteable());
}

#[test]
fn configured_bip_numeric_boundaries_and_quality() {
    for instance in [1, 255] {
        NetworkPortObject::new_bip(instance, "NP", Default::default()).unwrap();
    }
    for instance in [0, 256, 4_194_303, u32::MAX] {
        assert!(NetworkPortObject::new_bip(instance, "NP", Default::default()).is_err());
    }
    for (number, quality) in [(0, 0), (1, 3), (65534, 3)] {
        let object = bip(BipPortConfig {
            network_number: number,
            ..Default::default()
        });
        assert_eq!(
            object.read_property(P::NETWORK_NUMBER, None).unwrap(),
            PropertyValue::Unsigned(number.into())
        );
        assert_eq!(
            object
                .read_property(P::NETWORK_NUMBER_QUALITY, None)
                .unwrap(),
            PropertyValue::Enumerated(quality)
        );
    }
    assert!(
        NetworkPortObject::new_bip(
            1,
            "NP",
            BipPortConfig {
                network_number: 65535,
                ..Default::default()
            }
        )
        .is_err()
    );
    for length in [50, 51, 1497, u32::MAX] {
        let object = bip(BipPortConfig {
            apdu_length: length,
            ..Default::default()
        });
        assert_eq!(
            object.read_property(P::APDU_LENGTH, None).unwrap(),
            PropertyValue::Unsigned(length.into())
        );
    }
    for length in [0, 49] {
        assert!(
            NetworkPortObject::new_bip(
                1,
                "NP",
                BipPortConfig {
                    apdu_length: length,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}

#[test]
fn configured_bip_dns_array_and_derived_mac() {
    for (ip, udp) in [
        ([0; 4], 0),
        ([192, 0, 2, 1], 47808),
        ([127, 0, 0, 1], 65535),
    ] {
        let object = bip(BipPortConfig {
            ip_address: ip,
            udp_port: udp,
            ..Default::default()
        });
        let mut mac = ip.to_vec();
        mac.extend_from_slice(&udp.to_be_bytes());
        assert_eq!(
            object.read_property(P::MAC_ADDRESS, None).unwrap(),
            PropertyValue::OctetString(mac)
        );
    }
    for dns in [vec![[0; 4]], vec![[192, 0, 2, 53], [198, 51, 100, 53]]] {
        let object = bip(BipPortConfig {
            dns_servers: dns.clone(),
            ..Default::default()
        });
        assert!(object.is_array_property(P::IP_DNS_SERVER));
        assert_eq!(
            object.read_property(P::IP_DNS_SERVER, Some(0)).unwrap(),
            PropertyValue::Unsigned(dns.len() as u64)
        );
        assert_eq!(
            object.read_property(P::IP_DNS_SERVER, None).unwrap(),
            PropertyValue::List(
                dns.iter()
                    .map(|ip| PropertyValue::OctetString(ip.to_vec()))
                    .collect()
            )
        );
        for (i, ip) in dns.iter().enumerate() {
            assert_eq!(
                object
                    .read_property(P::IP_DNS_SERVER, Some(i as u32 + 1))
                    .unwrap(),
                PropertyValue::OctetString(ip.to_vec())
            );
        }
        for index in [dns.len() as u32 + 1, u32::MAX] {
            error(
                object.read_property(P::IP_DNS_SERVER, Some(index)),
                ErrorCode::INVALID_ARRAY_INDEX,
            );
        }
    }
    assert!(
        NetworkPortObject::new_bip(
            1,
            "NP",
            BipPortConfig {
                dns_servers: vec![],
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn configured_snapshot_refuses_activation_writes_without_state_change() {
    let mut object = bip(Default::default());
    for &p in ALL {
        let before = object.read_property(p, None).unwrap();
        let result = object.write_property(p, None, before.clone(), None);
        if matches!(p, P::DESCRIPTION | P::OUT_OF_SERVICE) {
            result.unwrap();
        } else {
            assert!(
                matches!(result,Err(Error::Remote(RemoteError::Error {code,..})) if code==ErrorCode::WRITE_ACCESS_DENIED),
                "{p:?}"
            );
        }
        assert_eq!(object.read_property(p, None).unwrap(), before);
    }
    for p in [P::COMMAND_NP, P::MAX_APDU_LENGTH_ACCEPTED] {
        assert!(
            matches!(object.write_property(p,None,PropertyValue::Unsigned(1),None),Err(Error::Remote(RemoteError::Error {code,..})) if code==ErrorCode::UNKNOWN_PROPERTY)
        );
    }
    assert_eq!(
        object.read_property(P::CHANGES_PENDING, None).unwrap(),
        PropertyValue::Boolean(false)
    );
    assert!(
        object
            .write_property(P::DESCRIPTION, None, PropertyValue::Unsigned(1), None)
            .is_err()
    );
    assert!(
        object
            .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Unsigned(1), None)
            .is_err()
    );
}

#[test]
fn non_bip_application_rows_do_not_claim_ipv4_fields() {
    for kind in [NetworkType::ETHERNET, NetworkType::VIRTUAL] {
        // B/IP-only instance policy does not constrain other profiles.
        let mut object = NetworkPortObject::new_non_bip(
            0,
            "other",
            kind,
            12,
            MacAddr::from_slice(&[1, 2, 3, 4, 5, 6]),
            51,
        )
        .unwrap();
        let expected: Vec<_> = ALL
            .iter()
            .copied()
            .filter(|p| {
                !matches!(
                    *p,
                    P::BACNET_IP_MODE
                        | P::IP_ADDRESS
                        | P::IP_DEFAULT_GATEWAY
                        | P::IP_SUBNET_MASK
                        | P::BACNET_IP_UDP_PORT
                        | P::IP_DNS_SERVER
                )
            })
            .collect();
        assert_eq!(object.property_list().as_ref(), expected);
        for row in object.property_metadata().iter() {
            object.read_property(row.property_identifier, None).unwrap();
        }
        assert_eq!(
            object.read_property(P::PROTOCOL_LEVEL, None).unwrap(),
            PropertyValue::Enumerated(2)
        );
        assert_eq!(
            object.read_property(P::APDU_LENGTH, None).unwrap(),
            PropertyValue::Unsigned(51)
        );
        for p in [
            P::BACNET_IP_MODE,
            P::IP_ADDRESS,
            P::IP_DEFAULT_GATEWAY,
            P::IP_SUBNET_MASK,
            P::BACNET_IP_UDP_PORT,
            P::IP_DNS_SERVER,
            P::COMMAND_NP,
            P::MAX_APDU_LENGTH_ACCEPTED,
        ] {
            error(object.read_property(p, None), ErrorCode::UNKNOWN_PROPERTY);
            assert!(
                object
                    .write_property(p, None, PropertyValue::Unsigned(1), None)
                    .is_err()
            );
        }
        // The classification follows the datatype, not this object's rows:
        // IP_DNS_SERVER is an array the non-B/IP port doesn't hold.
        assert!(object.is_array_property(P::IP_DNS_SERVER));
        error(
            object.read_property(P::IP_DNS_SERVER, Some(1)),
            ErrorCode::UNKNOWN_PROPERTY,
        );
    }
    assert!(
        NetworkPortObject::new_non_bip(1, "bad", NetworkType::IPV4, 0, MacAddr::new(), 1476)
            .is_err()
    );
}
