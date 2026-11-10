//! COV reporting (Clauses 13.1, 13.14 and 13.15): the subscription table
//! with its confirmed flights and completion tickets, the SubscribeCOV and
//! SubscribeCOVProperty services, who a subscription belongs to and what it
//! monitors, the subscription policy, the bounded values a subscription
//! keeps of what it last reported, the values a notification carries
//! ([`prepare`], [`flags`], [`reported`], [`value_source`], [`life_safety`]),
//! what a change of an object reports ([`evaluate()`], within an
//! [`EventBudget`]) and the Device's two subscription lists ([`active`]).
//!
//! Plain state on `bacnet-time`: no lock, clock or async. A host keeps the
//! [`CovTable`] under its own guard, passes `now` (one monotonic timeline)
//! to every call that can expire something, and after each mutation takes
//! what it fenced or removed ([`CovTable::take_fenced`]) and the counts it
//! made ([`CovTable::take_counts`]). `bacnet-server` runs its table this way.

pub mod active;
mod admission;
mod budget;
mod evaluate;
pub mod flags;
mod flight;
mod identity;
pub mod life_safety;
mod lifetime;
#[cfg(feature = "cov-multiple")]
pub mod multiple_chunks;
#[cfg(feature = "cov-multiple")]
pub mod multiple_items;
#[cfg(feature = "cov-multiple")]
pub mod multiple_reads;
mod observation;
mod observation_order;
mod policy;
pub mod prepare;
pub mod reported;
mod sample;
mod subscribe;
mod subscription;
mod table;
#[cfg(feature = "cov-multiple")]
pub mod timed;
pub mod value_source;
mod vec_map;

pub use admission::MultipleRefusal;
pub use budget::EventBudget;
pub use evaluate::{CovReport, Evaluation, evaluate};
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
