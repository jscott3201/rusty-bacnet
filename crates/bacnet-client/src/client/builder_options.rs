use super::*;

impl<T: TransportPort + 'static> ClientBuilder<T> {
    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    pub fn max_segments(mut self, max_segments: Option<u8>) -> Self {
        self.config.max_segments = max_segments;
        self
    }

    /// Set whether this client accepts segmented responses.
    pub fn segmented_response_accepted(mut self, accepted: bool) -> Self {
        self.config.segmented_response_accepted = accepted;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded ConfirmedCOVNotifications.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&ReceivedCOVNotification) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
    {
        self.options = self
            .options
            .with_confirmed_cov_notification_ack_policy(policy);
        self
    }
}

impl BipClientBuilder {
    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    pub fn max_segments(mut self, max_segments: Option<u8>) -> Self {
        self.config.max_segments = max_segments;
        self
    }

    /// Set whether this client accepts segmented responses.
    pub fn segmented_response_accepted(mut self, accepted: bool) -> Self {
        self.config.segmented_response_accepted = accepted;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded ConfirmedCOVNotifications.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&ReceivedCOVNotification) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
    {
        self.options = self
            .options
            .with_confirmed_cov_notification_ack_policy(policy);
        self
    }
}

#[cfg(feature = "ipv6")]
impl Bip6ClientBuilder {
    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    pub fn max_segments(mut self, max_segments: Option<u8>) -> Self {
        self.config.max_segments = max_segments;
        self
    }

    /// Set whether this client accepts segmented responses.
    pub fn segmented_response_accepted(mut self, accepted: bool) -> Self {
        self.config.segmented_response_accepted = accepted;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded ConfirmedCOVNotifications.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&ReceivedCOVNotification) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
    {
        self.options = self
            .options
            .with_confirmed_cov_notification_ack_policy(policy);
        self
    }
}

#[cfg(feature = "sc-tls")]
impl ScClientBuilder {
    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    pub fn max_segments(mut self, max_segments: Option<u8>) -> Self {
        self.config.max_segments = max_segments;
        self
    }

    /// Set whether this client accepts segmented responses.
    pub fn segmented_response_accepted(mut self, accepted: bool) -> Self {
        self.config.segmented_response_accepted = accepted;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded ConfirmedCOVNotifications.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&ReceivedCOVNotification) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
    {
        self.options = self
            .options
            .with_confirmed_cov_notification_ack_policy(policy);
        self
    }
}
