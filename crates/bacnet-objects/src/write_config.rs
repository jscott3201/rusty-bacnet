//! Keeping the configuration peers write to a Timer or a Channel across a
//! restart (#2243).
//!
//! A head end sets up a Timer's writes by writing State_Change_Values and
//! List_Of_Object_Property_References (Clauses 12.57.20 and 12.57.21), its
//! Default_Timeout (Clause 12.57.16, #2312), and a
//! Channel's by writing its members, Execution_Delay, Channel_Number,
//! Control_Groups and Allow_Group_Delay_Inhibit (Clauses 12.53.11 to
//! 12.53.15). Clause 12 says nothing about restarts, so keeping them is
//! this stack's choice, as for the Access Rights rules
//! ([`AccessRightsPersistence`]): a head end that configured a device once
//! expects the configuration to stay.
//!
//! One trait, [`WriteConfigPersistence`], and one file format,
//! `FileWriteConfigPersistence` (with std), serve both object types: a
//! [`WriteConfigSnapshot`] is tagged with the type it belongs to. The
//! storage belongs to the application. An object built with
//! `with_persistence` (on [`TimerObject`] or [`ChannelObject`]) loads it
//! once, and saves on its own writer ([`crate::durable`]) each time a write
//! changes a part it keeps. The bundled server stages such a write: the save
//! runs with the database guard dropped, and a write that cannot be saved
//! is refused with DEVICE / OPERATIONAL_PROBLEM, leaving the old value. A
//! write nobody staged saves where it is.
//!
//! Storage keeps only the parts a write has set; until then, what the
//! application configures applies at each start. Once a part is saved, a
//! rebuilt object serves the saved part, and the setter that configures it
//! checks its argument without taking it, as a Notification Class's
//! `add_destination` does. A Timer's three properties come as a set (Table
//! 12-75, footnote 5), so a write to either of the two peers can write
//! keeps the set whole, Priority_For_Writing included. A saved
//! Default_Timeout that no longer lies in the range the application gives
//! the Timer is dropped for the application's value, with a warning.
//!
//! The same store keeps the reporting settings peers write to the analog,
//! binary and multi-state Input, Output and Value objects (#2331), as kind 4
//! of the format: a [`ReportingWriteConfig`], kept once an object opts in
//! with its `with_reporting_persistence` builder call.
//!
//! A Command has nothing here: its Action is read-only over the network
//! (Table 12-12 codes it R, and this stack takes no write to it), so only
//! the application sets it.
//!
//! [`AccessRightsPersistence`]: crate::access_control::AccessRightsPersistence
//! [`TimerObject`]: crate::timer::TimerObject
//! [`ChannelObject`]: crate::channel::ChannelObject

mod persistence;
pub(crate) mod reporting;
pub(crate) mod saving;

#[cfg(feature = "std")]
pub use persistence::FileWriteConfigPersistence;
pub use persistence::{
    ChannelMembers, ChannelWriteConfig, ReportingWriteConfig, TimerStateChange, TimerWriteConfig,
    WriteConfigPersistence, WriteConfigSnapshot,
};

// std: these load and save through the file backend.
#[cfg(all(test, feature = "std"))]
#[path = "write_config/persistence_tests.rs"]
mod persistence_tests;
// event-reporting: the reporting persistence these test is compiled with it.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/reporting_tests.rs"]
mod reporting_tests;
#[cfg(test)]
#[path = "write_config/test_storage.rs"]
pub(crate) mod test_storage;
