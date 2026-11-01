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
//! [`BACnetObject`]: super::BACnetObject

mod access_events;
mod command_runner;
mod life_safety;
mod local_input;
mod staging_source;
mod timed;

pub use crate::durable::DurableWrites;
#[doc(hidden)]
pub use access_events::AccessEvents;
#[doc(hidden)]
pub use command_runner::CommandRunner;
pub use life_safety::LifeSafety;
pub use local_input::{LocalInput, LocalInputValue};
#[doc(hidden)]
pub use staging_source::StagingSource;
pub(crate) use timed::same_handle;
pub use timed::{Clocks, DeadlineWaker, MonotonicClock, Timed};

/// OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED: what an object answers for
/// an input or operation it doesn't model, and what a host answers for an
/// object that lends no capability for it.
pub fn not_supported() -> bacnet_types::error::Error {
    bacnet_types::error::Error::protocol(
        bacnet_types::enums::ErrorClass::OBJECT,
        bacnet_types::enums::ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    )
}
