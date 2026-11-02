//! COV reporting (Clauses 13.1, 13.14 and 13.15): the subscription table
//! with its confirmed flights and completion tickets, the SubscribeCOV and
//! SubscribeCOVProperty services, who a subscription belongs to and what it
//! monitors, the subscription policy, and the bounded values a
//! subscription keeps of what it last reported.
//!
//! Plain state on `bacnet-time`: no lock, clock or async. A host keeps the
//! [`CovTable`] under its own guard, passes `now` (one monotonic timeline)
//! to every call that can expire something, and after each mutation takes
//! what it fenced or removed ([`CovTable::take_fenced`]) and the counts it
//! made ([`CovTable::take_counts`]). `bacnet-server` runs its table this way.

mod admission;
mod flight;
mod identity;
mod lifetime;
mod observation;
mod observation_order;
mod policy;
mod prepare;
mod sample;
mod subscribe;
mod subscription;
mod table;

pub use admission::MultipleRefusal;
pub use flight::{BeginRefusal, Delivery, FlightTiming, Token};
pub use identity::{
    CovNotificationKind, CovRecipient, CovSubscriptionKey, MultipleContextKey, SubscriberEndpoint,
};
pub use lifetime::CovTimeRemaining;
pub use observation::{CovObservation, validate_flags};
pub use observation_order::{ObservationTicket, PreparedCovCompletion};
pub use policy::{CovCounters, CovPolicy};
pub use prepare::validate_sample;
pub use sample::CovSample;
pub use subscribe::validate_cov_property;
pub use subscription::{CovSubscription, CovSubscriptionSnapshot};
pub use table::{CovTable, Fenced};
