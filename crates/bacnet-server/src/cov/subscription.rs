//! A proposed COV subscription and the immutable snapshot a table accepts.
use core::ops::Deref;

use bacnet_device_core::cov::{
    CovNotificationKind, CovObservation, CovRecipient, CovSubscriptionKey, MultipleContextKey,
    SubscriberEndpoint,
};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_time::Instant;
use bacnet_types::MacAddr;
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// Proposed COV subscription data. Table acceptance validates its canonical
/// identity and returns an immutable [`CovSubscriptionSnapshot`] for delivery.
#[derive(Debug, Clone)]
pub struct CovSubscription {
    /// Immediate delivery MAC: the local subscriber or its current router.
    pub subscriber_mac: MacAddr,
    /// Routed source address when the subscriber is behind a BACnet router.
    pub subscriber_network: Option<NpduAddress>,
    /// Process identifier chosen by the subscriber.
    pub subscriber_process_identifier: u32,
    /// The object being monitored.
    pub monitored_object_identifier: ObjectIdentifier,
    /// Notification form. Mutable renewal data for ordinary/Single subscriptions;
    /// part of the canonical identity for Multiple references.
    pub issue_confirmed_notifications: bool,
    /// When this subscription expires (None = infinite lifetime), on the
    /// host's monotonic timeline: the instant the table compares with the
    /// `now` each call is given.
    pub expires_at: Option<Instant>,
    /// Last delivered bounded observation, including specialized command fields.
    pub last_notified_observation: Option<CovObservation>,
    /// Monitored property for Single-property and Multiple-reference subscriptions.
    pub monitored_property: Option<PropertyIdentifier>,
    /// Accepted property index; absent, zero and element indexes are independent.
    pub monitored_property_array_index: Option<u32>,
    /// COV increment override (SubscribeCOVProperty only).
    pub cov_increment: Option<f32>,
    /// Notification service family used for this subscription.
    pub notification_kind: CovNotificationKind,
    /// Whether COVNotificationMultiple values should include timeOfChange.
    pub timestamped: bool,
}

impl CovSubscription {
    /// Original client address shared by subscription identity and accounting.
    /// Process, family and monitored coordinates additionally distinguish subscriptions.
    pub fn recipient(&self) -> CovRecipient {
        CovRecipient::from_endpoint(&self.subscriber_mac, self.subscriber_network.as_ref())
    }

    /// Current immediate transport route and original routed source.
    pub fn endpoint(&self) -> SubscriberEndpoint {
        SubscriberEndpoint::new(&self.subscriber_mac, self.subscriber_network.as_ref())
    }

    /// Validate and derive the sole table identity from this proposed subscription.
    pub fn key(&self) -> Result<CovSubscriptionKey, Error> {
        let recipient = self.recipient();
        recipient.validate()?;
        let process_id = self.subscriber_process_identifier;
        let object = self.monitored_object_identifier;
        let index = self.monitored_property_array_index;
        match (self.notification_kind, self.monitored_property, index) {
            (CovNotificationKind::Single, None, None) => Ok(CovSubscriptionKey::Object { recipient, process_id, object }),
            (CovNotificationKind::Single, Some(property), _) => Ok(CovSubscriptionKey::Property { recipient, process_id, object, property, index }),
            (CovNotificationKind::Multiple, Some(property), _) => Ok(CovSubscriptionKey::Multiple {
                context: MultipleContextKey { recipient, process_id, confirmed: self.issue_confirmed_notifications }, object, property, index,
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
    /// The table that accepted it.
    pub(super) table: usize,
    // Zero is the initial marker; issued tickets start at one and never wrap.
    pub(super) last_successful_ticket: u64,
    // The flight this entry's confirmed reports run on (#896): its own for
    // ordinary and Single subscriptions, the whole context's for Multiple
    // ones. A Multiple context keeps one for one live route incarnation, so
    // a route change fences old snapshots; a same-route refresh keeps it.
    // Private so callers cannot forge completion authority.
    pub(super) flight: u64,
    pub(super) subscription: CovSubscription,
    /// Reported maximum notification delay of a Multiple reference; `None`
    /// for ordinary and Single entries. The timed store keeps the context's
    /// current value for its deadline backstop.
    pub(super) max_notification_delay: Option<u32>,
    /// Max-APDU-length-accepted the subscriber of a Multiple reference last
    /// advertised in a SubscribeCOVPropertyMultiple request header, if known.
    pub(super) subscriber_max_apdu: Option<u16>,
}

impl CovSubscriptionSnapshot {
    /// Canonical identity captured at acceptance.
    pub fn key(&self) -> &CovSubscriptionKey {
        &self.key
    }

    /// Table generation of this accepted entry; renewal publishes a new one.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Maximum notification delay reported for a Multiple reference (`None`
    /// for ordinary and Single entries). A table-held entry follows every
    /// refresh of its context; a captured snapshot keeps its acceptance value.
    pub fn max_notification_delay(&self) -> Option<u32> {
        self.max_notification_delay
    }

    /// Maximum APDU the subscriber of a Multiple reference advertised, when
    /// known; its notifications fit the smaller of it and the local maximum.
    /// A table-held entry follows every refresh of its context.
    pub fn subscriber_max_apdu(&self) -> Option<u16> {
        self.subscriber_max_apdu
    }
}

impl Deref for CovSubscriptionSnapshot {
    type Target = CovSubscription;
    fn deref(&self) -> &Self::Target {
        &self.subscription
    }
}
