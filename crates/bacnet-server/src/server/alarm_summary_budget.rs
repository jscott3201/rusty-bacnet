//! Local GetAlarmSummary scan and response limits.
use super::*;

/// Positive local limits for a complete GetAlarmSummary response.
///
/// Bounds total database objects before callbacks, and logical encoded service
/// ACK bytes (not APDU/NPDU or peer APDU size). Does not bound one callback,
/// property read, metadata operation, allocation capacity, CPU time or RSS.
/// Byte refusal does not roll back reads already performed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GetAlarmSummaryBudget {
    /// Maximum total database objects, including non-alarming objects (4096).
    pub max_objects: usize,
    /// Maximum encoded service ACK logical bytes (16384).
    pub max_service_ack_bytes: usize,
}

impl Default for GetAlarmSummaryBudget {
    fn default() -> Self {
        Self {
            max_objects: 4096,
            max_service_ack_bytes: 16384,
        }
    }
}

impl GetAlarmSummaryBudget {
    /// Reject zero; larger positive values are an operator policy choice.
    pub fn validate(&self) -> Result<(), Error> {
        for (label, value) in [
            (
                "alarm_summary_max_objects must be positive",
                self.max_objects,
            ),
            (
                "alarm_summary_max_service_ack_bytes must be positive",
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
    /// Set limits validated before transport startup.
    pub fn get_alarm_summary_budget(mut self, budget: GetAlarmSummaryBudget) -> Self {
        self.config.get_alarm_summary_budget = budget;
        self
    }
}

impl BipServerBuilder {
    /// Set limits validated before transport startup.
    pub fn get_alarm_summary_budget(mut self, budget: GetAlarmSummaryBudget) -> Self {
        self.config.get_alarm_summary_budget = budget;
        self
    }
}

#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set limits validated before SC dialing.
    pub fn get_alarm_summary_budget(mut self, budget: GetAlarmSummaryBudget) -> Self {
        self.config.get_alarm_summary_budget = budget;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::test_transport::TestTransport;

    #[tokio::test]
    async fn alarm_summary_defaults_builders_and_pre_start_validation() {
        let defaults = GetAlarmSummaryBudget::default();
        assert_eq!(
            (defaults.max_objects, defaults.max_service_ack_bytes),
            (4096, 16384)
        );
        assert_eq!(ServerConfig::default().get_alarm_summary_budget, defaults);
        assert!(format!("{:?}", ServerConfig::default()).contains("get_alarm_summary_budget"));
        GetAlarmSummaryBudget {
            max_objects: usize::MAX,
            max_service_ack_bytes: usize::MAX,
        }
        .validate()
        .unwrap();
        for budget in [
            GetAlarmSummaryBudget {
                max_objects: 0,
                ..defaults
            },
            GetAlarmSummaryBudget {
                max_service_ack_bytes: 0,
                ..defaults
            },
        ] {
            let direct = BACnetServer::start(
                ServerConfig {
                    get_alarm_summary_budget: budget,
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
                .get_alarm_summary_budget(budget)
                .build()
                .await
                .map_err(Error::from)
                .err();
            let bip = BACnetServer::bip_builder()
                .port(0)
                .get_alarm_summary_budget(budget)
                .build()
                .await
                .map_err(Error::from)
                .err();
            for error in [direct, generic, bip] {
                assert!(
                    matches!(error, Some(Error::InvalidConfig(m)) if m.to_string().contains("alarm_summary_max_"))
                );
            }
        }
    }
}
