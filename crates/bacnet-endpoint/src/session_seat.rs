//! A full `BACnetClient` on the session's link (#2617).
use super::*;
use bacnet_client::client::{BACnetClient, ClientOptions};
use bacnet_endpoint_core::seat::{CLIENT_SEAT_QUEUE, ClientSeat, ClientSeatCounters, SeatRoles};

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// A seat for a full client on this session's link: its network layer
    /// and invoke-ID coordinator, shared, and queues for what is the
    /// client's to receive. [`Self::full_client`] makes the seat and the
    /// client on it; take the seat to start the client yourself, with
    /// `BACnetClient::generic_builder()`'s `seat` or
    /// [`BACnetClient::start_on_seat`].
    ///
    /// The session must be running. One full client at a time: a second
    /// seat is refused until the first one's client is dropped. A session
    /// with a source Audit Reporter refuses a seat: the full client's
    /// requests are not audited, so none may leave beside audited ones.
    pub fn client_seat(&self) -> Result<ClientSeat<T>, Error> {
        if self.source_audit_reporter.is_some() {
            return Err(Error::Encoding(
                "a session with a source Audit Reporter has no full client: its requests \
                 would not be audited"
                    .into(),
            ));
        }
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
    /// See [`BACnetClient::start_on_seat`] for what a seat changes, and
    /// [`Self::client_seat`] for when one is refused.
    ///
    /// Drop the client before [`stop`](Self::stop): the client keeps the
    /// link alive, so a stop while it lives closes the session's roles and
    /// the client's tasks, fails with an error that says so, and finishes
    /// when called again after the client is dropped. Until then the link
    /// is still up, and what that client sends still goes out.
    ///
    /// ```no_run
    /// use bacnet_endpoint::session::{EndpointSession, SessionConfig, SessionRole};
    /// use bacnet_transport::loopback::LoopbackTransport;
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), bacnet_types::error::Error> {
    /// let (transport, _peer) = LoopbackTransport::pair(vec![0x01], vec![0x02]);
    /// let mut session =
    ///     EndpointSession::new(transport, SessionRole::Both, SessionConfig::default())?;
    /// session.start().await?;
    /// let mut client = session.full_client()?;
    /// client.who_is(None).await?;
    /// client.stop().await?;
    /// drop(client);
    /// session.stop().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn full_client(&self) -> Result<BACnetClient<T>, Error> {
        BACnetClient::start_on_seat(
            self.client_config.clone(),
            self.client_seat()?,
            ClientOptions::default(),
        )
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
