//! Per-service RPM limits, independent of concurrent handler admission.
use super::*;

/// Local ReadPropertyMultiple planning and encoded service-ACK limits.
///
/// These do not bound request decoding, object metadata, an individual property
/// read/value encoding, allocator capacity, or process memory. Byte refusal does
/// not roll back reads already performed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadPropertyMultipleBudget {
    /// Maximum aggregate expanded result occurrences (default 256).
    ///
    /// ReadProperty, ReadRange and `read_local` charge a Group's Present_Value
    /// to this limit as well. The shared endpoint (`bacnet-endpoint`) has no
    /// `ServerConfig`: its ReadProperty uses the session's `read_work_limit`,
    /// which has the same default and the same abort.
    pub max_result_elements: usize,
    /// Maximum encoded service parameters, excluding APDU/NPDU (default 16384).
    pub max_service_ack_bytes: usize,
}

impl Default for ReadPropertyMultipleBudget {
    fn default() -> Self {
        Self {
            max_result_elements: 256,
            max_service_ack_bytes: 16384,
        }
    }
}

impl ReadPropertyMultipleBudget {
    /// Reject zero; no semaphore-derived or allocation-capacity upper limit.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            ("rpm_max_result_elements", self.max_result_elements),
            ("rpm_max_service_ack_bytes", self.max_service_ack_bytes),
        ] {
            if value == 0 {
                return Err(Error::Encoding(format!("{name} must be positive")));
            }
        }
        Ok(())
    }
}

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
