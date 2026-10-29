//! Shared transport preparation for concrete and transport-erased clients.
use super::*;
use bacnet_transport::any::AnyTransport;

impl BipClientBuilder {
    fn prepare(self) -> Result<(ClientConfig, BipTransport, ClientOptions), Error> {
        let mut transport = BipTransport::new(
            self.config.interface,
            self.config.port,
            self.config.broadcast_address,
        );
        transport.set_share_port_by_address(self.config.share_port_by_address);
        transport.set_mode(self.mode);
        transport.set_multicast(self.multicast)?;
        Ok((self.config, transport, self.options))
    }

    /// Build and start the client, constructing a BipTransport from the config.
    pub async fn build(self) -> Result<BACnetClient<BipTransport>, Error> {
        Box::pin(async move {
            let (config, transport, options) = self.prepare()?;
            Box::pin(BACnetClient::start_with_options(config, transport, options)).await
        })
        .await
    }

    /// Build and start a configured B/IP client with a transport-erased type.
    ///
    /// Uses the same settings and validation as [`Self::build`], and returns
    /// a [`BACnetClient`] over its default transport type, [`AnyTransport`].
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let mut client: BACnetClient = BACnetClient::bip_builder()
    ///     .port(0)
    ///     .apdu_retries(2)
    ///     .build_any().await?;
    /// client.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn build_any(self) -> Result<BACnetClient<AnyTransport>, Error> {
        self.build_into().await
    }

    /// Build and start a configured B/IP client over any transport type a
    /// [`BipTransport`] converts into, such as an application's own enum of
    /// transports (#2071). [`Self::build_any`] is this with [`AnyTransport`].
    pub async fn build_into<T>(self) -> Result<BACnetClient<T>, Error>
    where
        T: TransportPort + From<BipTransport> + 'static,
    {
        Box::pin(async move {
            let (config, transport, options) = self.prepare()?;
            Box::pin(BACnetClient::start_with_options(
                config,
                transport.into(),
                options,
            ))
            .await
        })
        .await
    }
}

#[cfg(feature = "ipv6")]
impl Bip6ClientBuilder {
    fn prepare(self) -> (ClientConfig, Bip6Transport, ClientOptions) {
        let mut transport =
            Bip6Transport::new(self.interface, self.config.port, self.device_instance);
        if let Some(foreign) = self.foreign_device {
            transport.register_as_foreign_device(foreign);
        }
        (self.config, transport, self.options)
    }

    /// Build and start the client, constructing a Bip6Transport from the config.
    pub async fn build(self) -> Result<BACnetClient<Bip6Transport>, Error> {
        Box::pin(async move {
            let (config, transport, options) = self.prepare();
            Box::pin(BACnetClient::start_with_options(config, transport, options)).await
        })
        .await
    }

    /// Build and start a configured B/IPv6 client with a transport-erased type.
    ///
    /// Uses the same interface, device identity and client options as
    /// [`Self::build`], over the default transport type, [`AnyTransport`].
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// # async fn example() -> Result<(), bacnet_types::error::Error> {
    /// let mut client: BACnetClient = BACnetClient::bip6_builder()
    ///     .device_instance(123)
    ///     .build_any().await?;
    /// client.stop().await?;
    /// # Ok(()) }
    /// ```
    pub async fn build_any(self) -> Result<BACnetClient<AnyTransport>, Error> {
        self.build_into().await
    }

    /// Build and start a configured B/IPv6 client over any transport type a
    /// [`Bip6Transport`] converts into (#2071). [`Self::build_any`] is this
    /// with [`AnyTransport`].
    pub async fn build_into<T>(self) -> Result<BACnetClient<T>, Error>
    where
        T: TransportPort + From<Bip6Transport> + 'static,
    {
        Box::pin(async move {
            let (config, transport, options) = self.prepare();
            Box::pin(BACnetClient::start_with_options(
                config,
                transport.into(),
                options,
            ))
            .await
        })
        .await
    }
}

