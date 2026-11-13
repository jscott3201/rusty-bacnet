//! COV reporting for a host with nothing to await between steps, such as
//! firmware that owns its database: the [`Reporter`], which strings the
//! staged functions of [`cov`](crate::cov) together the way `bacnet-server`
//! runs them under its guards.
//!
//! The `Reporter` answers SubscribeCOV, SubscribeCOVProperty and, with the
//! `cov-multiple` feature, SubscribeCOVPropertyMultiple itself and
//! hands every other request to its [`Responder`](crate::responder::Responder),
//! reading the Device's subscription lists through an
//! [`ActiveCovOverlay`](crate::cov::active::ActiveCovOverlay). What a request
//! or the host's own change reports goes into a fixed outbox as service
//! parameters, and leaves through [`Reporter::pop`] as a whole APDU: a
//! confirmed one with an invoke ID the host lends ([`InvokeIds`]) and a
//! place in the confirmed table, which matches the answer
//! ([`Reporter::reply`]) and resends or gives up at its deadline
//! ([`Reporter::on_deadline`]). The host reports how each send ended through
//! [`Reporter::delivered`], and each object it deletes through
//! [`Reporter::remove_for_object`]: the `Responder` runs no DeleteObject,
//! so a host that serves it takes the request before
//! [`Reporter::handle`]. The application's own changes go through
//! [`Reporter::apply_local`], or the typed [`Reporter::update`],
//! [`Reporter::set_reliability`], [`Reporter::command`] and
//! [`Reporter::relinquish`] (#2446), which make the change with
//! [`write::local::apply`](crate::write::local::apply) and report it. All
//! time is the host's monotonic `now`.
//!
//! With `cov-multiple` a report to a SubscribeCOVPropertyMultiple context
//! leaves the same way, as one or more [`NotifyService::CovMultiple`]
//! notifications; a context with changes pending adds its
//! Max_Notification_Delay deadline to [`Reporter::next_deadline`] (#2320).
//!
//! With the `event-reporting` feature and a sweep period
//! (`ReportConfig::with_sweep`) the reporter runs intrinsic event
//! reporting too (#2303): the evaluation after each write and change, the
//! sweep at its period and at each Time_Delay's end, AcknowledgeAlarm and
//! GetEventInformation, and the fault pass when configured. Each event
//! notification goes to a Notification Class recipient as a
//! [`NotifyService::Event`] to a [`Destination::Recipient`], through the
//! same outbox and confirmed table as COV, ahead of the COV notifications
//! of the same change; one that finds the outbox full evicts the newest COV
//! notification queued, whose subscription is owed a later look. For each
//! address, no event notification is first sent before an older one: one
//! held for an invoke ID is tried first at every [`Reporter::pop`], and
//! later ones to its address wait behind it. A resend of an older confirmed
//! one can still land after a newer one, each on its own invoke ID, and the
//! receiver orders them by their timestamps. The key is the address alone,
//! whatever the process identifier or confirmed flag, and a Device
//! recipient rebound while one of its events is held keeps no order across
//! the rebinding.
//!
//! With the `event-enrollment` feature and an Event Enrollment period
//! (`ReportConfig::with_event_enrollment`) on top of the sweep, the
//! reporter also runs the Event Enrollment pass at that period (#2319) and
//! sends each transition it commits through the same distribution, outbox
//! and order as an intrinsic one. The pass, and the sweep at its period,
//! follow each enrollment's Event_Algorithm_Inhibit_Ref; a reference to a
//! value that changes between them is seen at the next of the two.
//!
//! While DeviceCommunicationControl disables initiation, keep popping and
//! report each notification [`Delivery::NotSent`](crate::cov::Delivery):
//! a host that stops popping lets event notifications fill the outbox and
//! evict the COV ones.

use bacnet_encoding::npdu::view::NetworkPriority;
#[cfg(feature = "event-reporting")]
use bacnet_time::Duration;
use bacnet_types::constructed::BACnetAddress;
use bacnet_types::enums::{ConfirmedServiceChoice, ServiceSupported};
use bacnet_types::primitives::ObjectIdentifier;

use crate::cov::{CovCounters, CovPolicy, FlightTiming, SubscriberEndpoint, Token};

mod confirmed;
mod outbox;
mod reporter;

pub use outbox::OutboxLimits;
pub use reporter::{LocalValue, Reporter};

/// The services a [`Reporter`] runs, for the Device's
/// Protocol_Services_Supported: the [`Responder`](crate::responder)'s,
/// SubscribeCOV and SubscribeCOVProperty, and with the `cov-multiple`
/// feature SubscribeCOVPropertyMultiple. Build the database from
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
    #[cfg(feature = "cov-multiple")]
    ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
];

