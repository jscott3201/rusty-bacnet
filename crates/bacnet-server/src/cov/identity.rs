use super::*;

impl CovSubscription {
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
    pub(super) owner: Arc<ObservationOwner>,
    // Zero is the initial marker; issued tickets start at one and never wrap.
    pub(super) last_successful_ticket: u64,
    // Outstanding confirmed report and hold-off of this entry's coordinate
    // (#896): the entry itself for ordinary and Single subscriptions, the whole
    // context for Multiple ones. A Multiple context shares it for one live route
    // incarnation, so a route change fences old snapshots; a same-route refresh
    // keeps it. Private so callers cannot forge completion authority.
    pub(super) flight: super::confirmed::FlightMarker,
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
    pub(crate) fn generation(&self) -> u64 {
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

impl std::ops::Deref for CovSubscriptionSnapshot {
    type Target = CovSubscription;
    fn deref(&self) -> &Self::Target {
        &self.subscription
    }
}
