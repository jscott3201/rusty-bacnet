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
        for (name, value) in [
            ("alarm_summary_max_objects", self.max_objects),
            (
                "alarm_summary_max_service_ack_bytes",
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
    struct NeverStart;
    impl TransportPort for NeverStart {
        async fn start(
            &mut self,
        ) -> Result<tokio::sync::mpsc::Receiver<bacnet_transport::port::ReceivedNpdu>, Error>
        {
            panic!("invalid alarm summary budget reached startup")
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
                NeverStart,
            )
            .await
            .err();
            let generic = BACnetServer::generic_builder()
                .transport(NeverStart)
                .get_alarm_summary_budget(budget)
                .build()
                .await
                .err();
            let bip = BACnetServer::bip_builder()
                .port(0)
                .get_alarm_summary_budget(budget)
                .build()
                .await
                .err();
            for error in [direct, generic, bip] {
                assert!(
                    matches!(error, Some(Error::Encoding(m)) if m.contains("alarm_summary_max_"))
                );
            }
        }
    }
}
