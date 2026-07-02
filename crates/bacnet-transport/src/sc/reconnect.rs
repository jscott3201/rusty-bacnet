/// Configuration for SC transport reconnection with exponential backoff.
#[derive(Debug, Clone)]
pub struct ScReconnectConfig {
    /// Initial delay before first reconnect attempt (ms).
    pub initial_delay_ms: u64,
    /// Maximum delay between reconnect attempts (ms).
    pub max_delay_ms: u64,
    /// Maximum number of reconnect attempts before giving up.
    pub max_retries: u32,
}

impl Default for ScReconnectConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 10_000,
            max_delay_ms: 600_000,
            max_retries: 10,
        }
    }
}

impl super::ScConnection {
    pub(super) fn reset_for_connect_retry(&mut self) {
        *self = Self::new(self.local_vmac, self.device_uuid);
    }
}
