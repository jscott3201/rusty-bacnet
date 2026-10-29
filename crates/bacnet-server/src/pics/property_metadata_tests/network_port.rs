use super::*;
use PropertyIdentifier as P;
use bacnet_objects::{network_port::NetworkPortObject, traits::BACnetObject};
use bacnet_types::primitives::PropertyValue;

#[test]
fn pics_network_port_property_metadata_is_exact() {
    // Independent (identifier, optional, writable) rows in declaration order; PICS sorts by property ID.
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, false, false),
        (P::NETWORK_TYPE, false, false),
        (P::PROTOCOL_LEVEL, false, false),
        (P::NETWORK_NUMBER, false, false),
        (P::NETWORK_NUMBER_QUALITY, false, false),
        (P::MAC_ADDRESS, false, false),
        (P::APDU_LENGTH, false, false),
        (P::LINK_SPEED, true, false),
        (P::CHANGES_PENDING, false, false),
        (P::BACNET_IP_MODE, false, false),
        (P::IP_ADDRESS, false, false),
        (P::IP_DEFAULT_GATEWAY, false, false),
        (P::IP_SUBNET_MASK, false, false),
        (P::BACNET_IP_UDP_PORT, false, false),
        (P::IP_DNS_SERVER, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for configured in [false, true] {
        for out_of_service in [false, true] {
            let mut object = NetworkPortObject::new_bip(
                7,
                "NP-7",
                bacnet_objects::network_port::BipPortConfig {
                    ip_address: if configured {
                        [192, 168, 1, 100]
                    } else {
                        [0; 4]
                    },
                    udp_port: if configured { 47809 } else { 47808 },
                    network_number: if configured { 5 } else { 0 },
                    ..Default::default()
                },
            )
            .unwrap();
            if configured {
                object.set_description("long network port label".repeat(100));
            }
            object
                .write_property(
                    P::OUT_OF_SERVICE,
                    None,
                    PropertyValue::Boolean(out_of_service),
                    None,
                )
                .unwrap();
            let required = object.required_properties();
            let mut db = ObjectDatabase::new();
            db.add(Box::new(object)).unwrap();
            let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
            assert_eq!(pics.supported_object_types.len(), 1);
            let support = &pics.supported_object_types[0];
            assert_eq!(support.object_type, ObjectType::NETWORK_PORT);
            assert!(!support.createable);
            assert!(!support.deleteable);
            let rows: Vec<_> = support
                .supported_properties
                .iter()
                .map(|row| {
                    assert!(row.access.readable);
                    (row.property_id, row.access.optional, row.access.writable)
                })
                .collect();
            assert_eq!(
                rows,
                sorted_rows(&expected),
                "configured={configured}, OOS={out_of_service}"
            );
            assert_eq!(
                rows.iter()
                    .filter_map(|&(p, optional, _)| (!optional).then_some(p))
                    .collect::<Vec<_>>(),
                sorted_required(required.as_ref())
            );
        }
    }
}

#[test]
fn pics_non_bip_application_profiles_exclude_ipv4_rows() {
    let expected = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, true),
        (P::RELIABILITY, false, false),
        (P::NETWORK_TYPE, false, false),
        (P::PROTOCOL_LEVEL, false, false),
        (P::NETWORK_NUMBER, false, false),
        (P::NETWORK_NUMBER_QUALITY, false, false),
        (P::MAC_ADDRESS, false, false),
        (P::APDU_LENGTH, false, false),
        (P::LINK_SPEED, true, false),
        (P::CHANGES_PENDING, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    for kind in [
        bacnet_types::enums::NetworkType::ETHERNET,
        bacnet_types::enums::NetworkType::VIRTUAL,
    ] {
        let object =
            NetworkPortObject::new_non_bip(0, "non-B/IP", kind, 0, Default::default(), 1476)
                .unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(object)).unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let rows: Vec<_> = pics.supported_object_types[0]
            .supported_properties
            .iter()
            .map(|row| (row.property_id, row.access.optional, row.access.writable))
            .collect();
        assert_eq!(rows, sorted_rows(&expected));
    }
}

