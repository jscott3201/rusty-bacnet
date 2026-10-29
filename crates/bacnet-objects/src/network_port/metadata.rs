use super::{IpConfig, NetworkPortObject};
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};
use alloc::borrow::Cow;
use bacnet_types::enums::{IPMode, PropertyIdentifier as P};

// 135-2020 Table12-71 incl application footnote25; Link_Speed optional per
// 2024-04-29 errata item23. Readonly configured snapshot: no activation owner.
const COMMON: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::OBJECT_IDENTIFIER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OBJECT_NAME, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::DESCRIPTION, Optional, None, Always),
    PropertyMetadata::new(P::OBJECT_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::STATUS_FLAGS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::OUT_OF_SERVICE, RequiredRead, None, Always),
    PropertyMetadata::new(P::RELIABILITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NETWORK_TYPE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROTOCOL_LEVEL, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NETWORK_NUMBER, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::NETWORK_NUMBER_QUALITY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::MAC_ADDRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::APDU_LENGTH, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::LINK_SPEED, Optional, None, ReadOnly),
    PropertyMetadata::new(P::CHANGES_PENDING, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::PROPERTY_LIST, RequiredRead, None, ReadOnly),
];
const BIP: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::BACNET_IP_MODE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::IP_ADDRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::IP_DEFAULT_GATEWAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::IP_SUBNET_MASK, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::BACNET_IP_UDP_PORT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::IP_DNS_SERVER, RequiredRead, None, ReadOnly),
];
// Table 12-71 footnotes 14 and 15, and Table 12-72's IPV6 rows: a B/IPv6
// application port has its mode, address, prefix length, UDP port, gateway,
// multicast group and DNS servers. They are read-only, as the B/IP rows
// are: this object has no activation owner for its addresses. Left out:
// IPv6_Auto_Addressing_Enable (footnote 16: only for a port that assigns
// its own address, which this one doesn't), IPv6_Zone_Index (footnote 17:
// only for a node with several link-local addresses; registration refuses
// link-local), and the DHCPv6 rows, which are plain optional.
const BIP6: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::BACNET_IPV6_MODE, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::IPV6_ADDRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::IPV6_PREFIX_LENGTH, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::BACNET_IPV6_UDP_PORT, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::IPV6_DEFAULT_GATEWAY, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(
        P::BACNET_IPV6_MULTICAST_ADDRESS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(P::IPV6_DNS_SERVER, RequiredRead, None, ReadOnly),
];
// Table 12-71 footnotes 11 and 12 (IPV4 or IPV6): a port that can act as a BBMD has these;
// the informative Table 12-72 lists them for BBMD mode. A port counts as
// able to only while configured as a BBMD, since its mode is fixed while
// registered. Clauses 12.56.34 and 12.56.35 make the first two writable in
// BBMD mode: a write waits, with Changes_Pending set, until a
// ReinitializeDevice activates it (#1578). They are listed read-only here
// and become writable only for an owner that activates changes.
const BBMD: &[PropertyMetadata] = &[
    PropertyMetadata::new(
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(
        P::BBMD_ACCEPT_FD_REGISTRATIONS,
        RequiredRead,
        None,
        ReadOnly,
    ),
    PropertyMetadata::new(P::BBMD_FOREIGN_DEVICE_TABLE, RequiredRead, None, ReadOnly),
];
// Table 12-71 footnotes 9 and 10 (IPV4 only): a port that can run behind a
// NAT router (Annex J.7.5) has BACnet_IP_NAT_Traversal, and one configured
// to has BACnet_IP_Global_Address. This stack's BBMD can, so an IPv4 BBMD
// port whose transport says so serves both, the address as the unset
// sentinel while none is configured; the address is listed required while
// NAT traversal is on, optional otherwise. Clauses 12.56.32 and 12.56.33 make
// them writable (the address because this stack never discovers it), with
// a write waiting for a ReinitializeDevice as the BBMD rows' do (#940).
// They are listed read-only here and become writable only for an owner
// that activates changes.
const NAT: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::BACNET_IP_NAT_TRAVERSAL, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::BACNET_IP_GLOBAL_ADDRESS, Optional, None, ReadOnly),
];
// Table 12-71 footnote 7 (IPV4 only): a port that supports B/IP multicast
// (Annex J.8) has BACnet_IP_Multicast_Address, in any mode, so an IPv4 port
// whose transport lends its group serves it, as 0.0.0.0 while off. Clause
// 12.56.26 makes it writable whenever present, a write waiting for a
// ReinitializeDevice as the mode rows' do (#941). It is listed read-only
// here and becomes writable only for an owner that activates changes: a
// deviation, as for the BBMD rows (#1578), since without one a written
// group could never take effect. The PICS draft shows it read-only then.
// On a BBMD the group goes in with the BBMD rows, in one activation
// (Annex J.8.3 ties it to the BBMD's own BDT row, #1963).
const MULTICAST: &[PropertyMetadata] = &[PropertyMetadata::new(
    P::BACNET_IP_MULTICAST_ADDRESS,
    RequiredRead,
    None,
    ReadOnly,
)];
// Table 12-71 footnote 13 and Table 12-72: FOREIGN mode has these, for
// either IP version. Clauses
// 12.56.37 and 12.56.38 make them writable, with a write waiting for a
// ReinitializeDevice as the BBMD rows' do (#1825). They are listed read-only
// here and become writable only for an owner that activates changes.
const FOREIGN: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::FD_BBMD_ADDRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::FD_SUBSCRIPTION_LIFETIME, RequiredRead, None, ReadOnly),
];
pub(super) fn for_object(object: &NetworkPortObject) -> Cow<'_, [PropertyMetadata]> {
    if let Some(config) = &object.ip {
        let profile = match config {
            IpConfig::V4(_) => BIP,
            IpConfig::V6(_) => BIP6,
        };
        let mode: &[PropertyMetadata] = match object.mode.ip_mode() {
            IPMode::BBMD => BBMD,
            IPMode::FOREIGN => FOREIGN,
            _ => &[],
        };
        let nat: &[PropertyMetadata] = if object.serves_nat() { NAT } else { &[] };
        let multicast: &[PropertyMetadata] = if object.serves_multicast() {
            MULTICAST
        } else {
            &[]
        };
        Cow::Owned(
            COMMON
                .iter()
                .chain(profile)
                .chain(multicast)
                .chain(nat)
                .chain(mode)
                .copied()
                .map(|mut row| {
                    if row.property_identifier == P::OUT_OF_SERVICE && object.is_bound() {
                        row.write_capability = ReadOnly;
                    }
                    // Footnote 10: required once the port runs behind NAT.
                    if row.property_identifier == P::BACNET_IP_GLOBAL_ADDRESS
                        && object.mode.nat_on()
                    {
                        row.conformance = RequiredRead;
                    }
                    if object.activates_changes()
                        && super::pending::writable_pending_row(row.property_identifier)
                    {
                        row.write_capability = Always;
                    }
                    if row.property_identifier == P::BACNET_IP_MULTICAST_ADDRESS
                        && object.activates_multicast()
                    {
                        row.write_capability = Always;
                    }
                    row
                })
                .collect(),
        )
    } else {
        Cow::Borrowed(COMMON)
    }
}
