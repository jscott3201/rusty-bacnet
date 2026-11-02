//! Who a COV subscription belongs to and what it monitors: the delivery
//! route, the original client address, and the coordinates every table
//! operation keys on.
use alloc::format;
use core::cmp::Ordering;

use bacnet_encoding::npdu::NpduAddress;
use bacnet_types::MacAddr;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// Delivery endpoint, including the immediate router for routed traffic.
/// All COV family keys use the original client recipient as identity.
/// Quota and notification accounting use that same [`CovRecipient`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriberEndpoint {
    /// Immediate transport destination (the router when routed).
    pub mac: MacAddr,
    /// Original routed NPDU source, if any.
    pub network: Option<NpduAddress>,
}

impl SubscriberEndpoint {
    /// Capture both parts of the admitted transport endpoint.
    pub fn new(mac: &[u8], network: Option<&NpduAddress>) -> Self {
        Self {
            mac: MacAddr::from_slice(mac),
            network: network.cloned(),
        }
    }
}

/// Original BACnet client address for COV identity, quota and notification accounting.
/// A routed address is independent of the immediate router; it is a claimed
/// protocol address, not an authentication credential.
///
/// Recipients order direct before routed, then by address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CovRecipient {
    /// A local-network client, identified by its source MAC.
    Direct(MacAddr),
    /// The original SNET/SADR of a client behind a router.
    Routed(NpduAddress),
}

impl CovRecipient {
    /// Derive client identity separately from the current delivery route.
    pub fn from_endpoint(mac: &[u8], network: Option<&NpduAddress>) -> Self {
        match network {
            Some(source) => Self::Routed(source.clone()),
            None => Self::Direct(MacAddr::from_slice(mac)),
        }
    }
    /// Table admission requires the same nonempty routed source MAC as NPDU
    /// decoding. The address either form reports in the Device's COV lists
    /// must also fit [`BACnetAddress::MAX_MAC_LEN`] octets, so every admitted
    /// subscription encodes there (#1156). Off the wire both forms already
    /// fit: the NPDU codec bounds a routed source (#1141) and the network
    /// layer drops a frame from a longer link-layer MAC (#1198). The check
    /// stays for subscriptions that reach the public table API another way.
    pub fn validate(&self) -> Result<(), Error> {
        if matches!(self, Self::Routed(source) if source.mac_address.is_empty()) {
            return Err(Error::Encoding(
                "COV routed recipient requires a nonempty source MAC".into(),
            ));
        }
        let mac = match self {
            Self::Direct(mac) => mac,
            Self::Routed(source) => &source.mac_address,
        };
        if mac.len() > BACnetAddress::MAX_MAC_LEN {
            return Err(Error::Encoding(format!(
                "COV recipient MAC of {} octets exceeds the {}-octet BACnetAddress limit",
                mac.len(),
                BACnetAddress::MAX_MAC_LEN
            )));
        }
        Ok(())
    }
}

/// Multiple notification context. Confirmed and unconfirmed forms are independent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MultipleContextKey {
    /// Original client address, without the immediate router.
    pub recipient: CovRecipient,
    /// Subscriber's process identifier.
    pub process_id: u32,
    /// Multiple notification form, unlike mutable ordinary/Single mode.
    pub confirmed: bool,
}

/// Complete accepted subscription coordinates used by every table operation.
///
/// Keys order by monitored object first, so a table finds an object's
/// subscriptions together; then ordinary before Single before Multiple, and
/// by recipient, process, form, property and index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CovSubscriptionKey {
    /// Ordinary whole-object subscription.
    Object {
        /// Original client address, without the immediate router.
        recipient: CovRecipient,
        /// Subscriber's process identifier.
        process_id: u32,
        /// Monitored object identifier.
        object: ObjectIdentifier,
    },
    /// Single-property subscription. Absent, zero and element indexes are distinct.
    Property {
        /// Original client address, without the immediate router.
        recipient: CovRecipient,
        /// Subscriber's process identifier.
        process_id: u32,
        /// Monitored object identifier.
        object: ObjectIdentifier,
        /// Monitored property identifier.
        property: PropertyIdentifier,
        /// Optional array coordinate; absence and zero remain distinct.
        index: Option<u32>,
    },
    /// A property reference within a Multiple context.
    Multiple {
        /// Exact Multiple context, including notification form.
        context: MultipleContextKey,
        /// Monitored object identifier.
        object: ObjectIdentifier,
        /// Monitored property identifier.
        property: PropertyIdentifier,
        /// Optional array coordinate; absence and zero remain distinct.
        index: Option<u32>,
    },
}

impl CovSubscriptionKey {
    /// Monitored object shared by all three subscription families.
    pub fn object(&self) -> ObjectIdentifier {
        match self {
            Self::Object { object, .. }
            | Self::Property { object, .. }
            | Self::Multiple { object, .. } => *object,
        }
    }

    /// Matching Multiple context, when this is a Multiple reference.
    pub fn multiple_context(&self) -> Option<&MultipleContextKey> {
        match self {
            Self::Multiple { context, .. } => Some(context),
            _ => None,
        }
    }

    /// Every coordinate but the object, in comparison order. Each field of
    /// each family appears, so two keys compare equal exactly when `Eq`
    /// says they are.
    #[allow(clippy::type_complexity)]
    fn rank(
        &self,
    ) -> (
        u8,
        &CovRecipient,
        u32,
        bool,
        Option<PropertyIdentifier>,
        Option<u32>,
    ) {
        match self {
            Self::Object {
                recipient,
                process_id,
                ..
            } => (0, recipient, *process_id, false, None, None),
            Self::Property {
                recipient,
                process_id,
                property,
                index,
                ..
            } => (1, recipient, *process_id, false, Some(*property), *index),
            Self::Multiple {
                context,
                property,
                index,
                ..
            } => (
                2,
                &context.recipient,
                context.process_id,
                context.confirmed,
                Some(*property),
                *index,
            ),
        }
    }
}

impl Ord for CovSubscriptionKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.object()
            .cmp(&other.object())
            .then_with(|| self.rank().cmp(&other.rank()))
    }
}

impl PartialOrd for CovSubscriptionKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// COV notification service family for a stored subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CovNotificationKind {
    /// COVNotification / ConfirmedCOVNotification.
    Single,
    /// COVNotificationMultiple / ConfirmedCOVNotificationMultiple.
    Multiple,
}
