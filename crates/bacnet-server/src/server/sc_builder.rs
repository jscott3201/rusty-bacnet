use super::*;

impl BACnetServer<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>> {
    /// Create an SC-specific builder that connects to a BACnet/SC hub.
    pub fn sc_builder() -> ScServerBuilder {
        ScServerBuilder {
            config: ServerConfig::default(),
            db: ObjectDatabase::new(),
            configured_device_bindings: Vec::new(),
            hub_url: String::new(),
            tls_config: None,
            vmac: [0; 6],
            heartbeat_interval_ms: 30_000,
            heartbeat_timeout_ms: 60_000,
            reconnect: None,
        }
    }
}

/// SC-specific server builder.
///
/// Created by [`BACnetServer::sc_builder()`]. Requires the `sc-tls` feature.
pub struct ScServerBuilder {
    pub(super) config: ServerConfig,
    db: ObjectDatabase,
    pub(super) configured_device_bindings: Vec<DeviceBinding>,
    hub_url: String,
    tls_config: Option<std::sync::Arc<tokio_rustls::rustls::ClientConfig>>,
    vmac: bacnet_transport::sc_frame::Vmac,
    heartbeat_interval_ms: u64,
    heartbeat_timeout_ms: u64,
    reconnect: Option<bacnet_transport::sc::ScReconnectConfig>,
}

impl ScServerBuilder {
    /// Set the hub WebSocket URL (e.g. `wss://hub.example.com/bacnet`).
    pub fn hub_url(mut self, url: &str) -> Self {
        self.hub_url = url.to_string();
        self
    }

    /// Set the segmentation support this device advertises and enforces.
    pub fn segmentation_supported(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation_supported = segmentation;
        self
    }

    /// Set the TLS client configuration.
    pub fn tls_config(
        mut self,
        config: std::sync::Arc<tokio_rustls::rustls::ClientConfig>,
    ) -> Self {
        self.tls_config = Some(config);
        self
    }

    /// Set the local VMAC address.
    pub fn vmac(mut self, vmac: [u8; 6]) -> Self {
        self.vmac = vmac;
        self
    }

    /// Set the object database (transfers ownership).
    pub fn database(mut self, db: ObjectDatabase) -> Self {
        self.db = db;
        self
    }

    /// Register one explicit unicast route for a Device recipient.
    pub fn device_binding(mut self, binding: DeviceBinding) -> Result<Self, Error> {
        register_configured_binding(&mut self.configured_device_bindings, binding)?;
        Ok(self)
    }

    /// Set the heartbeat interval in milliseconds (default 30 000).
    pub fn heartbeat_interval_ms(mut self, ms: u64) -> Self {
        self.heartbeat_interval_ms = ms;
        self
    }

    /// Set the heartbeat timeout in milliseconds (default 60 000).
    pub fn heartbeat_timeout_ms(mut self, ms: u64) -> Self {
        self.heartbeat_timeout_ms = ms;
        self
    }

    /// Enable automatic reconnection with the given configuration.
    pub fn reconnect(mut self, config: bacnet_transport::sc::ScReconnectConfig) -> Self {
        self.reconnect = Some(config);
        self
    }

    /// Set the password required for DeviceCommunicationControl requests.
    pub fn dcc_password(mut self, password: impl Into<String>) -> Self {
        self.config.dcc_password = Some(password.into());
        self
    }

    /// Set the password required for ReinitializeDevice requests.
    pub fn reinit_password(mut self, password: impl Into<String>) -> Self {
        self.config.reinit_password = Some(password.into());
        self
    }

    /// Set the policy that authorizes inbound LifeSafetyOperation requests.
    pub fn life_safety_operation_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&LifeSafetyOperationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.life_safety_operation_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Select the only local Audit Log that receives authorized notifications.
    pub fn audit_notification_sink(mut self, sink: ObjectIdentifier) -> Self {
        self.config.audit_notification_sink = Some(sink);
        self
    }

    /// Set the fail-closed ConfirmedAuditNotification authorization policy.
    pub fn audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&AuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Set the fail-closed UnconfirmedAuditNotification authorization policy.
    pub fn unconfirmed_audit_notification_authorizer<F>(mut self, authorizer: F) -> Self
    where
        F: Fn(&UnconfirmedAuditNotificationAuthorizationContext) -> bool + Send + Sync + 'static,
    {
        self.config.unconfirmed_audit_notification_authorizer = Some(Arc::new(authorizer));
        self
    }

    /// Enable periodic fault detection / reliability evaluation.
    ///
    /// When enabled, every object's opt-in reliability hook runs every 10
    /// seconds; the default hook is a no-op.
    pub fn enable_fault_detection(mut self, enabled: bool) -> Self {
        self.config.enable_fault_detection = enabled;
        self
    }

    /// Enable periodic Event Enrollment evaluation (default `true`).
    pub fn enable_event_enrollment(mut self, enabled: bool) -> Self {
        self.config.enable_event_enrollment = enabled;
        self
    }

