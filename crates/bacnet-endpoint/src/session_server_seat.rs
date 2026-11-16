//! A full `BACnetServer` on the session's link (#2617).
use super::*;
use bacnet_endpoint_core::seat::{NodeDevice, ServerSeat, ServerSeatCounters};
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
        if self.identity.is_none() {
            return refused(
                "the session has no Device identity, which is the node's one set of device \
                 settings: compose one with device()",
            );
        }
        if !self.is_running() {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        let mut seat = self
            .ingress
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint session ingress owner is missing".into()))?
            .server_seat(Arc::clone(&self.coordinator))?;
        seat.node = Some(NodeDevice {
            client: matches!(self.role, SessionRole::Both),
            client_max_apdu_length: self.client_config.max_apdu_length,
            apdu_timeout_ms: self.config.apdu_timeout_ms,
            apdu_retries: self.config.apdu_retries,
        });
        Ok(seat)
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
    /// server role answers from the session's database again. See
    /// [`SeatedServerBuilder::build`](bacnet_server::server::SeatedServerBuilder::build)
    /// for what a seat changes, and the builder options it refuses.
    ///
    /// **One device.** The node has one set of device settings, the
    /// session's, and the server serves them as the node's Device:
    ///
    /// - the session's [`DeviceIdentity`](crate::identity::DeviceIdentity),
    ///   with Max_APDU_Length_Accepted the smaller of the identity's and the
    ///   client role's [`SessionConfig::max_apdu_length`] (Clause 12.11.18),
    ///   resolved to the link. A builder identity that is not the default
    ///   and differs from it is refused, and the database the builder holds
    ///   must serve that Device, or be left out for a stock one;
    /// - APDU_Timeout and Number_Of_APDU_Retries are
    ///   [`SessionConfig::apdu_timeout_ms`] and
    ///   [`SessionConfig::apdu_retries`] (default 6000 ms and 3), which the
    ///   client roles use and the server's confirmed notifications use too. A builder that set other
    ///   values, or a database whose Device declares others, is refused; a
    ///   Device that declares the object model's defaults (6000 ms and 3, as
    ///   one built from an identity does) is given the session's;
    /// - on a [`SessionRole::Both`] session, Protocol_Services_Supported
    ///   lists what the node's client executes (I-Am, I-Have, both COV
    ///   notifications) beside the server's services (Clause 12.11.14). It
    ///   follows the session's role, not whether a full client is attached
    ///   at the moment: until one is, a ConfirmedCOVNotification the Device
    ///   lists reaches the server, which rejects it, and a full client on a
    ///   [`SessionRole::ServerOnly`] session executes services the Device
    ///   doesn't list. Following the attached client would cost every
    ///   ReadProperty a read of shared state;
    /// - Segmentation_Supported, Max_Segments_Accepted and
    ///   APDU_Segment_Timeout are the identity's, which the server follows.
    ///   A [`full_client`](Self::full_client) keeps its own segmentation
    ///   settings until the session builds its server itself (a later step
    ///   of #2617): it accepts segmented answers even where the Device says
    ///   it receives none, and, as before, it aborts a segmented
    ///   ConfirmedCOVNotification, which it can't reassemble.
    ///
    /// The session must be running, built with a server role
    /// ([`SessionRole::ServerOnly`] or [`SessionRole::Both`]), so that a
    /// [`full_client`](Self::full_client) leaves the server's requests to
    /// it, and composed with a Device identity
    /// ([`device`](Self::device)). One full server at a time. Refused, too, on a session with a
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
    /// **DeviceCommunicationControl binds the node.** The standard has it
    /// bind the whole device (Clause 16.1). While the server holds a
    /// DISABLE_INITIATION it accepted, the node initiates nothing but I-Am
    /// answers to Who-Is and audit notifications: the server's own requests,
    /// notifications and [`broadcast_i_am`](BACnetServer::broadcast_i_am),
    /// the full client's requests and unconfirmed sends, the narrow
    /// [`client`](Self::client)'s requests and the session's own
    /// [`broadcast_i_am`](Self::broadcast_i_am) fail with
    /// [`Error::InitiationDisabled`]
    /// with `sent: false` and send nothing, and a client request already
    /// waiting ends with `sent: true` when its retry falls due. Answers, and network-layer
    /// messages, still go. It ends when the control's duration runs out, an
    /// ENABLE or an accepted WARMSTART or COLDSTART ReinitializeDevice is
    /// accepted, or the server stops or is dropped. A full client its
    /// operator started with
    /// [`ClientOptions::with_node_dcc_override`](bacnet_client::client::ClientOptions::with_node_dcc_override)
    /// keeps initiating; the session logs a warning then.
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
        let seated = self.server_seat().map(|seat| {
            let builder = builder.seat(seat);
            match &self.identity {
                Some(identity) => builder.node_identity(identity.clone()),
                None => builder,
            }
        });
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
