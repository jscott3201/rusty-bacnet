//! Local AtomicWriteFile payload admission, not backend allocation or file-size limits.
use super::*;

/// Positive payload limits independent of peer APDU size and segmentation.
///
/// Checked after existing validation and before the single storage write. Tags,
/// headers, decoding, metadata hooks, and opaque backend/gap-fill work are excluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AtomicWriteFileBudget {
    /// Maximum stream payload octets (default 16384).
    pub max_stream_payload_octets: usize,
    /// Maximum records, including empty records (default 256).
    pub max_records: usize,
    /// Maximum sum of actual record payload lengths (default 16384).
    pub max_record_payload_bytes: usize,
}

impl Default for AtomicWriteFileBudget {
    fn default() -> Self {
        Self {
            max_stream_payload_octets: 16384,
            max_records: 256,
            max_record_payload_bytes: 16384,
        }
    }
}

impl AtomicWriteFileBudget {
    /// Reject zero configuration before startup; empty requests remain permitted.
    pub fn validate(&self) -> Result<(), Error> {
        for (name, value) in [
            (
                "atomic_write_file_max_stream_payload_octets",
                self.max_stream_payload_octets,
            ),
            ("atomic_write_file_max_records", self.max_records),
            (
                "atomic_write_file_max_record_payload_bytes",
                self.max_record_payload_bytes,
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
    /// Set AtomicWriteFile payload limits, validated before transport startup.
    pub fn atomic_write_file_budget(mut self, budget: AtomicWriteFileBudget) -> Self {
        self.config.atomic_write_file_budget = budget;
        self
    }
}
impl BipServerBuilder {
    /// Set AtomicWriteFile payload limits, validated before transport startup.
    pub fn atomic_write_file_budget(mut self, budget: AtomicWriteFileBudget) -> Self {
        self.config.atomic_write_file_budget = budget;
        self
    }
}
#[cfg(feature = "sc-tls")]
impl ScServerBuilder {
    /// Set AtomicWriteFile payload limits, validated before SC dialing.
    pub fn atomic_write_file_budget(mut self, budget: AtomicWriteFileBudget) -> Self {
        self.config.atomic_write_file_budget = budget;
        self
    }
}
