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
/// released (a Load Control's application is told to stop, and the object
/// keeps its request), and a registered Network Port it published is put
/// back as configured, so the same database can go into another start. A
/// returned Load Control still reads the shed state and levels its request
/// had, though its application was told to stop, until the next start
/// looks at the request again.
///
/// The database comes back with what every start sets and the next sets
/// again:
///
/// - the identity's Device bound as the local Device;
/// - the start's clocks and wakers;
/// - a registered Network Port's pending writes dropped;
/// - each selected Audit Reporter's configured flag, and the forwarding
///   Audit Log's, as the start's route checks left them;
/// - Color objects and Color Temperature objects at the value the start
///   drove them to, each one's one-time restore from storage spent;
/// - on a seat, the Device given the node's APDU timing and the node's
///   registered port told whether this server activates its changes.
///
/// It is `None` when the builder had none and refused before it made the
/// stock one.
///
/// The endpoint crate's link builders and its session's start return it
/// too (#2766): a builder hands back the database it was given, as given,
/// and a session's start the one it took, once a refusal has ended the
/// session; their documentation says when theirs is `None`.
///
/// Unless [`into_parts`](Self::into_parts) hands the database over, it is
/// let go of off the async runtime
/// ([`drop_database_off_runtime`](super::drop_database_off_runtime)), as
/// objects with storage wait for their saves as they drop: when `?` turns
/// the refusal into [`Error`], and when it is dropped. It displays as its
/// error and gives that error's source. It is one pointer wide, so a
/// `Result` holding it is no wider than the server. A caller whose own
/// error type converts only from [`Error`] maps it first:
/// `.map_err(Error::from)`.
///
/// [`BACnetServer::start`]: super::BACnetServer::start
pub struct StartError(Box<Refused>);

const _: () = assert!(std::mem::size_of::<StartError>() == std::mem::size_of::<usize>());

/// It goes wherever an error does: across threads and into a boxed error.
const _: fn() = || {
    fn pinned<T: Send + Sync + 'static + std::error::Error>() {}
    pinned::<StartError>();
};

struct Refused {
    error: Error,
    database: OffRuntime,
}

/// The database a refusal holds, let go of off the runtime when dropped.
struct OffRuntime(Option<ObjectDatabase>);

impl Drop for OffRuntime {
    fn drop(&mut self) {
        if let Some(database) = self.0.take() {
            let held = std::sync::Arc::new(tokio::sync::RwLock::new(database));
            drop(super::drop_database_off_runtime(held));
        }
    }
}

/// The endpoint's refusals (#2766): a [`StartError`] for `error`, with the
/// database the endpoint's build or start took, if any.
pub fn refused(error: Error, database: Option<ObjectDatabase>) -> StartError {
    StartError::new(error, database)
}

impl StartError {
    pub(crate) fn new(error: Error, database: Option<ObjectDatabase>) -> Self {
        Self(Box::new(Refused {
            error,
            database: OffRuntime(database),
        }))
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
        self.0.database.0.as_ref()
    }

    /// The error, and the database to build another server over.
    pub fn into_parts(self) -> (Error, Option<ObjectDatabase>) {
        let Refused {
            error,
            mut database,
        } = *self.0;
        let database = database.0.take();
        (error, database)
    }
}

impl From<StartError> for Error {
    fn from(refused: StartError) -> Self {
        let Refused { error, database } = *refused.0;
        drop(database);
        error
    }
}

impl fmt::Debug for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StartError")
            .field("error", &self.0.error)
            .field("database", &self.database().map(ObjectDatabase::len))
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
