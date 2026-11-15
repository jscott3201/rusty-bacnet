//! Writes (#2560): command priorities ([`Priority`]), the guard on them
//! ([`WritePolicy`]), what a write sends ([`IntoWriteValue`]) and returns
//! ([`Written`], [`PriorityView`], [`WritePreview`]), and the analysis of
//! what a write at a priority does ([`Commandability`], [`effect`]).
//!
//! The calls are [`Device`](crate::Device)'s:
//!
//! - [`write`](crate::Device::write) sends one WriteProperty, at 16 on a
//!   property the standard makes commandable when no priority is given;
//! - [`relinquish`](crate::Device::relinquish) writes NULL to one slot of
//!   an object's Priority_Array, then reads its priorities back;
//! - [`priority_array`](crate::Device::priority_array) reads them;
//! - [`preview_write`](crate::Device::preview_write) reads what a write
//!   would do, and sends nothing.
//!
//! The library sends what it is told after local checks that cost no
//! traffic: the [`WritePolicy`] (priorities 1, 2, 5 and 6 refused unless
//! allowed), a priority on a property that takes none, NULL where it would
//! relinquish a slot, and a write to Priority_Array itself. Prompts,
//! confirmation tiers, the ledger and the audit log are the `bacnet`
//! tool's (`bacnet-tool`), which plans its writes with [`commands`] and
//! [`effect`] and sends them through the low-level client.

mod command;
mod policy;
mod priority;
mod value;
mod view;

pub(crate) use command::keeps_priorities;
pub use command::{Commandability, Effect, Occupancy, commands, effect};
pub use policy::{WriteDenied, WritePolicy};
pub use priority::{Priority, one_priority};
pub use value::{IntoWriteValue, WriteValue};
pub(crate) use view::{PRIORITY_PROPERTIES, unknown_property};
pub use view::{PriorityView, WritePreview, Written};