/// A BBMD lending empty tables; the PICS reads only the metadata.
struct NoTables;
impl bacnet_types::bip_port::BbmdTables for NoTables {
    fn broadcast_distribution_table(&self) -> Vec<bacnet_types::constructed::BACnetBDTEntry> {
        Vec::new()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        false
    }
    fn foreign_device_table(&self) -> Vec<bacnet_types::constructed::BACnetFDTEntry> {
        Vec::new()
    }
}

/// The PICS draft for a registered port in FOREIGN or BBMD mode lists that
/// mode's rows (Table 12-71 footnotes 11 to 13) as required and, until an
/// activation path exists, read-only (#939).
#[test]
fn pics_network_port_lists_the_rows_its_bip_mode_adds() {
    use bacnet_types::bip_port::BipPortMode;
    use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
    let normal = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, false),
        (P::RELIABILITY, false, false),
        (P::NETWORK_TYPE, false, false),
        (P::PROTOCOL_LEVEL, false, false),
        (P::NETWORK_NUMBER, false, false),
        (P::NETWORK_NUMBER_QUALITY, false, false),
        (P::MAC_ADDRESS, false, false),
        (P::APDU_LENGTH, false, false),
        (P::LINK_SPEED, true, false),
        (P::CHANGES_PENDING, false, false),
        (P::BACNET_IP_MODE, false, false),
        (P::IP_ADDRESS, false, false),
        (P::IP_DEFAULT_GATEWAY, false, false),
        (P::IP_SUBNET_MASK, false, false),
        (P::BACNET_IP_UDP_PORT, false, false),
        (P::IP_DNS_SERVER, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    let foreign = BipPortMode::fixed_foreign(
        BACnetHostNPort {
            host: BACnetHostAddress::Ip([10, 0, 0, 1].into()),
            port: 47808,
        },
        300,
    );
    let bbmd = BipPortMode::Bbmd {
        tables: Some(std::sync::Arc::new(NoTables)),
    };
    for (mode, added) in [
        (
            foreign,
            &[
                (P::FD_BBMD_ADDRESS, false, false),
                (P::FD_SUBSCRIPTION_LIFETIME, false, false),
            ][..],
        ),
        (
            bbmd,
            &[
                (P::BBMD_BROADCAST_DISTRIBUTION_TABLE, false, false),
                (P::BBMD_ACCEPT_FD_REGISTRATIONS, false, false),
                (P::BBMD_FOREIGN_DEVICE_TABLE, false, false),
            ][..],
        ),
    ] {
        let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
        let mut db = ObjectDatabase::new();
        db.add(Box::new(
            NetworkPortObject::new_bip(
                1,
                "NP-1",
                bacnet_objects::network_port::BipPortConfig {
                    ip_address: [127, 0, 0, 1],
                    ..Default::default()
                },
            )
            .unwrap(),
        ))
        .unwrap();
        let (_, _lease) = db
            .reserve_bip_port_internal(oid, [127, 0, 0, 1], 47808)
            .unwrap();
        db.publish_bip_port_internal(oid, [127, 0, 0, 1], 47808, 1476, mode)
            .unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let rows: Vec<_> = pics.supported_object_types[0]
            .supported_properties
            .iter()
            .map(|row| (row.property_id, row.access.optional, row.access.writable))
            .collect();
        let expected: Vec<_> = normal.iter().chain(added).copied().collect();
        assert_eq!(rows, sorted_rows(&expected));
        let text = pics.generate_text();
        for (property, ..) in added {
            assert!(
                text.contains(&property.to_string()),
                "{property} in PICS text"
            );
        }
    }
}

/// A BBMD's tables and a foreign device's registration that take written
/// values, or don't.
struct Views(bool);
impl bacnet_types::bip_port::BbmdTables for Views {
    fn broadcast_distribution_table(&self) -> Vec<bacnet_types::constructed::BACnetBDTEntry> {
        Vec::new()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        false
    }
    fn foreign_device_table(&self) -> Vec<bacnet_types::constructed::BACnetFDTEntry> {
        Vec::new()
    }
    fn can_change(&self) -> bool {
        self.0
    }
}
impl bacnet_types::bip_port::ForeignRegistration for Views {
    fn bbmd(&self) -> bacnet_types::constructed::BACnetHostNPort {
        bacnet_types::constructed::BACnetHostNPort::from_socket_addr(
            "10.0.0.1:47808".parse().unwrap(),
        )
    }
    fn subscription_lifetime(&self) -> u16 {
        300
    }
    fn can_change(&self) -> bool {
        self.0
    }
}