    /// Set the interval in seconds between Event Enrollment evaluation passes.
    pub fn event_enrollment_interval_secs(mut self, secs: u64) -> Self {
        self.config.event_enrollment_interval_secs = secs;
        self
    }

    /// Set the COV quota and notification work budget policy.
    pub fn cov_policy(mut self, policy: CovPolicy) -> Self {
        self.config.cov_policy = policy;
        self
    }

    /// Connect to the hub and start the server.
    ///
    /// Reconnect configuration is validated before binding-table construction,
    /// TLS lookup, or dialing, and again when the transport starts. An error
    /// still consumes this builder and drops its inputs; this does not promise
    /// generic endpoint rollback.
    pub async fn build(
        self,
    ) -> Result<
        BACnetServer<bacnet_transport::sc::ScTransport<bacnet_transport::sc_tls::TlsWebSocket>>,
        Error,
    > {
        if let Some(config) = &self.reconnect {
            config.validate()?;
        }
        DeviceBindingTable::from_configured(self.configured_device_bindings.clone(), |mac| {
            mac == bacnet_transport::sc_frame::BROADCAST_VMAC
        })?;

        let tls_config = self
            .tls_config
            .ok_or_else(|| Error::Encoding("SC server builder: tls_config is required".into()))?;

        self.config.request_admission_policy.validate()?;
        self.config.read_property_multiple_budget.validate()?;
        self.config.get_alarm_summary_budget.validate()?;
        self.config.get_enrollment_summary_budget.validate()?;
        self.config.atomic_read_file_budget.validate()?;
        self.config.atomic_write_file_budget.validate()?;
        self.config.read_range_budget.validate()?;
        self.config.get_event_information_budget.validate()?;

        let ws = bacnet_transport::sc_tls::TlsWebSocket::connect(&self.hub_url, tls_config.clone())
            .await?;

        let mut transport = bacnet_transport::sc::ScTransport::new(ws, self.vmac)
            .with_heartbeat_interval_ms(self.heartbeat_interval_ms)
            .with_heartbeat_timeout_ms(self.heartbeat_timeout_ms);
        if let Some(rc) = self.reconnect {
            let hub_url = self.hub_url.clone();
            let tls_config = tls_config.clone();
            #[allow(deprecated)]
            {
                transport = transport
                    .with_connector(move || {
                        let hub_url = hub_url.clone();
                        let tls_config = tls_config.clone();
                        async move {
                            bacnet_transport::sc_tls::TlsWebSocket::connect(&hub_url, tls_config)
                                .await
                        }
                    })
                    .with_reconnect(rc);
            }
        }

        BACnetServer::start_with_clock_mode_and_bindings(
            self.config,
            self.db,
            transport,
            Some(ClockConfig::default()),
            self.configured_device_bindings,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_transport::sc::ScReconnectConfig;

    #[tokio::test]
    async fn alarm_summary_sc_budget_validation_before_dial() {
        for budget in [
            GetAlarmSummaryBudget {
                max_objects: 0,
                ..Default::default()
            },
            GetAlarmSummaryBudget {
                max_service_ack_bytes: 0,
                ..Default::default()
            },
        ] {
            let tls = tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
                .with_no_client_auth();
            let builder = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(Arc::new(tls))
                .get_alarm_summary_budget(budget);
            assert_eq!(builder.config.get_alarm_summary_budget, budget);
            assert!(
                matches!(builder.build().await.err(), Some(Error::Encoding(m)) if m.contains("alarm_summary_max_"))
            );
        }
    }

    #[tokio::test]
    async fn rpm_sc_budget_validation_before_dial() {
        for budget in [
            ReadPropertyMultipleBudget {
                max_result_elements: 0,
                ..Default::default()
            },
            ReadPropertyMultipleBudget {
                max_service_ack_bytes: 0,
                ..Default::default()
            },
        ] {
            let tls = tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
                .with_no_client_auth();
            let builder = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(Arc::new(tls))
                .read_property_multiple_budget(budget);
            assert_eq!(builder.config.read_property_multiple_budget, budget);
            let error = builder.build().await.err().unwrap();
            assert!(matches!(error, Error::Encoding(m) if m.contains("rpm_max_")));
            let error = BACnetServer::sc_builder()
                .read_property_multiple_budget(budget)
                .build()
                .await
                .err()
                .unwrap();
            assert!(matches!(error, Error::Encoding(m) if m.contains("tls_config")));
        }
    }

    #[tokio::test]
    async fn sc_server_builder_rejects_invalid_reconnect_before_tls_and_bindings() {
        for broadcast_binding in [false, true] {
            for max_retries in [0, 10] {
                for (initial_delay_ms, max_delay_ms) in [(0, 1), (1, 0), (0, 0), (2, 1)] {
                    let mut builder = BACnetServer::sc_builder().reconnect(ScReconnectConfig {
                        initial_delay_ms,
                        max_delay_ms,
                        max_retries,
                    });
                    if broadcast_binding {
                        let device = ObjectIdentifier::new(ObjectType::DEVICE, 46).unwrap();
                        let binding = DeviceBinding::local(
                            device,
                            bacnet_transport::sc_frame::BROADCAST_VMAC,
                        )
                        .unwrap();
                        builder = builder.device_binding(binding).unwrap();
                    }
                    let error = builder
                        .build()
                        .await
                        .err()
                        .expect("invalid reconnect must fail");
                    assert!(
                        matches!(&error, Error::OutOfRange(message) if message.contains("reconnect")),
                        "expected reconnect error before TLS/bindings, got {error:?}"
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn sc_server_builder_valid_reconnect_preserves_missing_tls_error() {
        for reconnect in [
            None,
            Some(ScReconnectConfig::default()),
            Some(ScReconnectConfig {
                initial_delay_ms: 1,
                max_delay_ms: 1,
                max_retries: 0,
            }),
            Some(ScReconnectConfig {
                initial_delay_ms: 1,
                max_delay_ms: 2,
                max_retries: u32::MAX,
            }),
        ] {
            let mut builder = BACnetServer::sc_builder();
            if let Some(config) = reconnect {
                builder = builder.reconnect(config);
            }
            assert!(matches!(
                builder.build().await,
                Err(Error::Encoding(message)) if message == "SC server builder: tls_config is required"
            ));
        }
    }

    #[test]
    fn sc_server_builder_cov_policy_configures_server_config() {
        let policy = CovPolicy {
            max_subscriptions_per_peer: 12,
            ..CovPolicy::default()
        };
        let builder = BACnetServer::sc_builder().cov_policy(policy.clone());
        assert_eq!(builder.config.cov_policy, policy);
    }

    #[tokio::test]
    async fn admission_sc_invalid_policy_precedes_dial_and_preserves_reconnect_precedence() {
        for bad in [0, usize::MAX] {
            let policy = RequestAdmissionPolicy {
                max_confirmed_in_flight: 1,
                confirmed_recovery_reserve: 0,
                max_unconfirmed_in_flight: bad,
                ..Default::default()
            };
            let tls = tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
                .with_no_client_auth();
            let error = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(Arc::new(tls))
                .request_admission_policy(policy)
                .build()
                .await
                .err()
                .unwrap();
            assert!(matches!(error, Error::Encoding(m) if m.contains("max_unconfirmed_in_flight")));
            let error = BACnetServer::sc_builder()
                .request_admission_policy(policy)
                .reconnect(ScReconnectConfig {
                    initial_delay_ms: 0,
                    ..Default::default()
                })
                .build()
                .await
                .err()
                .unwrap();
            assert!(matches!(error, Error::OutOfRange(m) if m.contains("reconnect")));
        }
    }

    #[tokio::test]
    async fn admission_sc_invalid_policy_recovery_precedes_dial() {
        for reserve in [64, 65] {
            let policy = RequestAdmissionPolicy {
                confirmed_recovery_reserve: reserve,
                ..Default::default()
            };
            let tls = tokio_rustls::rustls::ClientConfig::builder()
                .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
                .with_no_client_auth();
            let error = BACnetServer::sc_builder()
                .hub_url("not-a-websocket-url")
                .tls_config(Arc::new(tls))
                .request_admission_policy(policy)
                .build()
                .await
                .err()
                .unwrap();
            assert!(
                matches!(error, Error::Encoding(m) if m.contains("confirmed_recovery_reserve"))
            );
            let error = BACnetServer::sc_builder()
                .request_admission_policy(policy)
                .reconnect(ScReconnectConfig {
                    initial_delay_ms: 0,
                    ..Default::default()
                })
                .build()
                .await
                .err()
                .unwrap();
            assert!(matches!(error, Error::OutOfRange(m) if m.contains("reconnect")));
        }
    }

    #[tokio::test]
    async fn admission_sc_invalid_policy_peer_limits_precede_dial() {
        for confirmed in [true, false] {
            for bad in [0, usize::MAX] {
                let mut policy = RequestAdmissionPolicy::default();
                let name = if confirmed {
                    policy.max_confirmed_in_flight_per_peer = bad;
                    "max_confirmed_in_flight_per_peer"
                } else {
                    policy.max_unconfirmed_in_flight_per_peer = bad;
                    "max_unconfirmed_in_flight_per_peer"
                };
                let tls = tokio_rustls::rustls::ClientConfig::builder()
                    .with_root_certificates(tokio_rustls::rustls::RootCertStore::empty())
                    .with_no_client_auth();
                let builder = BACnetServer::sc_builder()
                    .hub_url("not-a-websocket-url")
                    .tls_config(Arc::new(tls))
                    .request_admission_policy(policy);
                assert_eq!(builder.config.request_admission_policy, policy);
                let error = builder.build().await.err().unwrap();
                assert!(matches!(error, Error::Encoding(m) if m.contains(name)));
            }
        }
    }
}
