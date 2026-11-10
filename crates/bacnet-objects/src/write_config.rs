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
//! The analog, binary and multi-state Input, Output and Value objects,
//! Access Door and Access Zone keep the event algorithm parameters a peer
//! writes in that file too (#2331): Time_Delay and Time_Delay_Normal on all
//! eleven; High_Limit, Low_Limit, Deadband and Limit_Enable on the analog
//! ones; Alarm_Value on a Binary Input or Value; Alarm_Values on a
//! Multi-state Input or Value, an Access Door and an Access Zone;
//! Fault_Values and Masked_Alarm_Values on an Access Door. They are
//! an [`AlgorithmWriteConfig`] in the `algorithm` field of the object's
//! [`ReportingWriteConfig`], stored as kind 6 of the format. They follow
//! the reporting settings' rules, and a rebuilt object's detector starts
//! with them. Only the lists have setters (`set_alarm_values`, and a
//! door's `set_fault_values` and `set_masked_alarm_values`), which give way
//! to a saved list like the other setters. For the rest an
//! application sets its own values with `write_property` before the
//! builder call, and a saved write then goes over them. A `write_property`
//! after the builder call is a write like a peer's, and is kept. Storage
//! holding a value the object refuses on the wire, or a parameter its type
//! doesn't keep, fails the builder call.
//!
//! An Access Door's and an Access Zone's lists are checked against their
//! enumerations alone (a named or proprietary state other than NORMAL), on
//! a write and on a load alike, so no saved list is one the rebuilt object
//! would refuse. A door that has taken its saved lists settles
//! Door_Alarm_State as it does after a write of one: a state the lists no
//! longer admit goes back to NORMAL. An Event Enrollment keeps no
//! parameter yet.
//!
//! One wire rule doesn't fail the builder call: an Alarm_Values entry past
//! Number_Of_States, which a write refuses (#1429). The state count and
//! State_Text aren't kept. So a count a peer raised through State_Text is
//! the application's again after a restart, and a list the peer then wrote
//! may name a state the rebuilt object lacks; so may any saved list once an
//! application builds the object with fewer states. Such a list is restored
//! as it was written, since two writes the device accepted mustn't stop it
//! from starting. The object holds it as it holds a list a local
//! `set_number_of_states` shrink leaves: it serves it, and a write of the
//! same list is refused. What follows differs by type, and is the cost of
//! opening. A Multi-state Input reports no fault, and the entries within
//! its states still alarm. A Multi-state Value reports CONFIGURATION_ERROR
//! and goes to FAULT, and while it is there no listed state raises
//! OFFNORMAL, the ones within its states included: it raises no alarm on
//! its value until a list within its states is written or the states are
//! there again.
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

// Access Door's and Access Zone's parameters: their setters, the clear and
// a request's staged writes.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/access_parameter_rules_tests.rs"]
mod access_parameter_rules_tests;
// Access Door's and Access Zone's parameters, kept as the others'.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/access_parameters_tests.rs"]
mod access_parameters_tests;
// What a restore does to a door and a zone beyond storing the values.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/access_restore_tests.rs"]
mod access_restore_tests;
// A written Description kept beside the rest (#2364).
#[cfg(test)]
#[path = "write_config/description_tests.rs"]
mod description_tests;
// Every row of the format's table, through the object of each type that
// keeps it.
#[cfg(all(test, feature = "event-reporting"))]
#[path = "write_config/parameter_table_tests.rs"]
mod parameter_table_tests;
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
