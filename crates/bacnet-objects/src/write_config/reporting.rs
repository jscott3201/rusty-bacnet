//! Keeping the reporting settings peers write to an intrinsic reporter
//! (#2331): Notification_Class, Event_Enable, Notify_Type,
//! Event_Detection_Enable, Event_Message_Texts_Config and the
//! Event_Algorithm_Inhibit pair, the rows every intrinsic reporter shares.
//!
//! The values stay where the object holds them (its detector,
//! Event_Detection_Enable and the `ReportingOptions` of its
//! `EventHistory`). An object opts in by holding a `ReportingStore`,
//! implementing `KeepsReporting` to lend its rows, and offering each write
//! to [`admit`] before it makes it; [`impl_reporting_persistence!`] gives it
//! the builder call and the staging, and the `KeepsReporting` too for
//! objects that hold the rows in the usual fields. Each object keeps its
//! Description the same way, as one more row (#2364): it isn't a reporting
//! setting, but these objects already keep a file, and a head end that
//! labels an object expects the label to stay. Every object type with
//! intrinsic reporting here serves all seven rows, so each keeps them all:
//! the analog, binary and multi-state objects, Access Door and Access Zone
//! (through their `ChangeOfStateReporting`), and Event and Alert
//! Enrollment. The logs keep the same rows in their own files (#1952).
//!
//! The analog, binary and multi-state objects, Access Door and Access Zone
//! keep their event algorithm parameters as more rows (the `parameters`
//! module here, compiled with `event-reporting` like the rest): the two
//! time delays on all eleven, the four OUT_OF_RANGE parameters on the
//! analog ones, Alarm_Value on a Binary Input or Value, Alarm_Values on a
//! Multi-state Input or Value, a door and a zone, and a door's Fault_Values
//! and Masked_Alarm_Values. An Event Enrollment keeps Event_Parameters,
//! Fault_Parameters and Time_Delay_Normal the same way
//! (`event_enrollment::kept`), and a multi-state object its State_Text,
//! and with it its count, as one more row of the same table (#2414). They
//! stay where the object holds them (the
//! detector; a door's two other lists beside it; an enrollment's own
//! fields), and a staged write of one
//! is checked by the code the object writes it through. Which
//! parameters a type keeps is one table,
//! [`kept`](crate::snapshot::write_config::parameters::kept);
//! a write of one the type doesn't keep is made as without storage.
//!
//! Storage is the object's
//! [`WriteConfigPersistence`](super::WriteConfigPersistence), one file per
//! object, of kind
//! [`KIND_REPORTING`](crate::snapshot::write_config::KIND_REPORTING), or
//! [`KIND_REPORTING_PARAMETERS`](crate::snapshot::write_config::KIND_REPORTING_PARAMETERS)
//! once it holds an algorithm parameter.
//! As for Timers and Channels ([`super::saving`]), a write of a kept row is
//! saved before the object serves it, and refused with DEVICE /
//! OPERATIONAL_PROBLEM, the old value kept, when the save fails. The server
//! stages it, so the save runs off the database guard; a write nobody staged
//! saves where it is. Only the rows a write set are kept, and a kept row
//! wins over its setter until `clear_written_reporting` drops them all,
//! Description and the algorithm parameters included. A NULL to Description
//! changes nothing, so it is neither saved nor staged.
//!
//! An object CreateObject builds through the built-in constructors
//! ([`crate::creation::builtin_constructor`]) keeps nothing: an application
//! that wants a created object's settings kept registers a constructor of its
//! own that attaches storage.
//!
//! All of this needs the `event-reporting` feature, which `std` turns on.
//! Without it the objects keep nothing: they hold no store, [`admit`] lets
//! every write through, and the builder call isn't there, so an image that
//! reports nothing links none of it.

#[cfg(feature = "event-reporting")]
mod kept;
#[cfg(feature = "event-reporting")]
mod macros;
#[cfg(feature = "event-reporting")]
pub(crate) mod parameters;
#[cfg(feature = "event-reporting")]
pub(crate) use kept::*;
#[cfg(feature = "event-reporting")]
pub(crate) use macros::*;

#[cfg(not(feature = "event-reporting"))]
pub(crate) use off::*;

/// Without `event-reporting`: nothing is kept.
#[cfg(not(feature = "event-reporting"))]
mod off {
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::error::Error;
    use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

    /// Nothing to hold: an object built without `event-reporting` keeps no
    /// reporting settings, and this field costs it nothing.
    #[derive(Clone, Default)]
    pub(crate) struct ReportingStore;

    /// Every setter applies.
    pub(crate) fn setter_gives_way(
        _store: &ReportingStore,
        _object: ObjectIdentifier,
        _property: PropertyIdentifier,
    ) -> bool {
        false
    }

    /// Every write goes through as the object makes it.
    pub(crate) fn admit<T>(
        _object: &mut T,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: &PropertyValue,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// The object lends only its Tags staging, if it has any; its
    /// Description setter always applies. A `states:` argument names a
    /// field nothing here reads.
    macro_rules! impl_reporting_persistence {
        ($object:ty $(, states: $states:ident)?) => {
            impl $object {
                /// Set the description string.
                pub fn set_description(&mut self, desc: impl Into<String>) {
                    self.core.description = desc.into();
                }

                #[cfg(feature = "durable-writes")]
                fn durable_capability(
                    &mut self,
                ) -> Option<&mut dyn $crate::durable::DurableWrites> {
                    self.core.profile.capability()
                }
            }
        };
        ($object:ty, without_tags) => {
            impl $object {
                #[cfg(feature = "durable-writes")]
                fn durable_capability(
                    &mut self,
                ) -> Option<&mut dyn $crate::durable::DurableWrites> {
                    None
                }
            }
        };
    }

    pub(crate) use impl_reporting_persistence;
}
