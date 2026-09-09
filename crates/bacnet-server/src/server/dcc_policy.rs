use super::{BipServerBuilder, ServerBuilder, TransportPort};
use bacnet_types::error::Error;

/// Local DeviceCommunicationControl authorization, not source authentication.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DccPolicy {
    /// Deny all valid DCC modes, even with a correct configured password.
    #[default]
    DenyAll,
    /// Allow supported modes only with a configured nonempty password.
    RequirePassword,
    /// INSECURE compatibility mode: preserve optional-password authorization.
    /// A configured password is still checked; absence permits any requester.
    LegacyPermissive,
}

impl DccPolicy {
    /// Validate operator configuration before transport startup or dialing.
    pub fn validate(self, password: &Option<String>) -> Result<(), Error> {
        if self == Self::RequirePassword && password.as_ref().is_none_or(String::is_empty) {
            return Err(Error::Encoding(
                "RequirePassword DCC policy requires a nonempty dcc_password".into(),
            ));
        }
        Ok(())
    }
}

impl<T: TransportPort + 'static> ServerBuilder<T> {
    /// Select explicit local DCC authorization (default: deny all).
    pub fn dcc_policy(mut self, policy: DccPolicy) -> Self {
        self.config.dcc_policy = policy;
        self
    }
}

impl BipServerBuilder {
    /// Select explicit local DCC authorization (default: deny all).
    pub fn dcc_policy(mut self, policy: DccPolicy) -> Self {
        self.config.dcc_policy = policy;
        self
    }
}
