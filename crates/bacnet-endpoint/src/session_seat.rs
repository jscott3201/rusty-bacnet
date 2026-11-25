//! The session's client: a `BACnetClient` on the session's link (#2617).
use super::*;
use bacnet_client::client::{BACnetClient, ClientOptions};
use bacnet_endpoint_core::seat::{CLIENT_SEAT_QUEUE, ClientSeat, ClientSeatCounters, SeatRoles};

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// A seat for a client on this session's link: its network layer
    /// and invoke-ID coordinator, shared, and queues for what is the
    /// client's to receive.
    fn client_seat(&self) -> Result<ClientSeat<T>, Error> {
        if !self.is_running() {
            return Err(self.not_running());
        }
        let server = matches!(self.role, SessionRole::ServerOnly | SessionRole::Both);
        self.ingress
            .as_ref()
            .ok_or_else(server_seat::missing_ingress)?
            .client_seat(
                Arc::clone(&self.coordinator),
                SeatRoles::new(server),
                CLIENT_SEAT_QUEUE,
            )
    }

    /// The session's client: a [`BACnetClient`] on this session's link,
    /// beside its server role, with one transport, one address and one pool
    /// of invoke IDs.
    ///
    /// It is the client a transport of its own gives, with every call that
    /// one has: discovery, segmented reads, files, COV, router discovery,
    /// and, through `transport()`, the link's own calls and frame tap. It
    /// takes the session's client timers and APDU length, whatever role the
    /// session was made with. `bacnet_ops::Client::over` takes it as it is.
    /// See [`BACnetClient::start_on_seat`] for what a seat changes.
    ///
    /// The session must be running. One client at a time: another is
    /// refused until the first is dropped. The client is the caller's to
    /// keep, share behind an `Arc`, or move into a task.
    ///
    /// **Source Audit.** With a source Audit Reporter
    /// ([`with_source_audit_reporter`](Self::with_source_audit_reporter)),
    /// the client's requests of the services Table 19-5 maps to READ and
    /// WRITE are audited (#2617): ReadProperty, ReadPropertyMultiple,
    /// ReadRange, AtomicReadFile, AuditLogQuery, WriteProperty,
    /// WritePropertyMultiple, AtomicWriteFile, AddListElement and
    /// RemoveListElement. Each is admitted by the Reporter's settings and
    /// then owned by the session, so a caller that goes away loses neither
    /// the request nor its records. At most 64 are held at once, and at
    /// most 4 by one station on the link or by the stations of another
    /// network together (#2652). Past them a request waits for a slot in
    /// its caller's task, one whose station or network holds its 4 behind
    /// that one only, and its timeout starts once it is admitted. It waits
    /// out its turn at the destination (`min_request_interval_ms`) holding
    /// nothing and takes the turn with its slot. A waiting request takes a
    /// slot only as it is polled, so one its caller no longer polls holds
    /// none; an admitted one takes the node's database guard in its
    /// caller's task only when it is free at once, and otherwise waits for
    /// it in a task the session owns, so its caller never holds the guard
    /// (#2755). A write sent with a priority is a command at that
    /// priority, which the Audit_Priority_Filter applies to; one without a
    /// priority is not filtered by priority. An audited
    /// request goes to one station, direct or behind a router, and a raw
    /// request of an audited service that doesn't decode is refused before
    /// it is sent, a write whose value isn't whole application data among
    /// them. An unaudited write sends its value as given, as a standalone
    /// client does. Not audited yet: WriteGroup, subscriptions, object and
    /// device management, alarm acknowledgment and life safety. An
    /// operation cut off by a stop, of the session or of the client, is not
    /// recorded.
    ///
    /// **Segmentation.** The client does what the session's Device says
    /// (#2617). With an identity ([`device`](Self::device)) that receives
    /// segments (SEGMENTED_RECEIVE, or SEGMENTED_BOTH, the default) it
    /// accepts segmented answers and states in each request that it takes
    /// more than 64 segments; with one that transmits them
    /// (SEGMENTED_TRANSMIT or SEGMENTED_BOTH) it sends a request too long
    /// for one APDU as segments, and otherwise such a request fails with
    /// [`SegmentationError::LocalCannotTransmit`](bacnet_types::error::SegmentationError::LocalCannotTransmit)
    /// before anything is sent. Its segment timeout is the identity's
    /// APDU_Segment_Timeout, and it proposes a window of one segment. Its
    /// transfers and the server's run side by side: each segment,
    /// Segment-ACK and Abort goes to the role whose transaction it is, by
    /// its PDU type, which side sent it and the invoke ID the role leased,
    /// so one peer may hold a transaction with each role under the same
    /// invoke ID. Each role keeps its own reassembly limits (the client 64
    /// answers at once; the server 128 requests, 16 from one peer), so one
    /// role's transfers never take the other's room. A session without an
    /// identity leaves the client its own defaults: segmented both ways,
    /// with its APDU timeout as its segment timeout. When the Device
    /// receives segments, the client also reassembles a COV notification of
    /// either form that its sender segments, acknowledging each segment as
    /// the answering side, and then answers it as one that came whole
    /// (#2646); otherwise it aborts the first segment
    /// (SEGMENTATION_NOT_SUPPORTED). A requester's Abort ends such a
    /// transfer in the client and in the server alike.
    ///
    /// **Who answers what.** Beside a server role the client answers only
    /// the COV notifications of its own subscriptions; the server answers
    /// a ConfirmedEventNotification, and the client still publishes it to
    /// its `event_notifications()`. One that comes in segments the server
    /// reassembles and answers, and then hands the client whole (#2743), so
    /// the client publishes it once per transmission either way. A session with no server
    /// role leaves the client to answer as a standalone one does,
    /// acknowledgment included.
    ///
    /// **One device.** A DeviceCommunicationControl DISABLE_INITIATION a
    /// full server ([`with_server`](Self::with_server)) accepts binds this client too: its
    /// requests and unconfirmed sends fail with
    /// [`Error::InitiationDisabled`]
    /// until the control ends (see [`with_server`](Self::with_server)). An
    /// operator's own tool may start its client outside the node's control
    /// with [`client_with`](Self::client_with) and
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
    /// the client is still alive or not (#2617): it ends the client's
    /// tasks, fails its open requests and stops the link. From then on the
    /// client sends nothing: each request and each Who-Is or other
    /// unconfirmed send fails at once, as its seat has
    /// closed, and what goes through its `transport()` itself, such as a B/IP
    /// BVLC call, fails with the transport's stopped error. Dropping the
    /// client after that changes nothing, and the client's own
    /// [`BACnetClient::stop`] is not needed.
    ///
    /// Dropping the session without a stop ends the client the same way
    /// and aborts the link ([`TransportPort::abort`]), even while the client
    /// still holds it: nothing graceful is sent (no BACnet/SC disconnect,
    /// for one) and nothing is waited for. The link's sockets close as the
    /// runtime drops its aborted tasks, so a B/IP port binds again shortly
    /// after the drop.
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
    /// let client = session.client()?;
    /// client.who_is(Broadcast::Global, None).await?;
    /// session.stop().await?;
    /// // The client is still alive; its calls now fail.
    /// assert!(client.who_is(Broadcast::Global, None).await.is_err());
    /// # Ok(())
    /// # }
    /// ```
    pub fn client(&self) -> Result<BACnetClient<T>, Error> {
        self.client_with(ClientOptions::default())
    }

    /// [`Self::client`], with the client's startup `options`: its
    /// notification channel capacities, how it answers a confirmed COV
    /// notification, and whether its operator starts it outside the node's
    /// DeviceCommunicationControl ([`ClientOptions::with_node_dcc_override`]).
    pub fn client_with(&self, options: ClientOptions) -> Result<BACnetClient<T>, Error> {
        let mut config = self.client_config.clone();
        // The session keeps the Device's raw ceiling; a request states the
        // largest standard length at or below it.
        config.max_apdu_length =
            bacnet_encoding::apdu::max_apdu_header_at_or_below(u32::from(config.max_apdu_length))?;
        let seat = self.client_seat()?;
        match &self.source_audit {
            Some(source) => {
                let auditor = crate::source_audit::SeatAuditor(Arc::downgrade(source));
                BACnetClient::start_on_audited_seat(config, seat, options, Arc::new(auditor))
            }
            None => BACnetClient::start_on_seat(config, seat, options),
        }
    }

    /// What ingress handed to the client's seat, and what it dropped
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
