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
//! binary and multi-state Input, Output and Value objects, Access Door,
//! Access Zone, Event Enrollment and Alert Enrollment (#2331), as kind 4 of
//! the format: a [`ReportingWriteConfig`], kept once an object opts in with
//! its `with_reporting_persistence` builder call.
//!
//! The analog, binary and multi-state Input, Output and Value objects keep
//! the event algorithm parameters a peer writes in that file too (#2331):
//! Time_Delay and Time_Delay_Normal on all nine; High_Limit, Low_Limit,
//! Deadband and Limit_Enable on the analog ones; Alarm_Value on a Binary
//! Input or Value; Alarm_Values on a Multi-state Input or Value. They are
//! an [`AlgorithmWriteConfig`] in the `algorithm` field of the object's
//! [`ReportingWriteConfig`], stored as kind 6 of the format. They follow
//! the reporting settings' rules, and a rebuilt object's detector starts
//! with them. Only Alarm_Values has a setter (`set_alarm_values`), which
//! gives way to a saved list like the other setters. For the rest an
//! application sets its own values with `write_property` before the
//! builder call, and a saved write then goes over them. A `write_property`
//! after the builder call is a write like a peer's, and is kept. Storage
//! holding a value the object refuses on the wire, or a parameter its type
//! doesn't keep, fails the builder call.
//!
//! One such value turns on the object and not on the file: an Alarm_Values
//! entry past Number_Of_States, which a peer can't write (#1429). A peer
//! can't shrink the states under a held entry either, but an application
//! can rebuild the object with fewer states than a saved list names, and
//! Number_Of_States itself isn't kept, so a count a peer raised through
//! State_Text is the application's again after a restart. Either way the
//! builder call fails rather than serve a list no write leaves. The
//! application then opens the object with enough states and calls
//! `clear_written_reporting`, or removes what storage holds for it.
//!
//! Each of these object types keeps a written Description in the same file
//! too (#2364), as kind 5 of the format wrapped around the kind it writes,
//! with the same rules. A Timer and a Channel drop it with their
//! `clear_written_config`, and the reporting objects with
//! `clear_written_reporting`. The three logs, Notification Class,
//! Notification Forwarder and Access Rights keep it in their own formats
//! (see [`crate::snapshot`]). On every other object type a written
//! Description lasts until a restart.
//!
//! A Command has nothing here: its Action is read-only over the network
//! (Table 12-12 codes it R, and this stack takes no write to it), so only
//! the application sets it.
//!
//! [`AccessRightsPersistence`]: crate::access_control::AccessRightsPersistence
//! [`TimerObject`]: crate::timer::TimerObject
//! [`ChannelObject`]: crate::channel::ChannelObject

pub(crate) mod description;
mod persistence;
pub(crate) mod reporting;
pub(crate) mod saving;

#[cfg(feature = "std")]
pub use persistence::FileWriteConfigPersistence;
pub use persistence::{
    AlgorithmWriteConfig, ChannelMembers, ChannelWriteConfig, ReportingWriteConfig,
    TimerStateChange, TimerWriteConfig, WriteConfigPersistence, WriteConfigSnapshot,
};

// A written Description kept beside the rest (#2364).
#[cfg(test)]
#[path = "write_config/description_tests.rs"]
mod description_tests;
// The event algorithm parameters kept beside the reporting settings.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/parameters_tests.rs"]
mod parameters_tests;
// std: these load and save through the file backend.
#[cfg(all(test, feature = "std"))]
#[path = "write_config/persistence_tests.rs"]
mod persistence_tests;
// The binary and multi-state objects' parameters: the rules a request, a
// restart and the object's own state hold them to.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/state_parameter_rules_tests.rs"]
mod state_parameter_rules_tests;
// The binary and multi-state objects' parameters, kept as the analog ones.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/state_parameters_tests.rs"]
mod state_parameters_tests;
// event-reporting: the reporting persistence these test is compiled with it.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/reporting_tests.rs"]
mod reporting_tests;
// The other object types with intrinsic reporting: Access Door and Zone,
// Event and Alert Enrollment.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/reporting_types_tests.rs"]
mod reporting_types_tests;
#[cfg(test)]
#[path = "write_config/test_storage.rs"]
pub(crate) mod test_storage;
