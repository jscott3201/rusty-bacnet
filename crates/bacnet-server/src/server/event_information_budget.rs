//! GetEventInformation admission and response-page limits: the builder
//! setters for `bacnet_device_core::event::GetEventInformationBudget`.
use super::*;

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
    use crate::server::test_transport::TestTransport;

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
                TestTransport::never_start(),
            )
            .await
            .err();
            let generic = BACnetServer::generic_builder()
                .transport(TestTransport::never_start())
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
