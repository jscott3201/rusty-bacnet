//! Per-service RPM limits, independent of concurrent handler admission. The
//! budget itself is bacnet-device-core's
//! [`ReadPropertyMultipleBudget`]; these are the builders' setters.
use super::*;
use bacnet_device_core::read::ReadPropertyMultipleBudget;

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Set positive per-service RPM limits, validated before transport startup.
    pub fn read_property_multiple_budget(mut self, budget: ReadPropertyMultipleBudget) -> Self {
        self.config.read_property_multiple_budget = budget;
        self
    }
}

impl BipServerBuilder {
    /// Set positive per-service RPM limits, validated before transport startup.
    pub fn read_property_multiple_budget(mut self, budget: ReadPropertyMultipleBudget) -> Self {
        self.config.read_property_multiple_budget = budget;
        self
    }
}

#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set positive per-service RPM limits, validated before SC dialing.
    pub fn read_property_multiple_budget(mut self, budget: ReadPropertyMultipleBudget) -> Self {
        self.config.read_property_multiple_budget = budget;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_transport::TestTransport;

    #[tokio::test]
    async fn rpm_defaults_builders_and_validation_before_start() {
        let defaults = ReadPropertyMultipleBudget::default();
        assert_eq!(
            (defaults.max_result_elements, defaults.max_service_ack_bytes),
            (256, 16384)
        );
        assert_eq!(
            ServerConfig::default().read_property_multiple_budget,
            defaults
        );
        assert!(format!("{:?}", ServerConfig::default()).contains("read_property_multiple_budget"));
        ReadPropertyMultipleBudget {
            max_result_elements: usize::MAX,
            max_service_ack_bytes: usize::MAX,
        }
        .validate()
        .unwrap();
        for budget in [
            ReadPropertyMultipleBudget {
                max_result_elements: 0,
                ..defaults
            },
            ReadPropertyMultipleBudget {
                max_service_ack_bytes: 0,
                ..defaults
            },
        ] {
            let direct = BACnetServer::start(
                ServerConfig {
                    read_property_multiple_budget: budget,
                    ..Default::default()
                },
                ObjectDatabase::new(),
                TestTransport::never_start(),
            )
            .await;
            let generic = BACnetServer::generic_builder()
                .transport(TestTransport::never_start())
                .read_property_multiple_budget(budget)
                .build()
                .await;
            let bip = BACnetServer::bip_builder()
                .port(0)
                .read_property_multiple_budget(budget)
                .build()
                .await;
            for error in [direct.err(), generic.err(), bip.err()] {
                assert!(matches!(error, Some(Error::Encoding(m)) if m.contains("rpm_max_")));
            }
            assert!(
                bacnet_objects::DeviceIdentity::default()
                    .with_max_apdu(3)
                    .is_err()
            );
        }
    }
}
