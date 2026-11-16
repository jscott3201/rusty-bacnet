//! A full `BACnetServer` on the session's link (#2617).
use super::*;
use bacnet_endpoint_core::seat::{ServerSeat, ServerSeatCounters};
use bacnet_server::server::{BACnetServer, ServerBuilder};

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// A seat for a full server on this session's link: its network layer
    /// and invoke-ID coordinator, shared, and a queue for what is the
    /// server's to receive, as deep as the session's request queue
    /// ([`SessionConfig::queue_capacity`]), whose place it takes. Refused
    /// where the server could not be the session's only server role (see
    /// [`Self::full_server`]); those refusals come before the check that the
    /// session runs.
    fn server_seat(&self) -> Result<ServerSeat<T>, Error> {
        let refused = |why: &str| Err(Error::Encoding(format!("no full server here: {why}")));
        if !matches!(self.role, SessionRole::ServerOnly | SessionRole::Both) {
            return refused(
                "the session has no server role, so a full client on it would answer \
                 requests too",
            );
        }
        if self.source_audit_reporter.is_some() {
            return refused("the session has a source Audit Reporter");
        }
        if self.cov.config.is_some() {
            return refused(
                "the session runs its own COV reporting; the full server brings its own",
            );
        }
        if self.registered_network_port.is_some() {
            return refused("the session's database holds its registered Network Port");
        }
        if !self.is_running() {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        self.ingress
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint session ingress owner is missing".into()))?
            .server_seat(Arc::clone(&self.coordinator))
    }

    /// A full [`BACnetServer`] on this session's link, built from `builder`:
    /// one transport, one address, one pool of invoke IDs, beside the
    /// session's client roles.
    ///
    /// It is the server a transport of its own gives, with every service,
    /// policy and counter that one has. While it lives it takes every
    /// request the session's own narrow server role would have taken, and
    /// answers it from its own database: the session's database, its
    /// Device writes, ReinitializeDevice handler and read limit no longer
    /// answer anything. Once the server is stopped or dropped, the narrow
    /// server role answers from the session's database again. A session
    /// with a [`DeviceIdentity`](crate::identity::DeviceIdentity) gives it
    /// to the server, replacing any the builder had, so the node is one
    /// device on its address; the database the builder holds must then
    /// serve that Device, or be left out for a stock one. See
    /// [`SeatedServerBuilder::build`](bacnet_server::server::SeatedServerBuilder::build)
    /// for what a seat changes, and the builder options it refuses.
    ///
    /// The session must be running and built with a server role
    /// ([`SessionRole::ServerOnly`] or [`SessionRole::Both`]), so that a
    /// [`full_client`](Self::full_client) leaves the server's requests to
    /// it. One full server at a time. Refused, too, on a session with a
    /// source Audit Reporter, with its own COV reporting
    /// ([`with_cov`](Self::with_cov)), or with a registered Network Port:
    /// those stay with the session's narrow server role for now. The session's
    /// own COV reporting, local writes and persistence go with that role in
    /// a later step of #2617, when the session builds its server itself.
    ///
    /// **Who answers what.** The server answers every confirmed request but
    /// a ConfirmedCOVNotification, which is a full client's; a
    /// ConfirmedEventNotification is the server's to acknowledge, and a full
    /// client hears a copy. It hears every unconfirmed request (Who-Is among
    /// them), as a full client does.
    ///
    /// **A server that falls behind.** The session never waits for it:
    /// what its seat's queue has no room for is dropped and counted in
    /// [`server_seat_counters`](Self::server_seat_counters), and a full
    /// client's traffic goes on. The queue is as deep as the session's
    /// request queue ([`SessionConfig::queue_capacity`]). Unconfirmed
    /// requests fill at most half of it, confirmed requests all but its last
    /// quarter, and answers to the server's own requests all of it. Past the
    /// queue, the server's own admission
    /// ([`RequestAdmissionPolicy`](bacnet_server::server::RequestAdmissionPolicy))
    /// refuses work its handlers have no room for.
    ///
    /// **One pool of invoke IDs.** The server and a full client lease from
    /// the session's one pool of 256. When it runs out, the server's
    /// confirmed notifications wait for an ID, while the client's requests
    /// fail at once: a server with many confirmed COV notifications in
    /// flight (up to 255) can leave the client's requests none.
    ///
    /// **DeviceCommunicationControl is the server's alone, for now.** The
    /// standard has it bind the whole device (Clause 16.1.2). Today a
    /// DISABLE_INITIATION the server accepts silences the server: its own
    /// requests and notifications and its
    /// [`broadcast_i_am`](BACnetServer::broadcast_i_am). It does not silence
    /// the full client's requests or the session's own
    /// [`broadcast_i_am`](Self::broadcast_i_am), which still go out. A later
    /// step of #2617 makes it hold for the whole node.
    ///
    /// **Stopping.** Stop the server, then the session: the server's
    /// [`stop`](BACnetServer::stop) joins its work and settles its saves,
    /// and leaves the link up. A session stopped first stops the link all
    /// the same; the server runs on for its own notifications, and what it
    /// sends fails with the stopped transport's error.
    ///
    /// The seat is taken when this is called; the future that builds the
    /// server borrows nothing of the session.
    pub fn full_server(
        &self,
        builder: ServerBuilder<T>,
    ) -> impl Future<Output = Result<BACnetServer<T>, Error>> + use<T> {
        let builder = match &self.identity {
            Some(identity) => builder.device(identity.clone()),
            None => builder,
        };
        let seated = self.server_seat().map(|seat| builder.seat(seat));
        async move { seated?.build().await }
    }

    /// What ingress handed to the full server's seat, and what it dropped
    /// because the server had left its queue full.
    pub fn server_seat_counters(&self) -> ServerSeatCounters {
        self.ingress
            .as_ref()
            .map(EndpointIngress::server_seat_counters)
            .unwrap_or_default()
    }
}
