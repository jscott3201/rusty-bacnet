//! Capabilities an object can opt into beyond the core [`BACnetObject`]
//! surface (#2179).
//!
//! Each capability is a trait of its own, and the core trait has one
//! accessor for it that defaults to `None`:
//!
//! ```ignore
//! fn timed(&mut self) -> Option<&mut dyn Timed> {
//!     None
//! }
//! ```
//!
//! An object that has the behaviour implements the capability trait and
//! overrides the accessor to return `Some(self)`, or `Some` of the field
//! that holds the behaviour. The host asks the accessor before it drives a
//! capability, so an object without the behaviour costs one call that
//! answers `None`, and its implementer writes nothing for it. A capability's
//! methods also default to no-ops where a no-op is a sound answer, so an
//! object overrides only the parts it uses.
//!
//! Accessors that return a capability borrowed mutably cover hooks the host
//! calls while it holds the object exclusively, such as the server's
//! monotonic pass under the database's write guard. An adapter that wraps
//! another object (an endpoint's source Reporter, a Device read view) hands
//! the accessor on to the object inside, so the wrapped object's own
//! capability answers.
//!
//! A capability the host only drives exclusively has one mutable accessor
//! with the plain name, as [`Timed`] has `timed()`. One the host also reads
//! under a shared guard adds a `&self` accessor: that one takes the plain
//! name, and the mutable one gains a `_mut` suffix.
//!
//! Two capabilities have cargo features (#2215): `event-reporting`
//! ([`EventReporting`], and the Event Enrollment's evaluation) and
//! `durable-writes` ([`DurableWrites`]). With one off, the built-in objects
//! lend nothing through its accessors, so a firmware image links none of
//! the code only those reach. `std` turns both on.
//!
//! [`BACnetObject`]: super::BACnetObject

mod access_events;
mod audit_log;
mod audit_policy;
mod calendar_state;
mod command_runner;
mod cov_reporting;
mod enrollment_eval;
mod event_reporting;
mod life_safety;
mod local_input;
mod log_buffer;
mod reliability_eval;
mod schedule_runner;
mod staging_source;
mod timed;

pub use crate::durable::DurableWrites;
#[doc(hidden)]
pub use access_events::AccessEvents;
#[doc(hidden)]
pub use audit_log::AuditLog;
#[doc(hidden)]
pub use audit_policy::AuditPolicy;
pub(crate) use audit_policy::{impl_audit_policy, lend_audit_policy};
#[doc(hidden)]
pub use calendar_state::CalendarState;
#[doc(hidden)]
pub use command_runner::CommandRunner;
pub(crate) use cov_reporting::lend_cov_reporting;
pub use cov_reporting::{CovReportedProperty, CovReporting};
#[doc(hidden)]
pub use enrollment_eval::EnrollmentEval;
pub(crate) use enrollment_eval::lend_enrollment_eval;
pub(crate) use event_reporting::lend_event_reporting;
pub use event_reporting::{EventReporting, no_alarm_configured};
pub use life_safety::LifeSafety;
pub use local_input::{LocalInput, LocalInputValue};
#[doc(hidden)]
pub use log_buffer::LogBuffer;
pub(crate) use log_buffer::{impl_log_buffer, lend_log_buffer};
pub use reliability_eval::ReliabilityEval;
pub(crate) use reliability_eval::lend_reliability_eval;
pub use schedule_runner::ScheduleRunner;
pub(crate) use schedule_runner::lend_schedule_runner;
#[doc(hidden)]
pub use staging_source::StagingSource;
pub(crate) use timed::same_handle;
pub use timed::{Clocks, DeadlineWaker, MonotonicClock, Retirement, Timed};

/// OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED: what an object answers for
/// an input or operation it doesn't model, and what a host answers for an
/// object that lends no capability for it.
pub fn not_supported() -> bacnet_types::error::Error {
    bacnet_types::error::Error::protocol(
        bacnet_types::enums::ErrorClass::OBJECT,
        bacnet_types::enums::ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    )
}