/// The pending-activation rows are writable in the PICS only when the
/// server activates changes and the transport's view takes them (#1825):
/// a view that can't change, behind a reinitialize handler, stays read-only.
#[test]
fn pics_lists_the_pending_rows_writable_only_when_their_view_can_change() {
    use bacnet_types::bip_port::BipPortMode;
    use std::sync::Arc;
    for can_change in [false, true] {
        for activation in [false, true] {
            let writable = can_change && activation;
            for (mode, rows) in [
                (
                    BipPortMode::Bbmd {
                        tables: Some(Arc::new(Views(can_change))),
                    },
                    [
                        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
                        P::BBMD_ACCEPT_FD_REGISTRATIONS,
                    ],
                ),
                (
                    BipPortMode::Foreign {
                        registration: Arc::new(Views(can_change)),
                    },
                    [P::FD_BBMD_ADDRESS, P::FD_SUBSCRIPTION_LIFETIME],
                ),
            ] {
                let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
                let mut db = ObjectDatabase::new();
                db.add(Box::new(
                    NetworkPortObject::new_bip(
                        1,
                        "NP-1",
                        bacnet_objects::network_port::BipPortConfig {
                            ip_address: [127, 0, 0, 1],
                            ..Default::default()
                        },
                    )
                    .unwrap(),
                ))
                .unwrap();
                let (_, _lease) = db
                    .reserve_bip_port_internal(oid, [127, 0, 0, 1], 47808)
                    .unwrap();
                db.publish_bip_port_internal(oid, [127, 0, 0, 1], 47808, 1476, mode)
                    .unwrap();
                db.set_network_port_activation_internal(oid, activation)
                    .unwrap();
                let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
                let listed = &pics.supported_object_types[0].supported_properties;
                for property in rows {
                    let row = listed
                        .iter()
                        .find(|row| row.property_id == property)
                        .unwrap();
                    assert_eq!(
                        row.access.writable, writable,
                        "{property}, can_change {can_change}, activation {activation}"
                    );
                }
            }
        }
    }
}

