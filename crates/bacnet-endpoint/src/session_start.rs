//! Post-ingress composition publishes each cleanup owner before fallible work.
use super::*;
impl<T: TransportPort + 'static> EndpointSession<T> {
    pub(super) fn start_roles(
        &mut self,
        receivers: bacnet_endpoint_core::endpoint_ingress::IngressReceivers,
        mut source_routes: Option<crate::source_read::recipient::SourceRoutes>,
        device_write_target: Option<ObjectIdentifier>,
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
        let (source_read, source_recipient) = if let Some(selected) = self.source_audit_reporter {
            let broadcast = receivers.bip_broadcast_endpoint.ok_or_else(|| {
                Error::Encoding("source Audit lost its B/IP capability at startup".into())
            })?;
            let mut routes = source_routes.take().expect("preflight routes");
            routes.finalize(broadcast);
            let (source, recipient) = crate::source_read::SourceRead::new(
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
        self.source_read = source_read.clone();
        self.source_recipient = source_recipient.clone();
        let (requester, client_handle) =
            if matches!(self.role, SessionRole::ClientOnly | SessionRole::Both) {
                let requester = EndpointRequester::new(
                    egress.clone(),
                    Arc::clone(&self.coordinator),
                    self.client_config.clone(),
                )?;
                let mut handle = ClientRoleHandle::new(&self.shared.token, requester.clone());
                if let Some(source) = &source_read {
                    handle = handle.with_source_read(source);
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
                let mut responder = EndpointResponder::new(Arc::clone(db), egress.clone());
                if let Some(device) = device_write_target {
                    responder = responder.with_device_writes(
                        device,
                        self.device_write_authorizer
                            .clone()
                            .expect("validated authorizer"),
                    );
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
        };
        let audit_lease = source_recipient
            .as_ref()
            .map(|runtime| Arc::clone(&runtime.owner));
        let task = tokio::spawn(async move {
            let _audit_lease = audit_lease;
            dispatch_loop(dispatch, cancel_rx).await
        });

        // Dispatch owns the three ingress receivers (single consumer); the
        // session retains one egress clone for the identity I-Am path while
        // the receiver halves move into dispatch. No second demultiplexer.
        self.egress = Some(egress);
        self.source_read = source_read;
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
