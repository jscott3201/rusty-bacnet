//! Local AtomicReadFile request-count and logical service-ACK limits.
use super::*;

/// Positive local limits, independent of peer APDU size and segmentation.
///
/// Raw request counts are checked after existing handler validation, before the
/// storage read. ACK bytes include every service field but exclude APDU/NPDU.
/// This does not bound opaque storage work or its single owned read result;
/// notably, a record-count limit is not a total storage-byte limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtomicReadFileBudget {
    /// Maximum raw requested stream octets (default 16384).
    pub max_requested_stream_octets: usize,
    /// Maximum raw requested records (default 256); the legacy read window remains.
    pub max_requested_records: usize,
    /// Maximum complete encoded service ACK bytes (default 16384).
    pub max_service_ack_bytes: usize,
}

impl Default for AtomicReadFileBudget {
    fn default() -> Self {
        Self {
            max_requested_stream_octets: 16384,
            max_requested_records: 256,
            max_service_ack_bytes: 16384,
        }
    }
}

impl AtomicReadFileBudget {
    /// Reject zero configuration values before startup. Request counts may be zero.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            (
                "atomic_read_file_max_requested_stream_octets",
                self.max_requested_stream_octets,
            ),
            (
                "atomic_read_file_max_requested_records",
                self.max_requested_records,
            ),
            (
                "atomic_read_file_max_service_ack_bytes",
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
    /// Set AtomicReadFile limits validated before transport startup.
    pub fn atomic_read_file_budget(mut self, budget: AtomicReadFileBudget) -> Self {
        self.config.atomic_read_file_budget = budget;
        self
    }
}
impl BipServerBuilder {
    /// Set AtomicReadFile limits validated before transport startup.
    pub fn atomic_read_file_budget(mut self, budget: AtomicReadFileBudget) -> Self {
        self.config.atomic_read_file_budget = budget;
        self
    }
}
#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set AtomicReadFile limits validated before SC dialing.
    pub fn atomic_read_file_budget(mut self, budget: AtomicReadFileBudget) -> Self {
        self.config.atomic_read_file_budget = budget;
        self
    }
}
