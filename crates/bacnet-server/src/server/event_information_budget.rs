//! GetEventInformation admission and response-page limits.
use super::*;

/// Positive local limits; callback allocations, CPU and RSS are not bounded.
/// Every remaining object's strict projection is validated on admitted requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GetEventInformationBudget {
    /// Total database objects, including classes and objects before the cursor.
    pub max_objects: usize,
    /// Maximum summaries retained in a response page.
    pub max_returned_summaries: usize,
    /// Complete encoded service ACK, excluding APDU/NPDU envelopes.
    pub max_service_ack_bytes: usize,
}

impl Default for GetEventInformationBudget {
    fn default() -> Self {
        Self {
            max_objects: 4096,
            max_returned_summaries: 256,
            max_service_ack_bytes: 16384,
        }
    }
}

impl GetEventInformationBudget {
    /// Reject zero configuration before startup or SC dialing.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            ("event_information_max_objects", self.max_objects),
            (
                "event_information_max_returned_summaries",
                self.max_returned_summaries,
            ),
            (
                "event_information_max_service_ack_bytes",
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
    /// Set GetEventInformation limits, validated before startup.
    pub fn get_event_information_budget(mut self, budget: GetEventInformationBudget) -> Self {
        self.config.get_event_information_budget = budget;
        self
    }
}
impl BipServerBuilder {
    /// Set GetEventInformation limits, validated before startup.
    pub fn get_event_information_budget(mut self, budget: GetEventInformationBudget) -> Self {
        self.config.get_event_information_budget = budget;
        self
    }
}
#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set GetEventInformation limits, validated before SC dialing.
    pub fn get_event_information_budget(mut self, budget: GetEventInformationBudget) -> Self {
        self.config.get_event_information_budget = budget;
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
    async fn get_event_information_all_builders_validate_before_start_or_dial() {
        let default = GetEventInformationBudget::default();
        assert_eq!(
            (
                default.max_objects,
                default.max_returned_summaries,
                default.max_service_ack_bytes
            ),
            (4096, 256, 16384)
        );
        assert_eq!(
            ServerConfig::default().get_event_information_budget,
            default
        );
        assert!(format!("{:?}", ServerConfig::default()).contains("get_event_information_budget"));
        GetEventInformationBudget {
            max_objects: usize::MAX,
            max_returned_summaries: usize::MAX,
            max_service_ack_bytes: usize::MAX,
        }
        .validate()
        .unwrap();
        for budget in [
            GetEventInformationBudget {
                max_objects: 0,
                ..default
            },
            GetEventInformationBudget {
                max_returned_summaries: 0,
                ..default
            },
            GetEventInformationBudget {
                max_service_ack_bytes: 0,
                ..default
            },
        ] {
            let direct = BACnetServer::start(
                ServerConfig {
                    get_event_information_budget: budget,
                    ..Default::default()
                },
                ObjectDatabase::new(),
                NeverStart,
            )
            .await
            .err();
            let generic = BACnetServer::generic_builder()
                .transport(NeverStart)
                .get_event_information_budget(budget)
                .build()
                .await
                .err();
            let bip = BACnetServer::bip_builder()
                .port(0)
                .get_event_information_budget(budget)
                .build()
                .await
                .err();
            for error in [direct, generic, bip] {
                assert!(
                    matches!(error, Some(Error::Encoding(m)) if m.contains("event_information_max_"))
                );
            }
            #[cfg(feature = "sc-tls")]
            {
                let error = BACnetServer::sc_builder()
                    .hub_url("not-a-websocket-url")
                    .tls_config(crate::server::sc_builder::test_tls_config())
                    .device_uuid(crate::server::sc_builder::TEST_DEVICE_UUID)
                    .get_event_information_budget(budget)
                    .build()
                    .await
                    .err();
                assert!(
                    matches!(error, Some(Error::Encoding(m)) if m.contains("event_information_max_"))
                );
            }
        }
    }
}
