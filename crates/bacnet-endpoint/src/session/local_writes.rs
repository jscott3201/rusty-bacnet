//! The application's own writes to the objects of its session's database
//! (#2468): bacnet-device-core's local write
//! ([`apply`](bacnet_device_core::write::local::apply)) under the session's
//! database guard, staged as `bacnet-server` stages its own.
//!
//! A change that writes is reported to the session's COV subscribers when
//! COV is enabled ([`EndpointSession::with_cov`], #2490). What else follows
//! a change on the server, its event pass and the Command, Channel, Timer,
//! Schedule and Staging work, has nothing to run on here: the endpoint runs
//! no event pass, and no task makes a run's writes or times a Timer. A
//! change is seen by the next read.

use super::*;
use bacnet_device_core::write::local::{self, LocalChange, LocalCommandSource};
use bacnet_device_core::write::{Applied, end_unmade};
use bacnet_objects::traits::LocalInputValue;
use bacnet_types::enums::Reliability;

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Write a property of an object in the session's database as the
    /// application: a network-equivalent write made as `source`, the
    /// counterpart of `BACnetServer::write_local` (#2468).
    ///
    /// bacnet-device-core's local write checks and commits it under the
    /// session's database guard, so it is refused where the server's is:
    /// an unknown object with OBJECT / UNKNOWN_OBJECT, an array index as
    /// WriteProperty refuses it, a name another object holds with
    /// DUPLICATE_NAME, and otherwise as the object refuses it.
    /// Present_Value at a priority commands that slot and NULL relinquishes
    /// it; a NULL to a property that isn't commandable and has no NULL
    /// succeeds and changes nothing (#1396). `source` names the local
    /// Device or an object of it as the command's initiator.
    ///
    /// **COV.** With [`with_cov`](Self::with_cov), a change that writes is
    /// reported to the subscribers of `oid` once its run, if any, has
    /// ended as below.
    ///
    /// **What doesn't follow on an endpoint.** The endpoint sends no event
    /// notification after a local change and runs no event pass, so
    /// Event_State stays as it is. No task
    /// makes the writes a change starts either; each run ends at once,
    /// unmade, as bacnet-device-core's `Responder` ends one a client's write
    /// starts, so none leaves its object busy, but what it leaves differs:
    ///
    /// - a Command's Present_Value write that selects a list with actions
    ///   ends it with In_Process and All_Writes_Successful FALSE and no
    ///   action written (one that selects no list, or an empty one, starts
    ///   no run);
    /// - a Channel's Present_Value write to a Channel with members ends its
    ///   distribution with Write_Status FAILED, Reliability PROCESS_ERROR
    ///   and Status_Flags FAULT, and no member written (with no members it
    ///   starts no run and Write_Status stays IDLE);
    /// - a write that starts or moves a Timer gives it its new state, but the
    ///   State_Change_Values writes it owes aren't made (Reliability
    ///   PROCESS_ERROR, FAULT), and with no Timer task a running Timer never
    ///   expires;
    /// - a Schedule's targets aren't written, nor a Staging object's.
    ///
    /// **Saved first.** A property its object keeps in storage, such as the
    /// Description of a Device built with
    /// `DeviceObject::with_tags_persistence`, is saved before the call
    /// returns, with the database guard dropped, as the endpoint saves one
    /// a client writes; a failed save refuses the write with DEVICE /
    /// OPERATIONAL_PROBLEM. Every property `BACnetServer::write_local` saves
    /// first is saved first here too.
    ///
    /// **Audited where the network write is.** A write of the Device's
    /// Audit_Notification_Recipient goes through the source Audit runtime,
    /// as [`write_audit_recipient`](Self::write_audit_recipient) does: the
    /// old and the new recipient are told of it, with this Device as the
    /// source and no invoke ID. Without a running source Reporter
    /// ([`with_source_audit_reporter`](Self::with_source_audit_reporter))
    /// the Device has no such property, PROPERTY / UNKNOWN_PROPERTY. No
    /// other local write is audited, as no other network write to the
    /// endpoint is.
    ///
    /// # Errors
    /// [`Error::Encoding`] when the session is not running (before
    /// [`start`](Self::start), or once [`stop`](Self::stop) has begun, a
    /// cancelled stop included) or has no database; nothing changes then. Otherwise the refusal above, with
    /// nothing changed.
    ///
    /// # Cancellation
    /// Dropped before it commits, the call has changed nothing the object
    /// serves. A save it had staged by then is let go of from a task of its
    /// own once the database guard is free, so storage comes back to what
    /// the object serves soon after, and the next write to the object
    /// isn't held up behind it. Once the commit begins it runs to its end
    /// under the guard, with no await.
    pub async fn write_local(
        &self,
        oid: &ObjectIdentifier,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
        source: LocalCommandSource,
    ) -> Result<(), Error> {
        let change = LocalChange::Property {
            property,
            array_index,
            value,
            priority,
            source,
        };
        self.apply_local(*oid, change).await
    }

    /// Supply an object's logical Present_Value from the application, the
    /// counterpart of `BACnetServer::set_present_value_local` (#2468).
    ///
    /// The object's own local input takes it: the Analog, Binary and
    /// Multi-state Inputs, noncommandable Values, the Integer Value, Loop
    /// and Life Safety Point and Zone among the built-in objects. They
    /// refuse it as the server's call does: PROPERTY / WRITE_ACCESS_DENIED
    /// while Out_Of_Service is TRUE (a Life Safety object takes it either
    /// way), PROPERTY / INVALID_DATA_TYPE or VALUE_OUT_OF_RANGE for a value
    /// it can't hold, and OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED from
    /// an object that takes no application value, a commandable one
    /// included. A Value that tracks
    /// its source then names this session's Device as Value_Source.
    ///
    /// **What follows on an endpoint:** a COV notification with
    /// [`with_cov`](Self::with_cov), but no event notification and no event
    /// pass, as for [`write_local`](Self::write_local). An input is not
    /// audited and saves nothing first.
    ///
    /// # Errors
    /// As [`write_local`](Self::write_local).
    ///
    /// # Cancellation
    /// Dropped before it commits, the call has changed nothing; an input
    /// stages no save, so it leaves nothing behind.
    pub async fn set_present_value_local(
        &self,
        oid: &ObjectIdentifier,
        value: PropertyValue,
    ) -> Result<(), Error> {
        let input = LocalInputValue::present_value(value);
        self.apply_local(*oid, LocalChange::Input(input)).await
    }

    /// Set the Reliability the application works out for an object, such
    /// as NO_SENSOR when its hardware reports a missing sensor, the
    /// counterpart of `BACnetServer::set_reliability_local` (#2468).
    ///
    /// The built-in Inputs, Outputs and Values, the Integer Value, Loop,
    /// Schedule, Life Safety Point and Zone, Access Zone, Credential Data
    /// Input and Accumulator take it. They refuse it with PROPERTY /
    /// WRITE_ACCESS_DENIED while Out_Of_Service is TRUE and while
    /// Reliability_Evaluation_Inhibit is TRUE where they have it, and a
    /// value outside BACnetReliability with PROPERTY / VALUE_OUT_OF_RANGE;
    /// an object that takes none refuses it with OBJECT /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Status_Flags' FAULT follows it.
    ///
    /// **What follows on an endpoint:** a COV notification with
    /// [`with_cov`](Self::with_cov), but no event notification and no event
    /// pass, as for [`write_local`](Self::write_local), so an object that reports faults
    /// keeps its Event_State and its Status_Flags show FAULT without
    /// IN_ALARM, where the server's event pass would add it. Not audited.
    ///
    /// # Errors
    /// As [`write_local`](Self::write_local).
    ///
    /// # Cancellation
    /// Dropped before it commits, the call has changed nothing; a
    /// Reliability stages no save, so it leaves nothing behind.
    pub async fn set_reliability_local(
        &self,
        oid: &ObjectIdentifier,
        reliability: Reliability,
    ) -> Result<(), Error> {
        self.apply_local(*oid, LocalChange::Reliability(reliability))
            .await
    }

    /// Make `change` to `oid` under the database guard, staged as the
    /// server stages its local writes, while the session runs, and report
    /// it to COV subscribers.
    async fn apply_local(&self, oid: ObjectIdentifier, change: LocalChange) -> Result<(), Error> {
        let not_running = || Error::Encoding("endpoint session is not running".into());
        if !self.is_running() {
            return Err(not_running());
        }
        let database = self
            .database
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint session has no local database".into()))?;
        EndpointResponder::local_commit(database, oid, change, |db, change| {
            // A defence only: `stop` takes `&mut self`, which this call's
            // borrow of the session excludes, so the session can't stop
            // while the call runs.
            if !self.is_running() {
                return Err(not_running());
            }
            let apply = |db: &mut ObjectDatabase| {
                let applied = local::apply(db, oid, change, None)?;
                if applied == Applied::Written {
                    // Nothing here makes a Command's, Channel's or Timer's
                    // run; end it now rather than leave the object busy.
                    end_unmade(db, &[oid]);
                }
                Ok(applied)
            };
            match self.cov.host() {
                Some(cov) => cov.local(db, oid, apply),
                None => apply(db),
            }
            .map(|_| ())
        })
        .await
    }
}

#[cfg(test)]
#[path = "local_write_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "local_staged_release_tests.rs"]
mod staged_release_tests;
