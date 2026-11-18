//! The full `BACnetServer` the session builds on its own link (#2617).
use super::*;
use bacnet_endpoint_core::seat::{NodeDevice, ServerSeat, ServerSeatCounters};
use bacnet_server::mutation::MutationTarget;
use bacnet_server::policy::FnPolicy;
use bacnet_server::server::NodeLink;
use bacnet_server::server::{BACnetServer, ServerBuilder};

/// Where the session's source Audit is, once it is: the seated server's
/// observer of accepted ReinitializeDevice requests reads it.
pub(super) type SourceSlot =
    Arc<std::sync::OnceLock<std::sync::Weak<crate::source_audit::SourceAudit>>>;

impl<T: TransportPort + 'static> EndpointSession<T> {
    /// Have [`start`](Self::start) build the session's server from
    /// `builder` (before start). A session with a server role always runs a
    /// full [`BACnetServer`] on its link: one transport, one address, one
    /// pool of invoke IDs, beside the session's client roles. Without this
    /// call it is the read-only one, [`read_only_server`]: peers read,
    /// discover and subscribe to COV, and no peer writes, creates, deletes,
    /// reinitializes, controls communication or sets the clock (#2617).
    /// The session owns it; [`server`](Self::server) borrows it while the
    /// session runs, and [`stop`](Self::stop) stops it first.
    ///
    /// It is the server a transport of its own gives, with every service,
    /// policy and counter that one has, and `builder`'s settings as given:
    /// its policies (a builder with none lets any peer write, create and
    /// delete, as a standalone server does; one from [`read_only_server`]
    /// lets none), budgets, handlers, COV policy, device bindings and clock
    /// zone. It serves the session's database, which the session hands it
    /// at start ([`with_database`](Self::with_database), or a stock one
    /// built from the identity).
    ///
    /// **One device.** The node has one set of device settings, the
    /// session's, and the server serves them as the node's Device:
    ///
    /// - the session's [`DeviceIdentity`](crate::identity::DeviceIdentity),
    ///   with Max_APDU_Length_Accepted the smaller of the identity's and the
    ///   client role's [`SessionConfig::max_apdu_length`] (Clause 12.11.18),
    ///   resolved to the link. A builder identity that is not the default
    ///   is compared with it as the link serves both (after each is
    ///   resolved to the link) and leaving out the service list, which the
    ///   full server builds itself: one that differs otherwise is refused;
    /// - APDU_Timeout and Number_Of_APDU_Retries are
    ///   [`SessionConfig::apdu_timeout_ms`] and
    ///   [`SessionConfig::apdu_retries`] (default 6000 ms and 3), which the
    ///   client roles use and the server's confirmed notifications use too.
    ///   A builder that set other values, or a database whose Device
    ///   declares others, is refused; a Device that declares the object
    ///   model's defaults (6000 ms and 3, as one built from an identity
    ///   does) is given the session's;
    /// - on a [`SessionRole::Both`] session, Protocol_Services_Supported
    ///   lists what the node's client executes (I-Am, I-Have, both COV
    ///   notifications) beside the server's services (Clause 12.11.14);
    /// - Segmentation_Supported, Max_Segments_Accepted and
    ///   APDU_Segment_Timeout are the identity's, which the server follows.
    ///   A [`full_client`](Self::full_client) keeps its own segmentation
    ///   settings for now (a later step of #2617): it accepts segmented
    ///   answers even where the Device says it receives none, and it
    ///   aborts a segmented ConfirmedCOVNotification, which it can't
    ///   reassemble;
    /// - a registered Network Port ([`with_registered_network_port`](Self::with_registered_network_port))
    ///   is the session's to reserve and publish, and the server serves it
    ///   for wildcard reads; written values take effect through the
    ///   builder's ReinitializeDevice handler.
    ///
    /// **Refused at start**, before the link starts, leaving the session
    /// ready to correct and start again with its database: a builder on a
    /// session without a server role ([`SessionRole::ClientOnly`]); a
    /// server role, with this builder or the default, without a Device
    /// identity ([`device`](Self::device)); a builder with a
    /// transport, a database (give it to the session) or a registered
    /// Network Port (name it on the session) of its own, or with device
    /// settings that contradict the session's as above; a builder whose
    /// settings a server's start would refuse, which the session checks as
    /// the server will get them (policies and budgets, device bindings,
    /// Audit Reporters and their routes, the Device's segment timeout); and,
    /// beside a source Audit Reporter
    /// ([`with_source_audit_reporter`](Self::with_source_audit_reporter)),
    /// a builder with Audit Reporters of its own (the Device has one Audit
    /// recipient) or one that decides no mutations: give it a policy whose
    /// [`decides_mutations`](bacnet_server::ServerPolicy::decides_mutations)
    /// is `true`, or [`MutationPolicy::DenyAll`](bacnet_server::mutation::MutationPolicy::DenyAll),
    /// so that no peer rewrites the Device's Audit_Notification_Recipient
    /// by default. `decides_mutations` is `true` unless a policy says
    /// otherwise, and so is `authorize_mutation`: a policy of your own that
    /// overrides neither passes this check and lets any peer rewrite the
    /// recipient. Decide the recipient write in `authorize_mutation`.
    ///
    /// One check needs the started link, and comes after it: target Audit's
    /// recipient as the started link sees it, an Address that names this
    /// network by its number or a group address only the bound link
    /// reveals. Its refusal ends the session and drops the database the
    /// server was given.
    ///
    /// **Source Audit beside the server.** The session's source Audit
    /// sends its records through the server's pool of notifications, which
    /// completes and joins them; the server reports each ReinitializeDevice
    /// it accepts to the session's Reporter, which records it with its
    /// requester. A peer's write of the Device's Audit_Notification_Recipient
    /// that the policy allows is recorded with its requester as the source.
    ///
    /// **From the first request.** The server's seat is attached as the
    /// link starts, before its first request is classified, and the server
    /// takes what waits there once the session's start is done, so no
    /// request goes unanswered while the server builds. A start that fails after the server is built
    /// stops it; one cancelled there leaves it holding what waits,
    /// unanswered, until [`stop`](Self::stop) or drop. A server build that
    /// fails or is cancelled drops the database it was given.
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
    /// **One pool of invoke IDs.** The server and the client roles lease
    /// from the session's one pool of 256. The server holds at most 64 at
    /// once: its confirmed notifications (COV, event and Audit, the source
    /// Audit's records among them) and the reads its runs make. Past that,
    /// each does what it does when the pool is empty: an event notification
    /// waits for an ID, a COV notification's subscription waits for one of
    /// the server's IDs to come back and is then looked at again, an Audit
    /// record is counted lost. The client's requests find the rest.
    ///
    /// **DeviceCommunicationControl binds the node.** The standard has it
    /// bind the whole device (Clause 16.1). While the server holds a
    /// DISABLE_INITIATION it accepted, the node initiates nothing but I-Am
    /// answers to Who-Is and audit notifications, and nothing is sent: the
    /// server holds its own requests and notifications, and its
    /// [`broadcast_i_am`](BACnetServer::broadcast_i_am) fails with a
    /// SERVICES / COMMUNICATION_DISABLED [`Error::Remote`], as on a
    /// standalone server; the full client's requests and unconfirmed sends,
    /// the narrow [`client`](Self::client)'s requests and the session's own
    /// [`broadcast_i_am`](Self::broadcast_i_am) fail with
    /// [`Error::InitiationDisabled`] and `sent: false`. A client request
    /// already waiting ends with `sent: true` when its retry falls due.
    /// Answers, and network-layer messages, still go. It ends when the
    /// control's duration runs out, on an ENABLE or an accepted WARMSTART or
    /// COLDSTART ReinitializeDevice, or when the server stops or is dropped.
    /// A full client its operator started with
    /// [`ClientOptions::with_node_dcc_override`](bacnet_client::client::ClientOptions::with_node_dcc_override)
    /// keeps initiating; the session logs a warning then.
    pub fn with_server(mut self, builder: ServerBuilder<T>) -> Self {
        self.assert_configurable();
        self.server_builder = Some(builder);
        self
    }

    /// Borrows the session's server: the one [`with_server`](Self::with_server)
    /// had it build, or the read-only one. `Some` from the start that built
    /// it until a [`stop`](Self::stop) has stopped it: a start that failed
    /// after building it, or a stop, has stopped it too, while a cancelled
    /// start or stop leaves it `Some` until the next stop. `None` before
    /// start and for a session without a server role. For another task,
    /// clone what it needs from it ([`database`](BACnetServer::database),
    /// an `Arc`).
    pub fn server(&self) -> Option<&BACnetServer<T>> {
        self.server.as_ref()
    }

    /// What ingress handed to the full server's seat, and what it dropped
    /// because the server had left its queue full.
    pub fn server_seat_counters(&self) -> ServerSeatCounters {
        self.ingress
            .as_ref()
            .map(EndpointIngress::server_seat_counters)
            .unwrap_or_default()
    }

    /// The node's device settings, which a seated server serves.
    fn node_device(&self) -> NodeDevice {
        NodeDevice {
            client: matches!(self.role, SessionRole::Both),
            client_max_apdu_length: self.client_config.max_apdu_length,
            apdu_timeout_ms: self.config.apdu_timeout_ms,
            apdu_retries: self.config.apdu_retries,
        }
    }

    /// A session with a server role runs a full server: the read-only one
    /// ([`read_only_server`]) unless [`with_server`](Self::with_server)
    /// gave it a builder (#2617).
    pub(super) fn default_server(&mut self) {
        if self.server_builder.is_none()
            && matches!(self.role, SessionRole::ServerOnly | SessionRole::Both)
        {
            self.server_builder = Some(read_only_server());
        }
    }

    /// Refuse, before anything changes, a full server the session could
    /// not build (see [`with_server`](Self::with_server)), but for the
    /// device and server settings, which
    /// [`check_server_settings`](Self::check_server_settings) checks once
    /// the identity is resolved.
    pub(super) fn check_server_builder(&self) -> Result<(), Error> {
        let Some(builder) = &self.server_builder else {
            return Ok(());
        };
        let refused = |why: &str| Err(Error::Encoding(format!("no full server here: {why}")));
        if !matches!(self.role, SessionRole::ServerOnly | SessionRole::Both) {
            return refused("the session has no server role (ServerOnly or Both)");
        }
        if self.identity.is_none() {
            return refused(
                "a session with a server role runs a full server, which needs the node's \
                 Device identity, its one set of device settings: compose one with device(), \
                 or make a node that only asks a SessionRole::ClientOnly session (a link \
                 builder's role(), or the role EndpointSession::new takes)",
            );
        }
        let facts = builder.node_facts();
        if facts.transport {
            return refused("the builder has a transport: the server takes the session's link");
        }
        if facts.database {
            return refused(
                "the builder has a database: give it to the session (with_database), whose \
                 database the server serves",
            );
        }
        if facts.registered_network_port {
            return refused(
                "the builder names a registered Network Port: name it on the session \
                 (with_registered_network_port), which reserves and publishes it",
            );
        }
        if self.source_audit_reporter.is_some() && facts.audit_reporters {
            return refused(
                "the builder has Audit Reporters beside the session's source Audit Reporter: \
                 the Device has one Audit recipient",
            );
        }
        Ok(())
    }

    /// Refuse, once [`prepare_identity`](Self::prepare_identity) resolved
    /// the identity and made the database, a `with_server` the server's
    /// build would refuse without the started link: device settings that
    /// contradict the session's, and whatever every server start checks of
    /// its settings, as the seat will give them. The link is still
    /// unstarted, so the session stays ready to retry with its database.
    pub(super) async fn check_server_settings(&self) -> Result<(), Error> {
        let (Some(builder), Some(identity)) = (&self.server_builder, &self.identity) else {
            return Ok(());
        };
        // The guard first: nothing borrowed from the link is held across a
        // wait, so the start stays `Send`.
        let mut db = match &self.database {
            Some(db) => Some(db.write().await),
            None => None,
        };
        let ingress = self.ingress.as_ref().ok_or_else(missing_ingress)?;
        let is_group = |mac: &[u8]| ingress.is_group_destination(mac);
        let link = NodeLink {
            capacity: self.local_receive_capacity,
            is_group: &is_group,
            bip_broadcast: ingress.bip_broadcast_endpoint(),
        };
        builder.check_node(identity, self.node_device(), &link, db.as_deref_mut())
    }

    /// The start after its preflight ([`start`](Self::start)): the link,
    /// with the full server's seat attached before its first request is
    /// classified, the registered port, the server, then the roles. The
    /// server takes its requests once all of it is done (#2617).
    pub(super) async fn start_link(
        &mut self,
        source_routes: Option<crate::source_audit::recipient::SourceRoutes>,
    ) -> Result<(), Error> {
        let ingress = self.ingress.as_mut().ok_or_else(missing_ingress)?;
        // Routed source Audit bindings' routers learn from what comes from
        // their networks, whichever role takes it (#2371).
        if let Some(routers) = source_routes
            .as_ref()
            .map(|routes| Arc::clone(routes.routers()))
            .filter(|routers| !routers.is_empty())
        {
            ingress.observe_sources(Arc::new(move |received| routers.observe(received)));
        }
        let (receivers, seat) = match self.server_builder {
            Some(_) => {
                let coordinator = Arc::clone(&self.coordinator);
                let (receivers, seat) = ingress.start_with_server_seat(coordinator).await?;
                (receivers, Some(seat))
            }
            None => (ingress.start().await?, None),
        };
        #[cfg(test)]
        start_pause(|pauses| pauses.after_link).await;
        self.publish_registered_port(
            (receivers.bip_port.clone(), receivers.bip6_port.clone()),
            receivers.egress.local_network_number(),
        )
        .await?;
        let source = SourceSlot::default();
        if let Some(mut seat) = seat {
            seat.node = Some(self.node_device());
            self.build_server(seat, &source).await?;
        }
        #[cfg(test)]
        start_pause(|pauses| pauses.after_build).await;
        let bip_local_address = receivers.bip_local_address;
        // Source Audit installs itself under the database's guard, which a
        // started server may be holding a moment.
        let database = self
            .source_audit_reporter
            .and(self.database.as_ref().map(Arc::clone));
        let mut guard = match &database {
            Some(database) => Some(database.write().await),
            None => None,
        };
        let started = self.start_roles(receivers, source_routes, guard.as_deref_mut());
        drop(guard);
        started?;
        if let Some(audit) = &self.source_audit {
            let _ = source.set(Arc::downgrade(audit));
        }
        // The server takes its requests from here on. A start that failed
        // or was cancelled before this leaves them held until stop.
        self.server_gate = None;
        self.bip_local_address = bip_local_address;
        Ok(())
    }

    /// Build the server on `seat` from the session's builder, handing it
    /// the session's database, which the session then shares. The server
    /// takes nothing from its seat while the session holds its gate
    /// (`server_gate`), which the start lets go of once it is done, and a
    /// stop otherwise. `source` is where its observer of accepted
    /// ReinitializeDevice requests finds the session's source Audit.
    pub(super) async fn build_server(
        &mut self,
        seat: ServerSeat<T>,
        source: &SourceSlot,
    ) -> Result<(), Error> {
        let builder = self.server_builder.take().expect("checked server builder");
        let identity = self.identity.clone().expect("checked identity");
        let builder = match self.database.take() {
            Some(db) => match Arc::try_unwrap(db) {
                Ok(db) => builder.database(db.into_inner()),
                Err(db) => {
                    self.database = Some(db);
                    return Err(Error::Encoding(
                        "endpoint database is shared at start".into(),
                    ));
                }
            },
            None => builder,
        };
        let (gate, held) = oneshot::channel();
        let mut seated = builder
            .seat(seat)
            .node_identity(identity)
            .hold_requests_until(held);
        if let Some(oid) = self.registered_network_port {
            seated = seated.node_registered_port(oid);
        }
        if self.source_audit_reporter.is_some() {
            // The session's Reporter records an accepted reset, backup or
            // restore step with its requester (#1564). Weak, so the server
            // never keeps the source alive.
            let source = Arc::clone(source);
            seated = seated.on_accepted_reinitialize(Arc::new(move |db, operation, requester| {
                if let Some(source) = source.get().and_then(std::sync::Weak::upgrade) {
                    source.device_operation(db, operation, Some(requester));
                }
            }));
        }
        let server = seated.build().await?;
        self.database = Some(Arc::clone(server.database()));
        self.server = Some(server);
        self.server_gate = Some(gate);
        Ok(())
    }

    /// Stop the full server, first: its work joins and its saves settle
    /// while the link is still up. Kept through a cancelled stop, whose
    /// next call stops it again; its error is the stop's. A server still
    /// held by an unfinished start stops without taking what it holds.
    pub(super) async fn stop_server(&mut self) {
        if let Some(server) = self.server.as_mut() {
            if let Err(error) = server.stop().await {
                self.server_stop_error.get_or_insert(error);
            }
            self.server = None;
        }
        self.server_gate = None;
    }
}