/// The services a [`Reporter`] with its event path on
/// ([`ReportConfig::with_sweep`]) runs besides [`SERVICES`]:
/// AcknowledgeAlarm and GetEventInformation, for the Device's
/// Protocol_Services_Supported. ConfirmedEventNotification and
/// UnconfirmedEventNotification are the ones it initiates, which that
/// property doesn't list for a device that only sends them.
#[cfg(feature = "event-reporting")]
pub const EVENT_SERVICES: &[ServiceSupported] = &[
    ServiceSupported::ACKNOWLEDGE_ALARM,
    ServiceSupported::GET_EVENT_INFORMATION,
];

/// Where a notification goes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Destination {
    /// The route a subscription arrived on: send to its `mac`, and when it
    /// came through a router, address the NPDU to its `network` source.
    Subscriber(SubscriberEndpoint),
    /// A Notification Class recipient's address: the one it names, or its
    /// Device's as [`Bindings`] resolved it. Network 0 is the local
    /// network, 0xFFFF a global broadcast, and an empty MAC a broadcast on
    /// that network, as Clause 21 encodes a `BACnetAddress`; the host routes
    /// it.
    Recipient(BACnetAddress),
}

/// Which service a notification is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotifyService {
    /// ConfirmedCOVNotification or UnconfirmedCOVNotification.
    Cov,
    /// ConfirmedCOVNotificationMultiple or
    /// UnconfirmedCOVNotificationMultiple: one notification of a report to
    /// a SubscribeCOVPropertyMultiple context. A build without the
    /// `cov-multiple` feature hands none out.
    CovMultiple,
    /// ConfirmedEventNotification or UnconfirmedEventNotification. A build
    /// without the `event-reporting` feature hands none out.
    Event,
}

impl NotifyService {
    /// The confirmed service a notification of this kind goes out as.
    pub(crate) fn confirmed(self) -> bacnet_types::enums::ConfirmedServiceChoice {
        use bacnet_types::enums::ConfirmedServiceChoice as Choice;
        match self {
            Self::Cov => Choice::CONFIRMED_COV_NOTIFICATION,
            Self::CovMultiple => Choice::CONFIRMED_COV_NOTIFICATION_MULTIPLE,
            Self::Event => Choice::CONFIRMED_EVENT_NOTIFICATION,
        }
    }

    /// The unconfirmed service a notification of this kind goes out as.
    pub(crate) fn unconfirmed(self) -> bacnet_types::enums::UnconfirmedServiceChoice {
        use bacnet_types::enums::UnconfirmedServiceChoice as Choice;
        match self {
            Self::Cov => Choice::UNCONFIRMED_COV_NOTIFICATION,
            Self::CovMultiple => Choice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE,
            Self::Event => Choice::UNCONFIRMED_EVENT_NOTIFICATION,
        }
    }
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
    /// The NPDU's priority: normal for COV; for an event, the one Table
    /// 13-6 gives its event priority.
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
    /// The intrinsic sweep's period, which turns the event path on; `None`,
    /// the default, runs no event reporting (§6.1 of the design). See
    /// [`with_sweep`](Self::with_sweep).
    #[cfg(feature = "event-reporting")]
    pub sweep: Option<Duration>,
    /// The fault pass's period, `None` (the default) for none: each pass
    /// runs every object's reliability evaluation and reports what changed
    /// to COV, as `bacnet-server`'s `enable_fault_detection` does every
    /// 10 s; the sweep then turns a changed Reliability into its FAULT
    /// transition.
    #[cfg(feature = "event-reporting")]
    pub fault_detection: Option<Duration>,
    /// The Event Enrollment pass's period, `None` (the default) for none.
    /// See [`with_event_enrollment`](Self::with_event_enrollment).
    #[cfg(feature = "event-enrollment")]
    pub event_enrollment: Option<Duration>,
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

    /// This configuration with the event path on, sweeping every `period`
    /// (`bacnet-server` sweeps every second). Between sweeps the reporter
    /// also sweeps when a Time_Delay ends, so `period` only sets how often
    /// the standing duties run: FAULT from a Reliability no write reported,
    /// and Event_Algorithm_Inhibit references. It looks at the due instants
    /// after every write and change it is given, so a period longer than
    /// the shortest delay delays no transition the host reported; a FAULT
    /// from a Reliability nobody reported, the fault pass's included, waits
    /// for the next periodic sweep. Whatever `period` and the delays, it
    /// sweeps no sooner than
    /// [`SWEEP_FLOOR`] after its last sweep.
    #[cfg(feature = "event-reporting")]
    pub fn with_sweep(mut self, period: Duration) -> Self {
        self.sweep = Some(period);
        self
    }

