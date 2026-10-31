//! Every Network Port kind the crate builds: each conditional row keeps
//! Table 12-71's O code and carries its footnote's condition, and the rows
//! RPM REQUIRED and the PICS treat as required are the same as before the
//! conditions existed (#2191).

use super::*;
use crate::property_metadata::{PropertyConformance, PropertyPresenceCondition as C};
use bacnet_types::bip_port::{BbmdTables, BipPortMode, IpMulticast};
use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
};
use bacnet_types::enums::PropertyIdentifier as P;
use std::sync::Arc;

const UNSET: BACnetHostNPort = BACnetHostNPort {
    host: BACnetHostAddress::None,
    port: 0,
};

/// A BBMD's tables; `Some` NAT state for one that can run behind NAT.
struct Tables(Option<bool>);
impl BbmdTables for Tables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        Vec::new()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        false
    }
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        Vec::new()
    }
    fn nat_traversal(&self) -> Option<bool> {
        self.0
    }
    fn global_address(&self) -> Option<BACnetHostNPort> {
        self.0.map(|_| UNSET)
    }
    fn can_change(&self) -> bool {
        true
    }
}

struct Group;
impl IpMulticast for Group {
    fn group(&self) -> [u8; 4] {
        [239, 255, 94, 1]
    }
    fn can_change(&self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    Normal,
    Foreign,
    Bbmd,
    /// A BBMD that can run behind NAT, and whether it does.
    BbmdNat(bool),
}

#[derive(Clone, Copy, Debug)]
struct Kind {
    v6: bool,
    mode: Mode,
    multicast: bool,
    activation: bool,
}

fn build(kind: Kind) -> NetworkPortObject {
    let mut object = if kind.v6 {
        NetworkPortObject::new_bip6(1, "P", Bip6PortConfig::default()).unwrap()
    } else {
        NetworkPortObject::new_bip(1, "P", BipPortConfig::default()).unwrap()
    };
    let mode = match kind.mode {
        Mode::Normal => BipPortMode::Normal,
        Mode::Foreign => BipPortMode::fixed_foreign(UNSET, 60),
        Mode::Bbmd => BipPortMode::Bbmd {
            tables: Some(Arc::new(Tables(None))),
        },
        Mode::BbmdNat(on) => BipPortMode::Bbmd {
            tables: Some(Arc::new(Tables(Some(on)))),
        },
    };
    object.mode = bip_mode::LiveMode::from_port(mode).unwrap();
    if kind.multicast {
        object.multicast = multicast::Multicast::lent(Some(Arc::new(Group)));
    }
    object.activation = kind.activation;
    object
}

fn kinds() -> Vec<Kind> {
    let mut kinds = Vec::new();
    for v6 in [false, true] {
        for mode in [
            Mode::Normal,
            Mode::Foreign,
            Mode::Bbmd,
            Mode::BbmdNat(false),
            Mode::BbmdNat(true),
        ] {
            for multicast in [false, true] {
                for activation in [false, true] {
                    kinds.push(Kind {
                        v6,
                        mode,
                        multicast,
                        activation,
                    });
                }
            }
        }
    }
    kinds
}

/// The rows a kind serves, each with the condition its Table 12-71
/// footnote gives; `None` for a row whose code is the table's R or a plain
/// O. Independent of the production metadata.
fn oracle(kind: Kind) -> Vec<(P, Option<C>)> {
    let mut rows = vec![
        (P::OBJECT_IDENTIFIER, None),
        (P::OBJECT_NAME, None),
        (P::DESCRIPTION, None),
        (P::OBJECT_TYPE, None),
        (P::STATUS_FLAGS, None),
        (P::OUT_OF_SERVICE, None),
        (P::RELIABILITY, None),
        (P::NETWORK_TYPE, None),
        (P::PROTOCOL_LEVEL, None),
        (P::NETWORK_NUMBER, Some(C::ApplicationProtocolLevel)),
        (P::NETWORK_NUMBER_QUALITY, Some(C::ApplicationProtocolLevel)),
        (P::MAC_ADDRESS, Some(C::NotPointToPoint)),
        (P::APDU_LENGTH, Some(C::ApplicationProtocolLevel)),
        (P::LINK_SPEED, None),
        (P::CHANGES_PENDING, None),
        (P::PROPERTY_LIST, None),
    ];
    if kind.v6 {
        rows.extend([
            (P::BACNET_IPV6_MODE, Some(C::BacnetIpv6Application)),
            (P::IPV6_ADDRESS, Some(C::Ipv6Port)),
            (P::IPV6_PREFIX_LENGTH, Some(C::Ipv6Port)),
            (P::BACNET_IPV6_UDP_PORT, Some(C::BacnetIpv6Application)),
            (P::IPV6_DEFAULT_GATEWAY, Some(C::Ipv6Port)),
            (
                P::BACNET_IPV6_MULTICAST_ADDRESS,
                Some(C::BacnetIpv6Application),
            ),
            (P::IPV6_DNS_SERVER, Some(C::Ipv6Port)),
        ]);
    } else {
        for p in [
            P::BACNET_IP_MODE,
            P::IP_ADDRESS,
            P::IP_DEFAULT_GATEWAY,
            P::IP_SUBNET_MASK,
            P::BACNET_IP_UDP_PORT,
            P::IP_DNS_SERVER,
        ] {
            rows.push((p, Some(C::BacnetIpPort)));
        }
        if kind.multicast {
            rows.push((P::BACNET_IP_MULTICAST_ADDRESS, Some(C::BacnetIpMulticast)));
        }
        if let Mode::BbmdNat(on) = kind.mode {
            rows.push((P::BACNET_IP_NAT_TRAVERSAL, Some(C::NatTraversalCapable)));
            rows.push((
                P::BACNET_IP_GLOBAL_ADDRESS,
                on.then_some(C::NatTraversalConfigured),
            ));
        }
    }
    match kind.mode {
        Mode::Foreign => {
            rows.push((P::FD_BBMD_ADDRESS, Some(C::ForeignDeviceMode)));
            rows.push((P::FD_SUBSCRIPTION_LIFETIME, Some(C::ForeignDeviceMode)));
        }
        Mode::Bbmd | Mode::BbmdNat(_) => {
            for p in [
                P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
                P::BBMD_ACCEPT_FD_REGISTRATIONS,
                P::BBMD_FOREIGN_DEVICE_TABLE,
            ] {
                rows.push((p, Some(C::BbmdCapable)));
            }
        }
        Mode::Normal => {}
    }
    rows
}

#[test]
fn every_port_kind_keeps_the_table_code_and_carries_its_footnote_condition() {
    for kind in kinds() {
        let object = build(kind);
        let metadata = object.property_metadata();
        let mut expected = oracle(kind);
        let mut actual: Vec<_> = metadata
            .iter()
            .map(|row| (row.property_identifier, row.presence_condition))
            .collect();
        expected.sort_by_key(|(p, _)| p.to_raw());
        actual.sort_by_key(|(p, _)| p.to_raw());
        assert_eq!(actual, expected, "{kind:?}");
        for row in metadata.iter() {
            let p = row.property_identifier;
            // The plain optional rows: Description, and the global address
            // of a port not yet behind NAT.
            let plain_optional = p == P::DESCRIPTION
                || (p == P::BACNET_IP_GLOBAL_ADDRESS && row.presence_condition.is_none());
            assert_eq!(
                row.conformance,
                if row.presence_condition.is_some() || plain_optional {
                    PropertyConformance::Optional
                } else {
                    PropertyConformance::RequiredRead
                },
                "{kind:?} {p}"
            );
            // What RPM REQUIRED and the PICS read: every row but the plain
            // optional ones is required, as it was when these rows were R.
            assert_eq!(row.is_required(), !plain_optional, "{kind:?} {p}");
            object.read_property(p, None).unwrap();
        }
    }
}
