//! COV reporting for a host with nothing to await between steps, such as
//! firmware that owns its database: the [`Reporter`], which strings the
//! staged functions of [`cov`](crate::cov) together the way `bacnet-server`
//! runs them under its guards.
//!
//! The `Reporter` answers SubscribeCOV and SubscribeCOVProperty itself and
//! hands every other request to its [`Responder`](crate::responder::Responder),
//! reading the Device's subscription lists through an
//! [`ActiveCovOverlay`](crate::cov::active::ActiveCovOverlay). What a request
//! or the host's own change reports goes into a fixed outbox as service
//! parameters, and leaves through [`Reporter::pop`] as a whole APDU: a
//! confirmed one with an invoke ID the host lends ([`InvokeIds`]) and a
//! place in the confirmed table, which matches the answer
//! ([`Reporter::reply`]) and resends or gives up at its deadline
//! ([`Reporter::on_deadline`]). The host reports how each send ended through
//! [`Reporter::delivered`]. All time is the host's monotonic `now`.
//!
//! The COV-multiple services stay with `bacnet-server` until #2320, and
//! event notifications come with #2303's third part: the outbox and the
//! confirmed table take them as further [`NotifyService`] and
//! [`Destination`] variants.

use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::ServiceSupported;
use bacnet_types::primitives::ObjectIdentifier;

use crate::cov::{CovCounters, CovPolicy, FlightTiming, SubscriberEndpoint, Token};

mod confirmed;
mod outbox;
mod reporter;

pub use outbox::OutboxLimits;
pub use reporter::Reporter;

/// The services a [`Reporter`] runs, for the Device's
/// Protocol_Services_Supported: the [`Responder`](crate::responder)'s, and
/// SubscribeCOV and SubscribeCOVProperty. Build the database from
/// `DeviceIdentity::with_services(SERVICES)`, so the Device lists
/// Active_COV_Subscriptions.
pub const SERVICES: &[ServiceSupported] = &[
    ServiceSupported::READ_PROPERTY,
    ServiceSupported::READ_PROPERTY_MULTIPLE,
    ServiceSupported::WRITE_PROPERTY,
    ServiceSupported::WRITE_PROPERTY_MULTIPLE,
    ServiceSupported::WHO_HAS,
    ServiceSupported::WHO_IS,
    ServiceSupported::SUBSCRIBE_COV,
    ServiceSupported::SUBSCRIBE_COV_PROPERTY,
];

/// Where a notification goes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Destination {
    /// The route a subscription arrived on: send to its `mac`, and when it
    /// came through a router, address the NPDU to its `network` source.
    Subscriber(SubscriberEndpoint),
}

/// Which service a notification is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotifyService {
    /// ConfirmedCOVNotification or UnconfirmedCOVNotification.
    Cov,
}

/// One notification [`Reporter::pop`] hands out, its APDU in the caller's
/// buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Notification {
    /// How its send ended comes back with this, through
    /// [`Reporter::delivered`]; a confirmed one's answer through
    /// [`Reporter::reply`].
    pub token: Token,
    /// Where it goes.
    pub to: Destination,
    /// Which service it is.
    pub service: NotifyService,
    /// The invoke ID of a confirmed notification, `None` for an
    /// unconfirmed one.
    pub invoke_id: Option<u8>,
    /// The NPDU's priority: normal for COV.
    pub priority: NetworkPriority,
}

/// What a [`Reporter`] runs with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ReportConfig {
    /// The subscription caps and the per-input notification budget.
    pub cov: CovPolicy,
    /// The outbox's bounds.
    pub outbox: OutboxLimits,
    /// The APDU timeout and retries of confirmed notifications.
    pub confirmed: FlightTiming,
}

impl ReportConfig {
    /// The defaults, with `cov` as the policy.
    pub fn new(cov: CovPolicy) -> Self {
        Self {
            cov,
            ..Self::default()
        }
    }

    /// This configuration with an outbox of `outbox`.
    pub fn with_outbox(mut self, outbox: OutboxLimits) -> Self {
        self.outbox = outbox;
        self
    }

    /// This configuration with confirmed notifications timed by `timing`.
    pub fn with_confirmed(mut self, timing: FlightTiming) -> Self {
        self.confirmed = timing;
        self
    }
}

/// Resolves the Device a notification names to an address. Event
/// notifications to a Notification Class's Device recipients use it, with
/// #2303's third part; a COV notification goes to its subscriber's route.
pub trait Bindings {
    /// The address of `device`, if known.
    fn address_of(&self, device: ObjectIdentifier) -> Option<BACnetAddress>;
}

/// The host's invoke IDs, shared with the device's own client requests.
pub trait InvokeIds {
    /// An invoke ID free for a request to `to`, now taken; `None` when none
    /// is. Hand them out in turn rather than reusing one just released, as
    /// the server's coordinator does: a late answer to an ended request
    /// would otherwise match the next one sent under its ID.
    fn take(&mut self, to: &BACnetAddress) -> Option<u8>;
    /// Give back `id`, taken for `to`: its request has ended.
    fn release(&mut self, to: &BACnetAddress, id: u8);
}

/// What a [`Reporter`] has counted since it was built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReporterCounters {
    /// Subscriptions and notifications: the table's counts, and the
    /// notifications [`Reporter::pop`] handed out.
    pub cov: CovCounters,
    /// Reports refused for want of room in the outbox, or of an invoke ID
    /// for a confirmed one, each leaving its subscription owed a later look
    /// (unless it couldn't fit even an empty outbox).
    pub owed: u64,
    /// Confirmed notifications sent again after a timeout.
    pub resent: u64,
    /// Confirmed notifications that ended unanswered or refused.
    pub failed: u64,
    /// Notifications whose Device recipient [`Bindings`] couldn't resolve.
    /// None yet: COV notifications go to their subscriber's route.
    pub unresolved: u64,
}

/// The address a subscriber's notifications come back from, and that its
/// invoke IDs belong to: its routed source, or its link MAC on the local
/// network (network 0).
pub fn peer_address(endpoint: &SubscriberEndpoint) -> BACnetAddress {
    match &endpoint.network {
        Some(source) => BACnetAddress {
            network_number: source.network,
            mac_address: source.mac_address.clone(),
        },
        None => BACnetAddress {
            network_number: 0,
            mac_address: endpoint.mac.clone(),
        },
    }
}
