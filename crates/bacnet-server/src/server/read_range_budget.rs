//! ReadRange response-page limits, not total list-read work limits.
use super::*;

/// Positive limits on returned items and the complete logical service ACK.
/// APDU/NPDU envelopes are excluded. Unsegmented peers may further reduce bytes.
/// Full property reads, identity retrieval/selection, and one trial item encoding
/// remain outside these limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadRangeBudget {
    /// Maximum items per directional page (default 256).
    pub max_returned_items: usize,
    /// Maximum complete service-ACK bytes (default 16384).
    pub max_service_ack_bytes: usize,
}

impl Default for ReadRangeBudget {
    fn default() -> Self {
        Self {
            max_returned_items: 256,
            max_service_ack_bytes: 16384,
        }
    }
}

impl ReadRangeBudget {
    /// Reject zero configuration before startup or SC dialing.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            ("read_range_max_returned_items", self.max_returned_items),
            (
                "read_range_max_service_ack_bytes",
                self.max_service_ack_bytes,
            ),
        ] {
            if value == 0 {
                return Err(Error::Encoding(format!("{name} must be positive")));
            }
        }
        Ok(())
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Set ReadRange page limits, validated before startup.
    pub fn read_range_budget(mut self, budget: ReadRangeBudget) -> Self {
        self.config.read_range_budget = budget;
        self
    }
}
impl BipServerBuilder {
    /// Set ReadRange page limits, validated before startup.
    pub fn read_range_budget(mut self, budget: ReadRangeBudget) -> Self {
        self.config.read_range_budget = budget;
        self
    }
}
#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set ReadRange page limits, validated before SC dialing.
    pub fn read_range_budget(mut self, budget: ReadRangeBudget) -> Self {
        self.config.read_range_budget = budget;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct NeverStart;
    impl TransportPort for NeverStart {
        async fn start(
            &mut self,
        ) -> Result<mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error> {
            panic!("invalid budget reached startup")
        }
        async fn stop(&mut self) -> Result<(), Error> {
            Ok(())
        }
        async fn send_unicast(&self, _: &[u8], _: &[u8]) -> Result<(), Error> {
            Ok(())
        }
        async fn send_broadcast(&self, _: &[u8]) -> Result<(), Error> {
            Ok(())
        }
        fn local_mac(&self) -> &[u8] {
            &[1]
        }
    }
    #[tokio::test]
    async fn read_range_all_builders_validate_before_start_or_dial() {
        let default = ReadRangeBudget::default();
        assert_eq!(
            (default.max_returned_items, default.max_service_ack_bytes),
            (256, 16384)
        );
        assert_eq!(ServerConfig::default().read_range_budget, default);
        assert!(format!("{:?}", ServerConfig::default()).contains("read_range_budget"));
        ReadRangeBudget {
            max_returned_items: usize::MAX,
            max_service_ack_bytes: usize::MAX,
        }
        .validate()
        .unwrap();
        for budget in [
            ReadRangeBudget {
                max_returned_items: 0,
                ..default
            },
            ReadRangeBudget {
                max_service_ack_bytes: 0,
                ..default
            },
        ] {
            let direct = BACnetServer::start(
                ServerConfig {
                    read_range_budget: budget,
                    ..Default::default()
                },
                ObjectDatabase::new(),
                NeverStart,
            )
            .await
            .err();
            let generic = BACnetServer::generic_builder()
                .transport(NeverStart)
                .read_range_budget(budget)
                .build()
                .await
                .err();
            let bip = BACnetServer::bip_builder()
                .port(0)
                .read_range_budget(budget)
                .build()
                .await
                .err();
            for error in [direct, generic, bip] {
                assert!(matches!(error, Some(Error::Encoding(m)) if m.contains("read_range_max_")));
            }
            #[cfg(feature = "sc-tls")]
            {
                let error = BACnetServer::sc_builder()
                    .hub_url("not-a-websocket-url")
                    .tls_config(crate::server::sc_builder::test_tls_config())
                    .device_uuid(crate::server::sc_builder::TEST_DEVICE_UUID)
                    .read_range_budget(budget)
                    .build()
                    .await
                    .err();
                assert!(matches!(error, Some(Error::Encoding(m)) if m.contains("read_range_max_")));
            }
        }
    }
}
