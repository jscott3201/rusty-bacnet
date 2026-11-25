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
        for (label, value) in [
            (
                "read_range_max_returned_items must be positive",
                self.max_returned_items,
            ),
            (
                "read_range_max_service_ack_bytes must be positive",
                self.max_service_ack_bytes,
            ),
        ] {
            if value == 0 {
                return Err(Error::invalid_config(label));
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
    use crate::server::test_transport::TestTransport;

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
                TestTransport::never_start(),
            )
            .await
            .map_err(Error::from)
            .err();
            let generic = BACnetServer::generic_builder()
                .transport(TestTransport::never_start())
                .read_range_budget(budget)
                .build()
                .await
                .map_err(Error::from)
                .err();
            let bip = BACnetServer::bip_builder()
                .port(0)
                .read_range_budget(budget)
                .build()
                .await
                .map_err(Error::from)
                .err();
            for error in [direct, generic, bip] {
                assert!(
                    matches!(error, Some(Error::InvalidConfig(m)) if m.to_string().contains("read_range_max_"))
                );
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
                    .map_err(Error::from)
                    .err();
                assert!(
                    matches!(error, Some(Error::InvalidConfig(m)) if m.to_string().contains("read_range_max_"))
                );
            }
        }
    }
}
