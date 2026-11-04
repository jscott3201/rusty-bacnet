//! [`ActiveCovOverlay`]: a read overlay that answers the local Device's
//! Active_COV_Subscriptions and Active_COV_Multiple_Subscriptions from a
//! [`CovTable`], and serves everything else as stored (Clause 12.11).

use alloc::borrow::Cow;

use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::file::{FileConfiguration, FileStorage};
use bacnet_objects::property_metadata::{PropertyMetadata, has_property};
use bacnet_objects::traits::{
    AuditLog, AuditPolicy, BACnetObject, CalendarState, CommandRunner, CovReporting,
    EnrollmentEval, EventReporting, LogBuffer,
};
use bacnet_time::Instant;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{ActiveCovMultipleSubscriptions, ActiveCovSubscriptions};
use crate::cov::CovTable;
use crate::read::ReadOverlay;

/// The local Device's two subscription lists, read live from `table` at one
/// instant, for any read that takes a [`ReadOverlay`]
/// (`Responder::read`, the read plans).
///
/// A list is projected only when a read asks for it, against the database
/// the read runs on: an entry whose object is gone is left out, and each
/// numeric entry names the increment in use, as the server's lists do. The
/// Device answers for both only while its Property_List names them;
/// every other object, and every other property, is served as stored.
#[derive(Clone, Copy)]
pub struct ActiveCovOverlay<'a> {
    table: &'a CovTable,
    db: &'a ObjectDatabase,
    device: ObjectIdentifier,
    now: Instant,
}

impl<'a> ActiveCovOverlay<'a> {
    /// The lists of `device` in `table`, as of `now`, projected against
    /// `db`.
    pub fn new(
        table: &'a CovTable,
        db: &'a ObjectDatabase,
        device: ObjectIdentifier,
        now: Instant,
    ) -> Self {
        Self {
            table,
            db,
            device,
            now,
        }
    }

    /// The live value of `property` of the local Device, if it is one of
    /// the two lists.
    fn list(&self, property: P) -> Option<PropertyValue> {
        match property {
            P::ACTIVE_COV_SUBSCRIPTIONS => ActiveCovSubscriptions::project(
                self.db,
                self.device,
                self.table.active_cov_entries(self.now),
            )
            .resolve(self.device, property),
            P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS => ActiveCovMultipleSubscriptions::project(
                self.db,
                self.device,
                self.table.active_cov_multiple_entries(self.now),
            )
            .resolve(self.device, property),
            _ => None,
        }
    }
}

impl<'a> ReadOverlay for ActiveCovOverlay<'a> {
    type View<'v>
        = ActiveCovView<'v>
    where
        Self: 'v;

    fn view<'v>(&'v self, stored: &'v dyn BACnetObject) -> Option<ActiveCovView<'v>> {
        (stored.object_identifier() == self.device).then_some(ActiveCovView {
            stored,
            overlay: *self,
        })
    }
}

/// The local Device as an [`ActiveCovOverlay`] serves it.
pub struct ActiveCovView<'a> {
    stored: &'a dyn BACnetObject,
    overlay: ActiveCovOverlay<'a>,
}

impl BACnetObject for ActiveCovView<'_> {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.stored.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.stored.object_name()
    }
    fn read_property(&self, property: P, index: Option<u32>) -> Result<PropertyValue, Error> {
        // A list takes no index; the stored Device refuses one as it does.
        if index.is_none()
            && matches!(
                property,
                P::ACTIVE_COV_SUBSCRIPTIONS | P::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS
            )
            && has_property(&self.stored.property_metadata(), property)
            && let Some(live) = self.overlay.list(property)
        {
            return Ok(live);
        }
        self.stored.read_property(property, index)
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.stored.property_metadata()
    }
    // Every other read-only query is the stored object's own answer, as the
    // server's Device view forwards them; the view serves reads only.
    fn audit_policy(&self) -> Option<&dyn AuditPolicy> {
        self.stored.audit_policy()
    }
    fn audit_reporter(&self) -> Option<&AuditReporterObject> {
        self.stored.audit_reporter()
    }
    fn command_runner(&self) -> Option<&dyn CommandRunner> {
        self.stored.command_runner()
    }
    fn event_reporting(&self) -> Option<&dyn EventReporting> {
        self.stored.event_reporting()
    }
    fn enrollment_eval(&self) -> Option<&dyn EnrollmentEval> {
        self.stored.enrollment_eval()
    }
    fn cov(&self) -> Option<&dyn CovReporting> {
        self.stored.cov()
    }
    fn calendar_state(&self) -> Option<&dyn CalendarState> {
        self.stored.calendar_state()
    }
    fn audit_log(&self) -> Option<&dyn AuditLog> {
        self.stored.audit_log()
    }
    fn file_configuration(&self) -> Option<&dyn FileConfiguration> {
        self.stored.file_configuration()
    }
    fn file_storage(&self) -> Option<&dyn FileStorage> {
        self.stored.file_storage()
    }
    fn log_buffer(&self) -> Option<&dyn LogBuffer> {
        self.stored.log_buffer()
    }
}
