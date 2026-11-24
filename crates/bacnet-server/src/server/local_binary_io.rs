//! The application's routes to the physical side of a running Binary Input
//! and Binary Output, and to their Polarity (#2580). The two inputs are
//! typed local inputs ([`LocalWrite`]) made through bacnet-device-core's
//! local write, so the post-write work is the other `*_local` routes': the
//! event pass and the COV fanout.

use super::local_writes::LocalWrite;
use super::*;
use bacnet_types::enums::Polarity;

impl<T: TransportPort + 'static> BACnetServer<T> {
    /// Report the state the application read from a Binary Input's
    /// physical input, ON as `true`, while the server holds the object.
    ///
    /// In service the object derives Present_Value from it through
    /// Polarity: ACTIVE for ON under NORMAL and INACTIVE for ON under
    /// REVERSE (Table 12-7). A logical value goes through
    /// [`BACnetServer::set_present_value_local`] instead. Unlike that
    /// route, this one is taken while Out_Of_Service is TRUE: Present_Value
    /// then stays decoupled from the input and keeps a client's simulated
    /// value (Clause 12.6.10), the object keeps the state, and the return
    /// to service derives Present_Value from the latest one. An unknown
    /// object fails with OBJECT / UNKNOWN_OBJECT and any object other than
    /// a Binary Input with OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED, as
    /// with `set_present_value_local`.
    ///
    /// A change of Present_Value goes through the same post-write event and
    /// COV processing as `set_present_value_local` once the database lock
    /// is released, so CHANGE_OF_STATE and COV subscribers see the logical
    /// state. Like the other `*_local` inputs it is not audited. The Python
    /// binding exposes this as `BACnetServer.set_physical_input_local`.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the write has committed skips none of the work
    /// the write owes (#1367).
    /// A call dropped part way is as [`write_local`](Self::write_local)'s Cancellation section
    /// describes (#2772).
    pub async fn set_physical_input_local(
        &self,
        oid: &ObjectIdentifier,
        on: bool,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::ApplicationPhysicalInput(on),
            PropertyValue::Null,
        )
        .await
    }

    /// Set the Polarity of a Binary Input or Binary Output while the server
    /// holds it. Polarity is read-only on the network, so this is the
    /// application's route; `write_local` refuses it as a client's write is
    /// refused, with PROPERTY / WRITE_ACCESS_DENIED.
    ///
    /// NORMAL and REVERSE are taken in or out of service; any other value
    /// fails with PROPERTY / VALUE_OUT_OF_RANGE. In service a Binary Input
    /// derives Present_Value again from the physical input state it holds
    /// (Clause 12.6.11), and the event and COV processing of a changed
    /// Present_Value follows; one that has had no state from the
    /// application yet keeps its Present_Value. A Binary Output keeps its
    /// Present_Value and drives its output from the new Polarity at once
    /// ([`physical_output_local`](Self::physical_output_local)). Out of
    /// service neither changes until the return to service. An unknown
    /// object fails with OBJECT / UNKNOWN_OBJECT and any other object with
    /// OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. Not audited. The
    /// Python binding exposes this as `BACnetServer.set_polarity_local`.
    ///
    /// Like [`write_local`](Self::write_local), it must be awaited inside a
    /// Tokio runtime, failing before anything is written outside one, and a
    /// caller dropped once the write has committed skips none of the work
    /// the write owes (#1367).
    /// A call dropped part way is as [`write_local`](Self::write_local)'s Cancellation section
    /// describes (#2772).
    pub async fn set_polarity_local(
        &self,
        oid: &ObjectIdentifier,
        polarity: Polarity,
    ) -> Result<(), Error> {
        self.write_local_as(
            oid,
            LocalWrite::ApplicationPolarity(polarity),
            PropertyValue::Null,
        )
        .await
    }

    /// The state to drive a Binary Output's physical output to, ON as
    /// `true`.
    ///
    /// In service it is the commanded Present_Value through Polarity: ON
    /// for ACTIVE under NORMAL and OFF for ACTIVE under REVERSE (Table
    /// 12-9). While Out_Of_Service is TRUE it stays at the state it had
    /// when Out_Of_Service became TRUE, as commands keep moving
    /// Present_Value (Clause 12.7.10). An unknown object fails with
    /// OBJECT / UNKNOWN_OBJECT and any object other than a Binary Output,
    /// one an adapter wraps included, with OBJECT /
    /// OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED. The Python binding exposes
    /// this as `BACnetServer.physical_output_local`.
    pub async fn physical_output_local(&self, oid: &ObjectIdentifier) -> Result<bool, Error> {
        let target = *oid;
        let read = move |db: tokio::sync::OwnedRwLockReadGuard<ObjectDatabase>| {
            physical_output(&db, &target)
        };
        // Never waited for in the caller's task (#2772).
        let output = self
            .request_tasks
            .for_caller(Arc::clone(&self.db).read_owned(), read)
            .await;
        match output {
            Some(output) => output,
            // Its runtime dropped the task before it read: read here.
            None => physical_output(&*self.db.read().await, oid),
        }
    }
}

/// The state `oid`'s physical output is driven to, as
/// [`BACnetServer::physical_output_local`] reads it.
fn physical_output(db: &ObjectDatabase, oid: &ObjectIdentifier) -> Result<bool, Error> {
    if db.get(oid).is_none() {
        return Err(Error::protocol(
            ErrorClass::OBJECT,
            ErrorCode::UNKNOWN_OBJECT,
        ));
    }
    db.physical_output(oid)
        .ok_or_else(bacnet_objects::traits::not_supported)
}