#[cfg(feature = "sc-tls")]
impl ScClientBuilder {
    pub(super) fn validate(&self) -> Result<(), Error> {
        self.npdu_admission_policy.validate()?;
        if let Some(config) = &self.reconnect {
            config.validate()?;
        }
        if self.failover_hub_url.is_some() && self.reconnect.is_none() {
            // Without reconnection a lost connection is never re-established,
            // so the failover hub would never be reached or left again.
            return Err(Error::Encoding(
                "SC client builder: failover_hub_url requires reconnect".into(),
            ));
        }
        self.validate_identity()?;
        self.options.validate()?;
        self.config.validate_request_settings()?;
        self.config.validate_window()
    }

    async fn prepare(
        self,
    ) -> Result<
        (
            ClientConfig,
            bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>,
            ClientOptions,
        ),
        Error,
    > {
        self.validate()?;
        let tls_config =
            self.tls_config.as_ref().cloned().ok_or_else(|| {
                Error::Encoding("SC client builder: tls_config is required".into())
            })?;
        use bacnet_transport::sc::ScActiveHub;
        use bacnet_transport::sc_tls::TlsWebSocket;
        TlsWebSocket::validate_url(&self.hub_url)?;
        if let Some(url) = &self.failover_hub_url {
            TlsWebSocket::validate_url(url)?;
        }
        // An unreachable primary starts the client on the failover hub; the
        // transport restores the primary once it answers (Annex AB.5.2).
        // Bounded like the transport's own redials: a blackholed or stalled
        // hub must not hold up build() or the failover fallback.
        let limit = std::time::Duration::from_millis(self.connect_timeout_ms);
        let dial = |url: &str| {
            let tls = tls_config.clone();
            let url = url.to_owned();
            async move {
                bacnet_transport::sc::dial_within(limit, Box::pin(TlsWebSocket::connect(&url, tls)))
                    .await
            }
        };
        let (ws, hub) = match dial(&self.hub_url).await {
            Ok(ws) => (ws, ScActiveHub::Primary),
            Err(primary) => match &self.failover_hub_url {
                Some(url) => {
                    // No transport (or event stream) exists yet to report it.
                    tracing::warn!(error = %primary, "BACnet/SC primary hub dial failed; trying the failover hub");
                    match dial(url).await {
                        Ok(ws) => (ws, ScActiveHub::Failover),
                        Err(_) => return Err(primary),
                    }
                }
                None => return Err(primary),
            },
        };
        let mut transport = self.sc_transport(ws).with_initial_hub(hub);
        if let Some(failover_url) = self.failover_hub_url {
            let tls_config = tls_config.clone();
            transport = transport.with_failover_connector(move || {
                let failover_url = failover_url.clone();
                let tls_config = tls_config.clone();
                async move {
                    bacnet_transport::sc_tls::TlsWebSocket::connect(&failover_url, tls_config).await
                }
            });
        }
        if let Some(rc) = self.reconnect {
            let hub_url = self.hub_url;
            transport = transport
                .with_connector(move || {
                    let hub_url = hub_url.clone();
                    let tls_config = tls_config.clone();
                    async move {
                        bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config).await
                    }
                })
                .with_reconnect(rc);
        }
        Ok((self.config, transport, self.options))
    }

    /// Connect to the hub and start the client.
    ///
    /// NPDU admission and reconnect configuration are validated before TLS lookup or dialing, and
    /// again when the transport starts. An error still consumes this builder
    /// and drops its inputs; this does not promise generic endpoint rollback.
    ///
    /// The client's [`transport()`](BACnetClient::transport) lends the hub
    /// connection-state watch, hub-connector events and link status, accepted
    /// Hub certificate observation and NPDU drop counts:
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::sc::{ScConnectionState, ScEvent, ScReconnectConfig};
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    ///
    /// async fn watch_hub(tls: ScNodeTlsConfig) -> Result<(), bacnet_types::error::Error> {
    ///     let client = BACnetClient::sc_builder()
    ///         .hub_url("wss://hub.example.com/bacnet")
    ///         .failover_hub_url("wss://failover.example.com/bacnet")
    ///         .reconnect(ScReconnectConfig::default())
    ///         .tls_config(tls)
    ///         .vmac([0x02, 0, 0, 0, 0, 0x01])
    ///         .device_uuid([0x42; 16])
    ///         .build()
    ///         .await?;
    ///     // Owned receivers: they can live in their own tasks.
    ///     let mut state = client.transport().connection_state_changes();
    ///     tokio::spawn(async move {
    ///         while state.changed().await.is_ok() {
    ///             let _connected = *state.borrow_and_update() == ScConnectionState::Connected;
    ///         }
    ///     });
    ///     let mut events = client.transport().connection_events();
    ///     tokio::spawn(async move {
    ///         use tokio::sync::broadcast::error::RecvError;
    ///         loop {
    ///             match events.recv().await {
    ///                 Ok(event) => {
    ///                     let _failover = matches!(event, ScEvent::FailoverEngaged);
    ///                 }
    ///                 // Fell behind: the oldest events were dropped, not waited for.
    ///                 Err(RecvError::Lagged(_)) => continue,
    ///                 Err(RecvError::Closed) => break,
    ///             }
    ///         }
    ///     });
    ///     // Active hub, hub VMAC and heartbeat timing: poll when redrawing.
    ///     let _status = client.transport().connection_status();
    ///     // Owned historical TLS data, present only for an accepted active Hub.
    ///     let _peer = client.transport().hub_peer_info().await;
    ///     // A snapshot: poll it as often as needed.
    ///     let _full_queue_drops = client.transport().npdu_drop_counts().full_drops;
    ///     Ok(())
    /// }
    /// ```
    pub async fn build(
        self,
    ) -> Result<
        BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
        Error,
    > {
        Box::pin(async move {
            let (config, transport, options) = Box::pin(self.prepare()).await?;
            Box::pin(BACnetClient::start_with_options(config, transport, options)).await
        })
        .await
    }

    /// Connect to the hub and start the client, as [`build`](Self::build)
    /// does, and also return a hub-connector event receiver subscribed
    /// before the client started.
    ///
    /// The first [`ScEvent::Connected`](bacnet_transport::sc::ScEvent::Connected)
    /// goes out while `build` runs, before a caller of
    /// `transport().connection_events()` can subscribe. This receiver is
    /// already subscribed then, so it holds that event, followed by every
    /// later one, with the same lag rules as `connection_events()`.
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::sc::ScEvent;
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    ///
    /// async fn first_event(tls: ScNodeTlsConfig) -> Result<(), bacnet_types::error::Error> {
    ///     let (client, mut events) = BACnetClient::sc_builder()
    ///         .hub_url("wss://hub.example.com/bacnet")
    ///         .tls_config(tls)
    ///         .vmac([0x02, 0, 0, 0, 0, 0x01])
    ///         .device_uuid([0x42; 16])
    ///         .build_with_events()
    ///         .await?;
    ///     let first = events.recv().await;
    ///     assert!(matches!(first, Ok(ScEvent::Connected { .. })));
    ///     drop(client);
    ///     Ok(())
    /// }
    /// ```
    pub async fn build_with_events(
        self,
    ) -> Result<
        (
            BACnetClient<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
            tokio::sync::broadcast::Receiver<bacnet_transport::sc::ScEvent>,
        ),
        Error,
    > {
        Box::pin(async move {
            let (config, transport, options, events) = Box::pin(self.prepare_subscribed()).await?;
            let client =
                Box::pin(BACnetClient::start_with_options(config, transport, options)).await?;
            Ok((client, events))
        })
        .await
    }

    /// [`prepare`](Self::prepare), then subscribe to the transport's events
    /// before anything starts it. The first `Connected` goes out while the
    /// client starts, so every `*_with_events` build goes through here.
    async fn prepare_subscribed(
        self,
    ) -> Result<
        (
            ClientConfig,
            bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>,
            ClientOptions,
            tokio::sync::broadcast::Receiver<bacnet_transport::sc::ScEvent>,
        ),
        Error,
    > {
        let (config, transport, options) = Box::pin(self.prepare()).await?;
        let events = transport.connection_events();
        Ok((config, transport, options, events))
    }

    /// Connect to the hub and start a transport-erased BACnet/SC client.
    ///
    /// Shares [`Self::build`]'s validation, TLS policy, identity, heartbeat and
    /// optional reconnect configuration. Identity, NPDU admission, reconnect and client options
    /// are checked before dialing; heartbeat timing is checked when the transport
    /// starts. The client's type is the default, over [`AnyTransport`].
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::sc::ScReconnectConfig;
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    /// async fn connect(tls: ScNodeTlsConfig) -> Result<BACnetClient, bacnet_types::error::Error> {
    ///     BACnetClient::sc_builder()
    ///         .hub_url("wss://hub.example.com/bacnet")
    ///         .tls_config(tls)
    ///         .vmac([2, 0, 0, 0, 0, 1])
    ///         .device_uuid([0x42; 16])
    ///         .reconnect(ScReconnectConfig::default())
    ///         .build_any().await
    /// }
    /// ```
    pub async fn build_any(self) -> Result<BACnetClient<AnyTransport>, Error> {
        self.build_into().await
    }

    /// Connect to the hub and start a BACnet/SC client over any transport
    /// type an [`ScTransport`](bacnet_transport::sc::ScTransport) converts
    /// into (#2071). [`Self::build_any`] is this with [`AnyTransport`].
    pub async fn build_into<T>(self) -> Result<BACnetClient<T>, Error>
    where
        T: TransportPort
            + From<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>
            + 'static,
    {
        Box::pin(async move {
            let (config, transport, options) = Box::pin(self.prepare()).await?;
            Box::pin(BACnetClient::start_with_options(
                config,
                transport.into(),
                options,
            ))
            .await
        })
        .await
    }

    /// Connect to the hub and start a transport-erased BACnet/SC client, as
    /// [`build_any`](Self::build_any) does, and also return a hub-connector
    /// event receiver subscribed before the client started (#1909).
    ///
    /// As with [`build_with_events`](Self::build_with_events), the receiver
    /// holds the first
    /// [`ScEvent::Connected`](bacnet_transport::sc::ScEvent::Connected),
    /// which a later `connection_events()` call misses.
    ///
    /// ```no_run
    /// use bacnet_client::client::BACnetClient;
    /// use bacnet_transport::sc::ScEvent;
    /// use bacnet_transport::sc_tls::ScNodeTlsConfig;
    ///
    /// async fn first_event(tls: ScNodeTlsConfig) -> Result<(), bacnet_types::error::Error> {
    ///     let (client, mut events) = BACnetClient::sc_builder()
    ///         .hub_url("wss://hub.example.com/bacnet")
    ///         .tls_config(tls)
    ///         .vmac([0x02, 0, 0, 0, 0, 0x01])
    ///         .device_uuid([0x42; 16])
    ///         .build_any_with_events()
    ///         .await?;
    ///     assert!(matches!(events.recv().await, Ok(ScEvent::Connected { .. })));
    ///     drop(client);
    ///     Ok(())
    /// }
    /// ```
    pub async fn build_any_with_events(
        self,
    ) -> Result<
        (
            BACnetClient<AnyTransport>,
            tokio::sync::broadcast::Receiver<bacnet_transport::sc::ScEvent>,
        ),
        Error,
    > {
        self.build_into_with_events().await
    }

    /// [`Self::build_into`], also returning the hub-connector event receiver
    /// that [`Self::build_any_with_events`] returns (#2071).
    pub async fn build_into_with_events<T>(
        self,
    ) -> Result<
        (
            BACnetClient<T>,
            tokio::sync::broadcast::Receiver<bacnet_transport::sc::ScEvent>,
        ),
        Error,
    >
    where
        T: TransportPort
            + From<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>
            + 'static,
    {
        Box::pin(async move {
            let (config, transport, options, events) = Box::pin(self.prepare_subscribed()).await?;
            let client = Box::pin(BACnetClient::start_with_options(
                config,
                transport.into(),
                options,
            ))
            .await?;
            Ok((client, events))
        })
        .await
    }
}
