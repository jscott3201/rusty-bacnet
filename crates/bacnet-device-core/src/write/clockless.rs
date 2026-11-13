//! The Recipient_List write filter of a device with no clock (#2303,
//! decision 14). Without a clock the device sends only to destinations open
//! every day, all day ([`always_in_window`]), so a write that would store
//! any other is refused before it commits, which Clause 12.21.8 permits.
//! The server, which has the system clock, keeps storing them.

use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::notification_class::{always_in_window, decode_recipient_list};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{WriteCommitObserver, WriteTarget};

/// Whether a device with no clock refuses writing `value` to `oid`'s
/// `property`: a Notification Class's whole Recipient_List holding a
/// destination that isn't open every day, all day. Refused with PROPERTY /
/// VALUE_OUT_OF_RANGE. A value that doesn't decode as destinations passes,
/// for the class to refuse as it would. A host that runs AddListElement
/// itself checks each element list with this before the class takes it.
pub fn check_clockless_recipients(
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    value: &PropertyValue,
) -> Result<(), Error> {
    if oid.object_type() != ObjectType::NOTIFICATION_CLASS
        || property != PropertyIdentifier::RECIPIENT_LIST
    {
        return Ok(());
    }
    match decode_recipient_list(value) {
        Ok(destinations) if !destinations.iter().all(always_in_window) => Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::VALUE_OUT_OF_RANGE,
        )),
        _ => Ok(()),
    }
}

/// An observer that applies [`check_clockless_recipients`] to each write
/// before it commits, and passes every call on to the one it wraps. The
/// `Responder` wraps its caller's observer in it when it has no clock.
pub(crate) struct ClocklessRecipients<'a>(
    pub(crate) Option<&'a mut (dyn WriteCommitObserver + 'a)>,
);

impl WriteCommitObserver for ClocklessRecipients<'_> {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>) {
        if let Some(inner) = self.0.as_deref_mut() {
            inner.before(db, write);
        }
    }

    fn commit_policy(
        &mut self,
        db: &mut ObjectDatabase,
        write: WriteTarget<'_>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        // A whole-list write only: an element index is the class's to refuse.
        if write.array_index.is_none()
            && let Err(error) = check_clockless_recipients(write.oid, write.property, value)
        {
            return Some(Err(error));
        }
        self.0.as_deref_mut()?.commit_policy(db, write, value)
    }

    fn committed(&mut self, db: &mut ObjectDatabase) {
        if let Some(inner) = self.0.as_deref_mut() {
            inner.committed(db);
        }
    }

    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error) {
        if let Some(inner) = self.0.as_deref_mut() {
            inner.failed(db, error);
        }
    }

    fn applied(&mut self, db: &ObjectDatabase, oid: ObjectIdentifier) {
        if let Some(inner) = self.0.as_deref_mut() {
            inner.applied(db, oid);
        }
    }
}

#[cfg(test)]
#[path = "clockless_tests.rs"]
mod tests;
