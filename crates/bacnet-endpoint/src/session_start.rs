//! Post-ingress composition publishes each cleanup owner before fallible work.
use super::*;
impl<T: TransportPort + 'static> EndpointSession<T> {
    pub(super) fn start_roles(
        &mut self,
        receivers: bacnet_endpoint_core::endpoint_ingress::IngressReceivers,
        mut source_routes: Option<crate::source_audit::recipient::SourceRoutes>,
        source_database: Option<&mut ObjectDatabase>,
    ) -> Result<(), Error> {
        let egress = receivers.egress.clone();
        self.egress = Some(egress.clone());
        // Role registration shares ONE coordinator + ONE egress. No second
        // demultiplexer, no role-side Invoke-ID allocation: the requester and
        // the source Audit's notification pool reserve from
        // `self.coordinator`. A full server's pool is its own, and the
        // source Audit sends through it (#2617): the server completes and
        // joins those records. A session with no server has a pool only for
        // its source Audit.
        let server_pool = self
            .server
            .as_ref()
            .map(|server| Arc::clone(server.notification_pool()));
        self.notifications = (server_pool.is_none() && self.source_audit_reporter.is_some())
            .then(|| NotificationTransactions::with_coordinator(Arc::clone(&self.coordinator)));
        let notifications = self.notifications.clone();
        let source_pool = server_pool.or_else(|| notifications.clone());
        let (source_audit, source_recipient) = if let Some(selected) = self.source_audit_reporter {
            use crate::source_audit::recipient::SourceLink;
            let mut routes = source_routes.take().expect("preflight routes");
            // The started link's facts: B/IP's bound broadcast endpoint, or
            // any other link's own MAC, with the egress's group rule (#2357).
            let (link, broadcast) = match receivers.bip_broadcast_endpoint {
                Some(broadcast) => (SourceLink::Bip(broadcast), *broadcast.ip()),
                None if routes.is_bip() => {
                    return Err(Error::Encoding(
                        "source Audit lost its B/IP capability at startup".into(),
                    ));
                }
                None => {
                    let groups = egress.clone();
                    let groups = bacnet_transport::port::GroupDestinations::new(move |mac| {
                        groups.is_group_destination(mac)
                    });
                    let link = SourceLink::Other {
                        local: receivers.local_mac.clone(),
                        groups,
                    };
                    (link, std::net::Ipv4Addr::UNSPECIFIED)
                }
            };
            routes.finalize(link, egress.local_network_number().clone());
            let (source, recipient) = crate::source_audit::SourceAudit::new(
                Arc::clone(self.database.as_ref().expect("validated source database")),
                source_database.expect("the source database's guard"),
                selected,
                routes,
                broadcast,
                egress.clone(),
                source_pool.as_ref().expect("source worker owner"),
                self.client_config.max_apdu_length,
            )?;
            (Some(source), Some(recipient))
        } else {
            (None, None)
        };
        self.source_audit = source_audit.clone();
        self.source_recipient = source_recipient.clone();
        let (requester, client_handle) =
            if matches!(self.role, SessionRole::ClientOnly | SessionRole::Both) {
                let requester = EndpointRequester::new(
                    egress.clone(),
                    Arc::clone(&self.coordinator),
                    self.client_config.clone(),
                )?;
                // DeviceCommunicationControl a full server on the node
                // accepts binds this requester too (#2617).
                if let Some(dcc) = self.ingress.as_ref().and_then(EndpointIngress::node_dcc) {
                    requester.follow_node_dcc(&dcc);
                }
                let mut handle = ClientRoleHandle::new(&self.shared.token, requester.clone())
                    .with_bip_broadcast(
                        receivers
                            .bip_broadcast_endpoint
                            .map(|address| *address.ip()),
                    );
                if let Some(source) = &source_audit {
                    handle = handle.with_source_audit(source);
                }
                (Some(requester), Some(handle))
            } else {
                (None, None)
            };

        // Routed source Audit bindings' routers, which router announcements
        // teach here, and traffic at ingress (#2371).
        let routers = source_recipient
            .as_ref()
            .map(|runtime| Arc::clone(runtime.routes.routers()))
            .filter(|routers| !routers.is_empty());
        self.network_number_task = receivers.network_controls.map(|mut controls| {
            let egress = egress.clone();
            let registration_lease = self.registered_port_lease.upgrade();
            // The owner copies its state into the layer's number slot after
            // each control, where the requester and source routes read it
            // (#1403).
            let mut owner = bacnet_server::network_number::NetworkNumberOwner::new(
                self.registered_network_port.map(|oid| {
                    (
                        Arc::clone(
                            self.database
                                .as_ref()
                                .expect("validated registered database"),
                        ),
                        oid,
                    )
                }),
            )
            .publishing_to(egress.local_network_number().clone());
            // A changed number may change whether the source recipient
            // resolves; its Reporter's health follows at once (#1461).
            let source = source_recipient.as_ref().map(Arc::downgrade);
            let database = self.database.as_ref().map(Arc::downgrade);
            // A full client on the seat runs no owner of its own: it folds
            // its table when this one has published (#2617).
            let seat_number = receivers.seat_number.clone();
            tokio::spawn(async move {
                let _registration_lease = registration_lease;
                use bacnet_server::network_number::NumberTurn;
                loop {
                    // A control, or the answer a broadcast question left
                    // waiting (#2597).
                    let control = match owner.next(&mut controls).await {
                        NumberTurn::Closed => break,
                        NumberTurn::Control(control) => control,
                        NumberTurn::Answer(answer) => {
                            if let Some(npdu) = answer
                                && let Err(error) = egress.send_network_number_is(npdu).await
                            {
                                tracing::debug!(%error, "Network-Number-Is broadcast failed");
                            }
                            continue;
                        }
                    };
                    if let Some(routers) = &routers {
                        routers.observe_control(&control);
                    }
                    let before = egress.local_network_number().get();
                    let reply = owner.handle(control).await;
                    if egress.local_network_number().get() != before {
                        seat_number.published();
                    }
                    let source = source.as_ref().and_then(std::sync::Weak::upgrade);
                    // Held across the lock wait below, so it goes off the
                    // runtime even when the task is aborted there (#1561).
                    let database = database
                        .as_ref()
                        .and_then(std::sync::Weak::upgrade)
                        .map(crate::held_database::HeldDatabase::new);
                    if let (true, Some(source), Some(database)) = (
                        egress.local_network_number().get() != before,
                        source,
                        database,
                    ) {
                        source.number_changed(&*database.read().await);
                    }
                    if let Some(npdu) = reply
                        && let Err(error) = egress.send_network_number_is(npdu).await
                    {
                        tracing::debug!(%error, "Network-Number-Is broadcast failed");
                    }
                }
            })
        });

        let (cancel_tx, cancel_rx) = oneshot::channel();
        let dispatch = DispatchParts {
            inbound: receivers.inbound_requests,
            terminal: receivers.terminal_or_segment,
            policy: receivers.policy_outcomes,
            requester: requester.clone(),
            notifications: notifications.clone(),
            coordinator: Arc::clone(&self.coordinator),
            shared: Arc::clone(&self.shared),
            local_network: egress.local_network_number().clone(),
        };
        let audit_lease = source_recipient
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.owner));
        let registration_lease = self.registered_port_lease.upgrade();
        let task = tokio::spawn(async move {
            let _registration_lease = registration_lease;
            let _audit_lease = audit_lease;
            dispatch_loop(dispatch, cancel_rx).await
        });

        // Dispatch owns the three ingress receivers (single consumer); the
        // session retains one egress clone for the identity I-Am path while
        // the receiver halves move into dispatch. No second demultiplexer.
        self.egress = Some(egress);
        self.source_audit = source_audit;
        self.source_recipient = source_recipient;
        self.requester = requester;
        self.notifications = notifications;
        self.client_handle = client_handle;
        self.dispatch_task = Some(task);
        self.cancel_tx = Some(cancel_tx);
        Ok(())
    }
}
