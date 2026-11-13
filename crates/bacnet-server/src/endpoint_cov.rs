//! The seam through which an endpoint session's COV reporting joins its
//! responder (#2490).
//!
//! `bacnet-endpoint` keeps a `bacnet-device-core` `Reporter` for the objects
//! of its session's database and implements [`EndpointCov`] over it. The
//! endpoint responder, once given one
//! ([`EndpointResponder::with_cov`](crate::server::__endpoint_EndpointResponder::with_cov)),
//! hands it each SubscribeCOV, SubscribeCOVProperty and
//! SubscribeCOVPropertyMultiple and each read of the Device's
//! Active_COV_Subscriptions and Active_COV_Multiple_Subscriptions. It is
//! not an interface for applications.

use bacnet_device_core::cov::active::LiveDeviceCov;
use bacnet_device_core::read::LiveLists;
use bacnet_encoding::apdu::Apdu;
use bacnet_network::layer::ReceivedApdu;
use bacnet_objects::database::ObjectDatabase;

/// The COV reporting an endpoint session runs beside its responder.
///
/// The responder calls [`subscribe`](Self::subscribe) with the database's
/// write guard held and [`live`](Self::live) with its read guard held, and
/// [`replied`](Self::replied) once it has dropped the guard. An
/// implementation takes its own state after a guard, never before, and
/// never waits.
#[doc(hidden)]
pub trait EndpointCov: Send + Sync {
    /// Run the SubscribeCOV, SubscribeCOVProperty or
    /// SubscribeCOVPropertyMultiple request `received` carries against `db`
    /// and return the reply, `None` when it wrote none, which a request the
    /// responder decoded never draws. What it queues, the first
    /// notification of an accepted subscription, stays queued until
    /// [`replied`](Self::replied), so that it follows the reply onto the
    /// wire. Each call is followed by one `replied`, whatever it returned.
    fn subscribe(&self, db: &mut ObjectDatabase, received: &ReceivedApdu) -> Option<Apdu>;

    /// The reply to one [`subscribe`](Self::subscribe) has been handed off,
    /// or never will be: what that call queued may go.
    fn replied(&self);

    /// The Device's subscription lists `selection` names, live now and
    /// projected against `db`.
    fn live(&self, db: &ObjectDatabase, selection: LiveLists) -> LiveDeviceCov;
}