    /// This configuration with a fault pass every `period`; see
    /// [`fault_detection`](Self::fault_detection). [`Reporter::new`] refuses
    /// it without [`with_sweep`](Self::with_sweep), which turns its faults
    /// into transitions.
    #[cfg(feature = "event-reporting")]
    pub fn with_fault_detection(mut self, period: Duration) -> Self {
        self.fault_detection = Some(period);
        self
    }

    /// This configuration with an Event Enrollment pass every `period`
    /// (`bacnet-server`'s `event_enrollment_interval_secs`, 10 s by
    /// default): each pass evaluates every Event Enrollment object, commits
    /// what it finds and queues the notifications its transitions send
    /// (#2319). An enrollment sees its monitored value only at a pass, and
    /// a Time_Delay counts passes, rounded up, so it fires no earlier than
    /// the delay and up to one period later. `period` counts in whole
    /// seconds: it is rounded down, to at least one. The first pass runs at
    /// the first [`Reporter::on_deadline`]; a host that calls late gets one
    /// pass, and the next a period after it. [`Reporter::new`] refuses it
    /// without [`with_sweep`](Self::with_sweep), whose event path sends
    /// the notifications.
    #[cfg(feature = "event-enrollment")]
    pub fn with_event_enrollment(mut self, period: Duration) -> Self {
        self.event_enrollment = Some(period);
        self
    }
}

/// The least time from one sweep of a [`Reporter`]'s event path to the
/// next, 100 ms, as `bacnet-server` has it: each sweep walks the whole
/// database twice, and staggered delays could otherwise ask for one a
/// thousand times a second. So a transition goes out up to this long after
/// its Time_Delay ends, and a host sweeps at most ten times a second plus
/// once per period.
#[cfg(feature = "event-reporting")]
pub const SWEEP_FLOOR: Duration = Duration::from_millis(100);

/// Resolves the Device a notification names to an address: an event
/// notification to a Notification Class's Device recipient, at
/// [`Reporter::pop`]. A COV notification goes to its subscriber's route.
/// One the host can't resolve is dropped and counted
/// ([`ReporterCounters::unresolved`]); a host with a Who-Is/I-Am cache
/// resolves from it.
pub trait Bindings {
    /// The address of `device`, if known.
    fn address_of(&self, device: ObjectIdentifier) -> Option<BACnetAddress>;
}

/// The host's invoke IDs, shared with the device's own client requests.
pub trait InvokeIds {
    /// An invoke ID free for a `service` request to `to`, now taken; `None`
    /// when none is. `service` is the confirmed service it will go out as
    /// (ConfirmedCOVNotification, ConfirmedCOVNotificationMultiple or
    /// ConfirmedEventNotification), for
    /// a host that keeps its transactions by service. Hand them out in turn
    /// rather than reusing one just released, as the server's coordinator
    /// does: a late answer to an ended request would otherwise match the
    /// next one sent under its ID. A report refused one is looked at again
    /// as IDs come back, those the reporter gives back itself and those the
    /// host reports with [`Reporter::invoke_ids_released`] while
    /// [`Reporter::owes_invoke_id`] holds, and goes once it can take one; a
    /// host needn't hold its other requests back for it (#2512).
    fn take(&mut self, to: &BACnetAddress, service: ConfirmedServiceChoice) -> Option<u8>;
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
    /// Subscriptions left owed a later look by a report refused for want
    /// of room in the outbox, or of an invoke ID for a confirmed one: one
    /// count until any evaluation, a look's or a change's, reports it or
    /// finds nothing to report, however often it is refused or dropped by
    /// the budget meanwhile. A report that couldn't fit even an empty
    /// outbox, or one input's budget of octets, counts each time, and
    /// leaves nothing owed. A confirmed event notification counts once
    /// when it is first refused an invoke ID, whether it was held for one
    /// or behind an older one to its address.
    pub owed: u64,
    /// Confirmed notifications sent again after a timeout.
    pub resent: u64,
    /// Confirmed notifications that ended unanswered or refused.
    pub failed: u64,
    /// Event notifications whose Device recipient [`Bindings`] couldn't
    /// resolve at [`Reporter::pop`]: each is dropped and counted here.
    pub unresolved: u64,
    /// Event notifications [`Reporter::pop`] handed out.
    pub events: u64,
    /// Event notifications dropped unqueued: the outbox was full with no
    /// COV notification left to evict, or one was too large for it.
    pub events_dropped: u64,
    /// Queued COV notifications an event notification evicted; each left
    /// its subscription owed, as [`owed`](Self::owed) counts.
    pub evicted: u64,
    /// Event Enrollment transitions a pass committed but couldn't send:
    /// their Event Values didn't project or their committed history didn't
    /// read back (`bacnet-server` logs each at debug).
    #[cfg(feature = "event-enrollment")]
    pub enrollments_suppressed: u64,
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
