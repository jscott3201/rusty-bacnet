use super::NetworkPortObject;
use crate::property_metadata::{
    PropertyConformance::{Optional, RequiredRead},
    PropertyMetadata,
    PropertyWriteCapability::{Always, ReadOnly},
};
use bacnet_types::enums::{IPMode, PropertyIdentifier as P};
use std::borrow::Cow;

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
// Table 12-71 footnotes 11 and 12: a port that can act as a BBMD has these.
// Clauses 12.56.34 and 12.56.35 make the first two writable in BBMD mode;
// writes wait for an activation owner, so all three are read-only here.
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
// Table 12-71 footnote 13: FOREIGN mode has these. Clauses 12.56.37 and
// 12.56.38 make them writable; read-only here for the same reason.
const FOREIGN: &[PropertyMetadata] = &[
    PropertyMetadata::new(P::FD_BBMD_ADDRESS, RequiredRead, None, ReadOnly),
    PropertyMetadata::new(P::FD_SUBSCRIPTION_LIFETIME, RequiredRead, None, ReadOnly),
];
pub(super) fn for_object(object: &NetworkPortObject) -> Cow<'_, [PropertyMetadata]> {
    if object.bip.is_some() {
        let mode: &[PropertyMetadata] = match object.mode.ip_mode() {
            IPMode::BBMD => BBMD,
            IPMode::FOREIGN => FOREIGN,
            _ => &[],
        };
        Cow::Owned(
            COMMON
                .iter()
                .chain(BIP)
                .chain(mode)
                .copied()
                .map(|mut row| {
                    if row.property_identifier == P::OUT_OF_SERVICE && object.is_bound() {
                        row.write_capability = ReadOnly;
                    }
                    row
                })
                .collect(),
        )
    } else {
        Cow::Borrowed(COMMON)
    }
}
