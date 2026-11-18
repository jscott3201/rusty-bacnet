//! A full `BACnetClient` on the session's link (#2617).
use super::*;
use bacnet_client::client::{BACnetClient, ClientOptions};
use bacnet_endpoint_core::seat::{CLIENT_SEAT_QUEUE, ClientSeat, ClientSeatCounters, SeatRoles};

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// A seat for a full client on this session's link: its network layer
    /// and invoke-ID coordinator, shared, and queues for what is the
    /// client's to receive.
    fn client_seat(&self) -> Result<ClientSeat<T>, Error> {
        if !self.is_running() {
            return Err(Error::Encoding("endpoint session is not running".into()));
        }
        let server = matches!(self.role, SessionRole::ServerOnly | SessionRole::Both);
        self.ingress
            .as_ref()
            .ok_or_else(|| Error::Encoding("endpoint session ingress owner is missing".into()))?
            .client_seat(
                Arc::clone(&self.coordinator),
                SeatRoles::new(server),
                CLIENT_SEAT_QUEUE,
            )
    }

    /// A full [`BACnetClient`] on this session's link, beside its server
    /// role and its narrow [`client`](Self::client) role: one transport, one
    /// address, one pool of invoke IDs.
    ///
    /// It is the client a transport of its own gives, with every call that
    /// one has: discovery, segmented reads, files, COV, router discovery,
    /// and, through `transport()`, the link's own calls and frame tap. It
    /// takes the session's client timers and APDU length, whatever role the
    /// session was made with. `bacnet_ops::Client::over` takes it as it is.
    /// See [`BACnetClient::start_on_seat`] for what a seat changes.
    ///
    /// The session must be running. One full client at a time: another is
    /// refused until the first is dropped.
    ///
    /// **Source Audit.** With a source Audit Reporter
    /// ([`with_source_audit_reporter`](Self::with_source_audit_reporter)),
    /// the client's requests of the services Table 19-5 maps to READ and
    /// WRITE are audited (#2617): ReadProperty, ReadPropertyMultiple,
    /// ReadRange, AtomicReadFile, AuditLogQuery, WriteProperty,
    /// WritePropertyMultiple, AtomicWriteFile, AddListElement and
    /// RemoveListElement. Each is admitted by the Reporter's settings and
    /// then owned by the session, so a caller that goes away loses neither
    /// the request nor its records. At most 64 are held at once; past them
    /// a request waits for a slot in its caller's task, and its timeout
    /// starts once it is admitted. A write sent with a priority is a
    /// command at that priority, which the Audit_Priority_Filter applies to;
    /// one without a priority is not filtered by priority. An audited
    /// request goes to one station, direct or behind a router, and a raw
    /// request of an audited service that doesn't decode is refused before
    /// it is sent. Not audited yet: WriteGroup, subscriptions, object and
    /// device management, alarm acknowledgment and life safety. An
    /// operation cut off by a stop, of the session or of the client, is not
    /// recorded.
    ///
    /// **Who answers what.** Beside a server role the client answers only
    /// the COV notifications of its own subscriptions. Today's narrow
    /// server role doesn't execute ConfirmedEventNotification, so the node
    /// answers one with a Reject (unrecognized service) while the client
    /// still publishes it to its `event_notifications()`; a notifier that
    /// needs the acknowledgment gets it from a full server
    /// ([`with_server`](Self::with_server)). A session with no server role
    /// leaves the client to answer as a standalone one does, acknowledgment
    /// included.
    ///
    /// **One device.** A DeviceCommunicationControl DISABLE_INITIATION a
    /// full server ([`with_server`](Self::with_server)) accepts binds this client too: its
    /// requests and unconfirmed sends fail with
    /// [`Error::InitiationDisabled`]
    /// until the control ends (see [`with_server`](Self::with_server)). An
    /// operator's own tool may start its client outside the node's control
    /// with [`full_client_with`](Self::full_client_with) and
    /// [`ClientOptions::with_node_dcc_override`], an override of Clause 16.1
    /// the session warns about. A segmented answer no request of the node's
    /// holds comes to this client, which aborts it as an idle client does
    /// (Clause 5.4.4.1).
    ///
    /// **A client that falls behind.** The session never waits for the
    /// client: what its seat's queue has no room for is dropped and counted
    /// in [`client_seat_counters`](Self::client_seat_counters). The queue
    /// has three tiers (#2635): copies of unconfirmed requests (I-Ams,
    /// Who-Is, unconfirmed notifications) fill at most half of it,
    /// confirmed requests to the client (its COV notifications) all but its
    /// last quarter, and answers to its own requests all of it. Segmented
    /// answers to nothing the node asked share the copies' half. A burst of
    /// I-Ams from its own discovery, or of such answers, then shuts out
    /// neither its COV notifications nor its answers.
    ///
    /// **Stopping.** One [`stop`](Self::stop) stops the session, whether
    /// the client is still alive or not (#2617): it closes the session's
    /// roles, ends the client's tasks, fails its open requests and stops
    /// the link. From then on the client sends nothing: each request and
    /// each Who-Is or other unconfirmed send fails at once, as its seat has
    /// closed, and what goes through its `transport()` itself, such as a B/IP
    /// BVLC call, fails with the transport's stopped error. Dropping the
    /// client after that changes nothing, and the client's own
    /// [`BACnetClient::stop`] is not needed.
    ///
    /// ```no_run
    /// use bacnet_client::client::Broadcast;
    /// use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
    /// use bacnet_transport::loopback::LoopbackTransport;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), bacnet_types::error::Error> {
    /// let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    /// let mut session =
    ///     EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())?;
    /// session.start().await?;
    /// let client = session.full_client()?;
    /// client.who_is(Broadcast::Global, None).await?;
    /// session.stop().await?;
    /// // The client is still alive; its calls now fail.
    /// assert!(client.who_is(Broadcast::Global, None).await.is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn full_client(&self) -> Result<BACnetClient<T>, Error> {
        self.full_client_with(ClientOptions::default())
    }

    /// [`Self::full_client`], with the client's startup `options`: its
    /// notification channel capacities, how it answers a confirmed COV
    /// notification, and whether its operator starts it outside the node's
    /// DeviceCommunicationControl ([`ClientOptions::with_node_dcc_override`]).
    pub fn full_client_with(&self, options: ClientOptions) -> Result<BACnetClient<T>, Error> {
        let seat = self.client_seat()?;
        let config = self.client_config.clone();
        match &self.source_audit {
            Some(source) => {
                let auditor = crate::source_audit::SeatAuditor(Arc::downgrade(source));
                BACnetClient::start_on_audited_seat(config, seat, options, Arc::new(auditor))
            }
            None => BACnetClient::start_on_seat(config, seat, options),
        }
    }

    /// What ingress handed to the full client's seat, and what it dropped
    /// because the client had left its queue full.
    pub fn client_seat_counters(&self) -> ClientSeatCounters {
        self.ingress
            .as_ref()
            .map(EndpointIngress::client_seat_counters)
            .unwrap_or_default()
    }
}

/// A seat nobody reads while a burst arrives (#2635).
#[cfg(test)]
#[path = "session_seat_tests.rs"]
mod tests;
