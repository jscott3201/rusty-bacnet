//! A refused server start, with the database it took (#2752).

use std::fmt;

use bacnet_objects::database::ObjectDatabase;
use bacnet_types::error::Error;

/// Why a server didn't start, with the database the start took.
///
/// Every builder's `build` and every [`BACnetServer::start`] call that
/// refuses returns one: a setting a check refuses, a transport that won't
/// bind, a BACnet/SC hub that refuses the dial, a Device or Audit
/// configuration the start refuses. The server's start releases what it
/// began first: a bound transport is stopped, the outputs it drove are
/// released, and a registered Network Port it published is put back as
/// configured, so the same database can go into another start.
///
/// The database comes back with what every start sets and the next sets
/// again: the identity's Device bound as the local Device, the start's
/// clocks and wakers, and a registered Network Port's pending writes
/// dropped. A seated build has given the database's Device the node's APDU
/// timing too. It is `None` when the builder had none and refused before it
/// made the stock one.
///
/// `?` turns it into [`Error`], letting go of the database off the async
/// runtime ([`drop_database_off_runtime`](super::drop_database_off_runtime)).
/// It displays as its error and gives that error's source. It is one
/// pointer wide, so a `Result` holding it is no wider than the server.
///
/// [`BACnetServer::start`]: super::BACnetServer::start
pub struct StartError(Box<Refused>);

const _: () = assert!(std::mem::size_of::<StartError>() == std::mem::size_of::<usize>());

struct Refused {
    error: Error,
    database: Option<ObjectDatabase>,
}

impl StartError {
    pub(crate) fn new(error: Error, database: Option<ObjectDatabase>) -> Self {
        Self(Box::new(Refused { error, database }))
    }

    /// A refusal that takes the database from `database` with it, for
    /// `map_err` on a check made while the builder still holds it.
    pub(crate) fn taking(database: &mut Option<ObjectDatabase>) -> impl FnOnce(Error) -> Self + '_ {
        |error| Self::new(error, database.take())
    }

    /// Why the start was refused.
    pub fn error(&self) -> &Error {
        &self.0.error
    }

    /// The database the start took.
    pub fn database(&self) -> Option<&ObjectDatabase> {
        self.0.database.as_ref()
    }

    /// The error, and the database to build another server over.
    pub fn into_parts(self) -> (Error, Option<ObjectDatabase>) {
        let Refused { error, database } = *self.0;
        (error, database)
    }
}

impl From<StartError> for Error {
    fn from(refused: StartError) -> Self {
        let (error, database) = refused.into_parts();
        if let Some(database) = database {
            let held = std::sync::Arc::new(tokio::sync::RwLock::new(database));
            drop(super::drop_database_off_runtime(held));
        }
        error
    }
}

impl fmt::Debug for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartError")
            .field("error", &self.0.error)
            .field(
                "database",
                &self.0.database.as_ref().map(ObjectDatabase::len),
            )
            .finish()
    }
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0.error, f)
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.0.error)
    }
}
