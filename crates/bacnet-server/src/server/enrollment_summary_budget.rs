//! Local limits for the deprecated GetEnrollmentSummary interoperability service.
use super::*;

/// Positive limits on total database objects and complete logical ACK bytes.
///
/// Counts all objects before any callbacks, after request decoding. Bytes exclude
/// APDU/NPDU and are independent of peer APDU size and segmentation. Does not bound
/// request decoding, individual callbacks/reads, recipient-list processing,
/// allocation capacity/OOM, RSS, CPU or deadlines. Reads are not rolled back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GetEnrollmentSummaryBudget {
    /// Maximum total database objects, including noncandidates and classes (4096).
    pub max_objects: usize,
    /// Maximum encoded service ACK logical bytes (16384).
    pub max_service_ack_bytes: usize,
}

impl Default for GetEnrollmentSummaryBudget {
    fn default() -> Self {
        Self {
            max_objects: 4096,
            max_service_ack_bytes: 16384,
        }
    }
}

impl GetEnrollmentSummaryBudget {
    /// Reject zero; positive limits are local operator policy, not BACnet limits.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            ("enrollment_summary_max_objects", self.max_objects),
            (
                "enrollment_summary_max_service_ack_bytes",
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
    /// Set GetEnrollmentSummary limits validated before transport startup.
    pub fn get_enrollment_summary_budget(mut self, budget: GetEnrollmentSummaryBudget) -> Self {
        self.config.get_enrollment_summary_budget = budget;
        self
    }
}
impl BipServerBuilder {
    /// Set GetEnrollmentSummary limits validated before transport startup.
    pub fn get_enrollment_summary_budget(mut self, budget: GetEnrollmentSummaryBudget) -> Self {
        self.config.get_enrollment_summary_budget = budget;
        self
    }
}
#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set GetEnrollmentSummary limits validated before SC dialing.
    pub fn get_enrollment_summary_budget(mut self, budget: GetEnrollmentSummaryBudget) -> Self {
        self.config.get_enrollment_summary_budget = budget;
        self
    }
}