/// The PICS draft for a registered B/IPv6 port (#1579) lists the IPV6 rows
/// (Table 12-72) as required and read-only, no IPv4 row, and each mode's
/// own rows, as for B/IP.
#[test]
fn pics_network_port_lists_the_ipv6_rows_in_each_mode() {
    use bacnet_objects::network_port::Bip6PortConfig;
    use bacnet_types::bip_port::BipPortMode;
    use bacnet_types::constructed::BACnetHostNPort;
    let ip = std::net::Ipv6Addr::new(0x2001, 0xDB8, 0, 0, 0, 0, 0, 5);
    let common = [
        (P::OBJECT_IDENTIFIER, false, false),
        (P::OBJECT_NAME, false, false),
        (P::DESCRIPTION, true, true),
        (P::OBJECT_TYPE, false, false),
        (P::STATUS_FLAGS, false, false),
        (P::OUT_OF_SERVICE, false, false),
        (P::RELIABILITY, false, false),
        (P::NETWORK_TYPE, false, false),
        (P::PROTOCOL_LEVEL, false, false),
        (P::NETWORK_NUMBER, false, false),
        (P::NETWORK_NUMBER_QUALITY, false, false),
        (P::MAC_ADDRESS, false, false),
        (P::APDU_LENGTH, false, false),
        (P::LINK_SPEED, true, false),
        (P::CHANGES_PENDING, false, false),
        (P::BACNET_IPV6_MODE, false, false),
        (P::IPV6_ADDRESS, false, false),
        (P::IPV6_PREFIX_LENGTH, false, false),
        (P::BACNET_IPV6_UDP_PORT, false, false),
        (P::IPV6_DEFAULT_GATEWAY, false, false),
        (P::BACNET_IPV6_MULTICAST_ADDRESS, false, false),
        (P::IPV6_DNS_SERVER, false, false),
        (P::PROPERTY_LIST, false, false),
    ];
    let foreign = BipPortMode::fixed_foreign(
        BACnetHostNPort::from_socket_addr(std::net::SocketAddr::new(ip.into(), 47809)),
        300,
    );
    let bbmd = BipPortMode::Bbmd {
        tables: Some(std::sync::Arc::new(NoTables)),
    };
    // A transport whose foreign registration takes changes, behind a
    // ReinitializeDevice handler, lists the two FOREIGN rows writable, as
    // Bip6Transport's does (Clause 12.56.37, 12.56.38).
    let movable = BipPortMode::Foreign {
        registration: std::sync::Arc::new(Views(true)),
    };
    for (mode, activation, added) in [
        (BipPortMode::Normal, false, &[][..]),
        (
            movable,
            true,
            &[
                (P::FD_BBMD_ADDRESS, false, true),
                (P::FD_SUBSCRIPTION_LIFETIME, false, true),
            ][..],
        ),
        (
            foreign,
            false,
            &[
                (P::FD_BBMD_ADDRESS, false, false),
                (P::FD_SUBSCRIPTION_LIFETIME, false, false),
            ][..],
        ),
        (
            bbmd,
            false,
            &[
                (P::BBMD_BROADCAST_DISTRIBUTION_TABLE, false, false),
                (P::BBMD_ACCEPT_FD_REGISTRATIONS, false, false),
                (P::BBMD_FOREIGN_DEVICE_TABLE, false, false),
            ][..],
        ),
    ] {
        let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
        let mut db = ObjectDatabase::new();
        let config = Bip6PortConfig {
            ip_address: ip.octets(),
            ..Default::default()
        };
        db.add(Box::new(
            NetworkPortObject::new_bip6(1, "NP-1", config.clone()).unwrap(),
        ))
        .unwrap();
        let (_, _lease) = db
            .reserve_bip6_port_internal(oid, ip.octets(), 47808)
            .unwrap();
        db.publish_bip6_port_internal(
            oid,
            ip.octets(),
            47808,
            [0, 0, 1],
            config.multicast_address,
            1476,
            mode,
        )
        .unwrap();
        db.set_network_port_activation_internal(oid, activation)
            .unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let rows: Vec<_> = pics.supported_object_types[0]
            .supported_properties
            .iter()
            .map(|row| (row.property_id, row.access.optional, row.access.writable))
            .collect();
        let expected: Vec<_> = common.iter().chain(added).copied().collect();
        assert_eq!(rows, sorted_rows(&expected));
        let text = pics.generate_text();
        for (property, ..) in common.iter().chain(added) {
            assert!(
                text.contains(&property.to_string()),
                "{property} in PICS text"
            );
        }
    }
}

/// A BBMD whose transport can run behind a NAT router, and takes written
/// values or doesn't.
struct NatViews(bool);
impl bacnet_types::bip_port::BbmdTables for NatViews {
    fn broadcast_distribution_table(&self) -> Vec<bacnet_types::constructed::BACnetBDTEntry> {
        Vec::new()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        false
    }
    fn foreign_device_table(&self) -> Vec<bacnet_types::constructed::BACnetFDTEntry> {
        Vec::new()
    }
    fn nat_traversal(&self) -> Option<bool> {
        Some(false)
    }
    fn global_address(&self) -> Option<bacnet_types::constructed::BACnetHostNPort> {
        Some(bacnet_types::constructed::BACnetHostNPort {
            host: bacnet_types::constructed::BACnetHostAddress::None,
            port: 0,
        })
    }
    fn can_change(&self) -> bool {
        self.0
    }
}

