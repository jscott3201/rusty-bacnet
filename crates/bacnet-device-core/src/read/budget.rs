//! The work and byte limits every read service shares: ReadProperty,
//! ReadPropertyMultiple, ReadRange and the server's local read.
use bacnet_types::error::Error;

/// Why a bounded read produced no answer: a service error the requester sees,
/// or a work or byte limit the request would pass.
#[derive(Debug)]
#[non_exhaustive]
pub enum ReadFailure {
    /// A service error, which the requester sees in an Error or Reject PDU.
    Service(Error),
    /// Past the request's work limit (result rows), found in planning.
    Work,
    /// Past the request's byte limit (encoded service-ACK octets).
    Bytes,
}

impl ReadFailure {
    /// The error of a read made with no work or byte limit, which only a
    /// service error can fail.
    ///
    /// For bacnet-server's legacy, unbudgeted ReadPropertyMultiple helper only;
    /// it leaves the public API with that helper (#2178, PR 3).
    #[doc(hidden)]
    pub fn unlimited(self) -> Error {
        match self {
            Self::Service(error) => error,
            Self::Work | Self::Bytes => unreachable!("a read with no limit ran past one"),
        }
    }
}

/// The result rows one read request has expanded, counted against its work
/// limit. A Group's Present_Value charges each member row to the request that
/// reads it (#1172), so several Groups in one request share the one limit.
pub(crate) struct Work {
    used: usize,
    limit: usize,
}

impl Work {
    pub(crate) fn new(limit: usize) -> Self {
        Self { used: 0, limit }
    }

    /// Count one more row, failing once the request would pass its limit.
    pub(crate) fn charge(&mut self) -> Result<(), ReadFailure> {
        self.used = self
            .used
            .checked_add(1)
            .filter(|&n| n <= self.limit)
            .ok_or(ReadFailure::Work)?;
        Ok(())
    }
}

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
