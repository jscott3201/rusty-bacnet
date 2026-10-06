//! Post-ingress composition publishes each cleanup owner before fallible work.
use super::*;
impl<T: TransportPort + 'static> EndpointSession<T> {
    pub(super) fn start_roles(
        &mut self,
        receivers: bacnet_endpoint_core::endpoint_ingress::IngressReceivers,
        mut source_routes: Option<crate::source_audit::recipient::SourceRoutes>,
        device_target: Option<ObjectIdentifier>,
    ) -> Result<(), Error> {
        let egress = receivers.egress.clone();
        self.egress = Some(egress.clone());
        // Role registration shares ONE coordinator + ONE egress. No second
        // demultiplexer, no role-side Invoke-ID allocation: the requester and
        // notification pool reserve from `self.coordinator`; the responder
        // reuses the wire invoke ID directly.
        self.notifications = (matches!(self.role, SessionRole::ServerOnly | SessionRole::Both)
            || self.source_audit_reporter.is_some())
        .then(|| NotificationTransactions::with_coordinator(Arc::clone(&self.coordinator)));
        let notifications = self.notifications.clone();
        let (source_audit, source_recipient) = if let Some(selected) = self.source_audit_reporter {
            let broadcast = receivers.bip_broadcast_endpoint.ok_or_else(|| {
                Error::Encoding("source Audit lost its B/IP capability at startup".into())
            })?;
            let mut routes = source_routes.take().expect("preflight routes");
            routes.finalize(broadcast, egress.local_network_number().clone());
            let (source, recipient) = crate::source_audit::SourceAudit::new(
                Arc::clone(self.database.as_ref().expect("validated source database")),
                selected,
                routes,
                *broadcast.ip(),
                egress.clone(),
                notifications.as_ref().expect("source worker owner"),
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
        let (responder, server_handle) =
            if matches!(self.role, SessionRole::ServerOnly | SessionRole::Both) {
                let db = self
                    .database
                    .get_or_insert_with(|| Arc::new(RwLock::new(ObjectDatabase::new())));
                let mut responder = EndpointResponder::new(Arc::clone(db), egress.clone())
                    .with_read_work_limit(self.config.read_work_limit);
                if let Some(oid) = self.registered_network_port {
                    responder =
                        responder.with_registered_port(oid, self.registered_port_lease.clone());
                }
                if let (Some(device), Some(authorizer)) =
                    (device_target, self.device_write_authorizer.clone())
                {
                    responder = responder.with_device_writes(device, authorizer);
                }
                if let Some(handler) = self.reinitialize.clone() {
                    responder = responder.with_reinitialize(handler, self.reinit_password.clone());
                }
                let responder = Arc::new(responder);
                let handle = ServerRoleHandle::new(
                    &self.shared.token,
                    Arc::clone(&responder),
                    Arc::clone(notifications.as_ref().expect("server worker owner")),
                );
                (Some(responder), Some(handle))
            } else {
                (None, None)
            };

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
            tokio::spawn(async move {
                let _registration_lease = registration_lease;
                while let Some(control) = controls.recv().await {
                    let before = egress.local_network_number().get();
                    let reply = owner.handle(control).await;
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
                    if let Some(npdu) = reply {
                        if let Err(error) = egress.send_network_number_is(npdu).await {
                            tracing::debug!(%error, "Network-Number-Is broadcast failed");
                        }
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
            responder: responder.clone(),
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
        self.responder = responder;
        self.notifications = notifications;
        self.client_handle = client_handle;
        self.server_handle = server_handle;
        self.dispatch_task = Some(task);
        self.cancel_tx = Some(cancel_tx);
        Ok(())
    }
}