/// The read-only full server (#2617): the stance a node takes when no one
/// has said who may write. A session with a server role and no builder of
/// its own ([`EndpointSession::with_server`]) runs it, and the Python
/// endpoints build their server from it unless given a `mutation_policy`.
/// Peers read, discover and subscribe to COV; no peer writes, creates or
/// deletes. Each covered mutation but the
/// three COV subscriptions is denied (SERVICES / SERVICE_REQUEST_DENIED,
/// skipped for an inbound WriteGroup), and time synchronization, local and
/// UTC (Clauses 16.7 and 16.8), is ignored. Everything the server's other
/// defaults deny (DeviceCommunicationControl, ReinitializeDevice without a
/// handler, LifeSafetyOperation, Audit notifications) stays denied. The
/// policy allows the SubscribeCOV family. What it doesn't decide keeps
/// the server's defaults and is executed for any peer: AcknowledgeAlarm
/// (Clause 13.5), received event notifications, which Notification
/// Forwarder objects pass on, a received I-Am, and text messages (Clauses
/// 16.5 and 16.6). With time synchronization off, Protocol_Services_Supported
/// lists neither TimeSynchronization nor UTCTimeSynchronization, as for any
/// server whose `TimeSyncPolicy` is disabled (Clause 12.11.14).
///
/// It is an ordinary builder: set budgets, handlers, a COV policy and the
/// rest on it as on any. A later `policy` call replaces the stance's
/// policy, and a `time_sync_policy` call its time synchronization setting.
pub fn read_only_server<T: TransportPort + 'static>() -> ServerBuilder<T> {
    BACnetServer::generic_builder()
        .policy(FnPolicy::new().inline().mutation(|context| {
            matches!(
                context.target,
                MutationTarget::SubscribeCov(_)
                    | MutationTarget::SubscribeCovProperty(_)
                    | MutationTarget::SubscribeCovPropertyMultiple(_)
            )
        }))
        .time_sync_policy(bacnet_server::server::TimeSyncPolicy {
            enabled: false,
            ..Default::default()
        })
}

fn missing_ingress() -> Error {
    Error::Encoding("endpoint session ingress owner is missing".into())
}

/// Pauses a test puts into a start (#2617), to widen the windows the start
/// must keep closed: right after the link starts, and right after the
/// server is built.
#[cfg(test)]
#[derive(Clone, Copy, Default)]
pub(crate) struct StartPauses {
    pub(crate) after_link: std::time::Duration,
    pub(crate) after_build: std::time::Duration,
}

#[cfg(test)]
tokio::task_local! {
    /// The pauses of the start this task runs, when a test sets them.
    pub(crate) static START_PAUSES: StartPauses;
}

#[cfg(test)]
async fn start_pause(which: fn(&StartPauses) -> std::time::Duration) {
    let pause = START_PAUSES.try_with(which).unwrap_or_default();
    if !pause.is_zero() {
        tokio::time::sleep(pause).await;
    }
}

#[cfg(test)]
#[path = "session_server_tests.rs"]
pub(super) mod tests;