/// The PICS draft for an IPv4 BBMD port that can run behind NAT lists
/// BACnet_IP_NAT_Traversal as required and BACnet_IP_Global_Address as
/// optional (Table 12-71, footnotes 9 and 10), both writable when the server
/// activates changes and the transport takes them (Clause 12.56.32,
/// 12.56.33, #940).
#[test]
fn pics_lists_the_nat_rows_of_a_nat_capable_ipv4_bbmd() {
    use bacnet_types::bip_port::BipPortMode;
    use std::sync::Arc;
    for (can_change, activation) in [(false, false), (true, false), (true, true)] {
        let writable = can_change && activation;
        let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
        let mut db = ObjectDatabase::new();
        let config = bacnet_objects::network_port::BipPortConfig {
            ip_address: [127, 0, 0, 1],
            ..Default::default()
        };
        db.add(Box::new(
            NetworkPortObject::new_bip(1, "NP-1", config).unwrap(),
        ))
        .unwrap();
        let (_, _lease) = db
            .reserve_bip_port_internal(oid, [127, 0, 0, 1], 47808)
            .unwrap();
        let mode = BipPortMode::Bbmd {
            tables: Some(Arc::new(NatViews(can_change))),
        };
        db.publish_bip_port_internal(oid, [127, 0, 0, 1], 47808, 1476, mode)
            .unwrap();
        db.set_network_port_activation_internal(oid, activation)
            .unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let listed = &pics.supported_object_types[0].supported_properties;
        for (property, optional) in [
            (P::BACNET_IP_NAT_TRAVERSAL, false),
            (P::BACNET_IP_GLOBAL_ADDRESS, true),
        ] {
            let row = listed
                .iter()
                .find(|row| row.property_id == property)
                .unwrap_or_else(|| panic!("{property} listed"));
            assert_eq!(
                (row.access.optional, row.access.writable),
                (optional, writable),
                "{property}, can_change {can_change}, activation {activation}"
            );
        }
    }
}

/// A transport's B/IP multicast group that takes written values, or
/// doesn't.
struct Group(bool);
impl bacnet_types::bip_port::IpMulticast for Group {
    fn group(&self) -> [u8; 4] {
        [0; 4]
    }
    fn can_change(&self) -> bool {
        self.0
    }
}

/// The PICS draft for an IPv4 port that can use B/IP multicast lists
/// BACnet_IP_Multicast_Address as required (Table 12-71, footnote 7),
/// writable when the server activates changes and the transport takes them
/// (Clause 12.56.26, #941); a port that can't doesn't list it.
#[test]
fn pics_lists_the_multicast_row_of_a_port_that_can_use_multicast() {
    use bacnet_types::bip_port::BipPortMode;
    use std::sync::Arc;
    for (lent, can_change, activation) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, true, true),
    ] {
        let oid = ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap();
        let mut db = ObjectDatabase::new();
        let config = bacnet_objects::network_port::BipPortConfig {
            ip_address: [127, 0, 0, 1],
            ..Default::default()
        };
        db.add(Box::new(
            NetworkPortObject::new_bip(1, "NP-1", config).unwrap(),
        ))
        .unwrap();
        let (_, _lease) = db
            .reserve_bip_port_internal(oid, [127, 0, 0, 1], 47808)
            .unwrap();
        db.publish_bip_port_internal(oid, [127, 0, 0, 1], 47808, 1476, BipPortMode::Normal)
            .unwrap();
        let view = lent
            .then(|| Arc::new(Group(can_change)) as Arc<dyn bacnet_types::bip_port::IpMulticast>);
        db.set_network_port_multicast_internal(oid, view).unwrap();
        db.set_network_port_activation_internal(oid, activation)
            .unwrap();
        let pics = generate_pics(&db, &ServerConfig::default(), &PicsConfig::default());
        let row = pics.supported_object_types[0]
            .supported_properties
            .iter()
            .find(|row| row.property_id == P::BACNET_IP_MULTICAST_ADDRESS);
        let case = format!("lent {lent}, can_change {can_change}, activation {activation}");
        match row {
            Some(row) => {
                assert!(lent, "{case}");
                assert_eq!(
                    (row.access.optional, row.access.writable),
                    (false, can_change && activation),
                    "{case}"
                );
            }
            None => assert!(!lent, "{case}"),
        }
    }
}
