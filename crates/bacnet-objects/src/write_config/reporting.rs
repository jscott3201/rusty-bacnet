//! Keeping the reporting settings peers write to an intrinsic reporter
//! (#2331): Notification_Class, Event_Enable, Notify_Type,
//! Event_Detection_Enable, Event_Message_Texts_Config and the
//! Event_Algorithm_Inhibit pair, the rows every intrinsic reporter shares.
//!
//! The values stay where the object holds them (its detector,
//! Event_Detection_Enable and the `ReportingOptions` of its
//! `EventHistory`). An object opts in by holding a `ReportingStore`,
//! implementing `KeepsReporting` to lend its rows, and offering each write
//! to [`admit`] before it makes it; [`impl_reporting_persistence!`] does all
//! of that, and the builder call, for objects that hold the rows in the
//! usual fields.
//!
//! Storage is the object's
//! [`WriteConfigPersistence`](super::WriteConfigPersistence), one file per
//! object, of kind
//! [`KIND_REPORTING`](crate::snapshot::write_config::KIND_REPORTING).
//! As for Timers and Channels ([`super::saving`]), a write of a kept row is
//! saved before the object serves it, and refused with DEVICE /
//! OPERATIONAL_PROBLEM, the old value kept, when the save fails. The server
//! stages it, so the save runs off the database guard; a write nobody staged
//! saves where it is. Only the rows a write set are kept, and a kept row
//! wins over its setter until `clear_written_reporting` drops them all.
//!
//! All of this needs the `event-reporting` feature, which `std` turns on.
//! Without it the objects keep nothing: they hold no store, [`admit`] lets
//! every write through, and the builder call isn't there, so an image that
//! reports nothing links none of it.

#[cfg(feature = "event-reporting")]
mod kept;
#[cfg(feature = "event-reporting")]
pub(crate) use kept::*;

#[cfg(not(feature = "event-reporting"))]
pub(crate) use off::*;

/// Without `event-reporting`: nothing is kept.
#[cfg(not(feature = "event-reporting"))]
mod off {
    use bacnet_types::enums::PropertyIdentifier;
    use bacnet_types::error::Error;
    use bacnet_types::primitives::PropertyValue;

    /// Every write goes through as the object makes it.
    pub(crate) fn admit<T>(
        _object: &mut T,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: &PropertyValue,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// The object lends only its Tags staging.
    macro_rules! impl_reporting_persistence {
        ($object:ty) => {
            impl $object {
                #[cfg(feature = "durable-writes")]
                fn durable_capability(
                    &mut self,
                ) -> Option<&mut dyn $crate::durable::DurableWrites> {
                    self.profile.capability()
                }
            }
        };
    }

    pub(crate) use impl_reporting_persistence;
}
