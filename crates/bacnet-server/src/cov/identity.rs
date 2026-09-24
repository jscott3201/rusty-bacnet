use super::*;

/// Exact subscriber identity, including the immediate router for routed traffic.
/// Quota grouping deliberately uses the separate [`CovPeerKey`].
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

/// Multiple notification context. Confirmed and unconfirmed forms are independent.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MultipleContextKey {
    /// Exact subscriber endpoint.
    pub endpoint: SubscriberEndpoint,
    /// Subscriber's process identifier.
    pub process_id: u32,
    /// Multiple notification form, unlike mutable ordinary/Single mode.
    pub confirmed: bool,
}

/// Complete accepted subscription coordinates used by every table operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CovSubscriptionKey {
    /// Ordinary whole-object subscription.
    Object {
        /// Exact transport and routed endpoint.
        endpoint: SubscriberEndpoint,
        /// Subscriber's process identifier.
        process_id: u32,
        /// Monitored object identifier.
        object: ObjectIdentifier,
    },
    /// Single-property subscription. Absent, zero and element indexes are distinct.
    Property {
        /// Exact transport and routed endpoint.
        endpoint: SubscriberEndpoint,
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
    /// The exact transport/routed endpoint, never the quota-only grouping.
    pub fn endpoint(&self) -> &SubscriberEndpoint {
        match self {
            Self::Object { endpoint, .. } | Self::Property { endpoint, .. } => endpoint,
            Self::Multiple { context, .. } => &context.endpoint,
        }
    }

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
}

impl CovSubscription {
    /// Validate and derive the sole table identity from this proposed subscription.
    pub fn key(&self) -> Result<CovSubscriptionKey, Error> {
        let endpoint =
            SubscriberEndpoint::new(&self.subscriber_mac, self.subscriber_network.as_ref());
        let process_id = self.subscriber_process_identifier;
        let object = self.monitored_object_identifier;
        let index = self.monitored_property_array_index;
        match (self.notification_kind, self.monitored_property, index) {
            (CovNotificationKind::Single, None, None) => Ok(CovSubscriptionKey::Object { endpoint, process_id, object }),
            (CovNotificationKind::Single, Some(property), _) => Ok(CovSubscriptionKey::Property { endpoint, process_id, object, property, index }),
            (CovNotificationKind::Multiple, Some(property), _) => Ok(CovSubscriptionKey::Multiple {
                context: MultipleContextKey { endpoint, process_id, confirmed: self.issue_confirmed_notifications }, object, property, index,
            }),
            _ => Err(Error::Encoding("COV reference requires a property; whole-object subscriptions cannot carry an index".into())),
        }
    }
}

/// Immutable accepted entry carried through initial notification and fanout work.
/// Only a table can create it. Renewal/recreation invalidates earlier snapshots;
/// snapshots from another table cannot complete this table's entries.
#[derive(Debug, Clone)]
pub struct CovSubscriptionSnapshot {
    pub(super) key: CovSubscriptionKey,
    pub(super) generation: u64,
    pub(super) owner: Arc<()>,
    pub(super) subscription: CovSubscription,
}

impl CovSubscriptionSnapshot {
    /// Canonical identity captured at acceptance.
    pub fn key(&self) -> &CovSubscriptionKey {
        &self.key
    }
}

impl std::ops::Deref for CovSubscriptionSnapshot {
    type Target = CovSubscription;
    fn deref(&self) -> &Self::Target {
        &self.subscription
    }
}
