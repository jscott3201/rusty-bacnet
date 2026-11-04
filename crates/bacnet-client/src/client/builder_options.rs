use super::*;

impl<T: TransportPort + 'static> ClientBuilder<T> {
    /// Set the independent event notification broadcast channel capacity.
    pub fn event_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.event_channel_capacity = capacity;
        self
    }

    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the least time, in milliseconds, from the latest confirmed
    /// request to a destination finishing to the next being sent (default
    /// 0: no pacing; at most an hour). See
    /// [`ClientConfig::min_request_interval_ms`].
    pub fn min_request_interval_ms(mut self, ms: u64) -> Self {
        self.config.min_request_interval_ms = ms;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    ///
    /// Values 0 and 1 fail at build time. Other finite non-rung values are
    /// conservatively rounded down in Confirmed-Request headers.
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
    /// In [`BipMode::Foreign`], after a move to another BBMD (a Network
    /// Port activation, or the BBMD's host name resolving elsewhere), ask
    /// the BBMD left behind to delete this device's entry rather than let
    /// it lapse (#2307). Off by default; see
    /// [`BipTransport::set_release_previous_bbmd`] for why, and for the
    /// counts it keeps. Ignored, with a warning, in the other modes.
    pub fn release_previous_bbmd(mut self, release: bool) -> Self {
        self.release_previous_bbmd = release;
        self
    }

    /// Look up a BBMD named by host name with `resolver` instead of the
    /// system's resolver (#1852, #2307). See
    /// [`BipTransport::set_bbmd_resolver`].
    pub fn bbmd_resolver(mut self, resolver: Arc<dyn BbmdResolver>) -> Self {
        self.bbmd_resolver = Some(resolver);
        self
    }

    /// Set the independent event notification broadcast channel capacity.
    pub fn event_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.event_channel_capacity = capacity;
        self
    }

    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the least time, in milliseconds, from the latest confirmed
    /// request to a destination finishing to the next being sent (default
    /// 0: no pacing; at most an hour). See
    /// [`ClientConfig::min_request_interval_ms`].
    pub fn min_request_interval_ms(mut self, ms: u64) -> Self {
        self.config.min_request_interval_ms = ms;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    ///
    /// Values 0 and 1 fail at build time. Other finite non-rung values are
    /// conservatively rounded down in Confirmed-Request headers.
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
    /// Make the client a foreign device of a B/IPv6 BBMD (Annex U.4.5),
    /// which then carries its broadcasts. Needs
    /// [`Self::device_instance`]; the build fails without one, or when the
    /// registration fails [`Bip6ForeignDeviceConfig::check`]. A BBMD named
    /// by host name fails the build on the interface `::` unless it
    /// resolves at start; on an explicit interface the client waits for it
    /// (#2365, [`Bip6Transport::register_as_foreign_device`]).
    pub fn foreign_device(mut self, config: Bip6ForeignDeviceConfig) -> Self {
        self.foreign_device = Some(config);
        self
    }

    /// As a foreign device, after a move to another BBMD (a Network Port
    /// activation, or the BBMD's host name resolving elsewhere), ask the
    /// BBMD left behind to delete this device's entry rather than let it
    /// lapse (#2338). Off by default; see
    /// [`Bip6Transport::set_release_previous_bbmd`] for why, and
    /// [`Bip6Transport::previous_bbmd_releases`] for the counts it keeps.
    /// Ignored, with a warning, without [`Self::foreign_device`].
    pub fn release_previous_bbmd(mut self, release: bool) -> Self {
        self.release_previous_bbmd = release;
        self
    }

    /// Look up a BBMD that FD_BBMD_Address names by host name with
    /// `resolver` instead of the system's resolver (#2308, #2338). See
    /// [`Bip6Transport::set_bbmd_resolver`].
    pub fn bbmd_resolver(mut self, resolver: Arc<dyn BbmdResolver>) -> Self {
        self.bbmd_resolver = Some(resolver);
        self
    }

    /// Set the independent event notification broadcast channel capacity.
    pub fn event_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.event_channel_capacity = capacity;
        self
    }

    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the least time, in milliseconds, from the latest confirmed
    /// request to a destination finishing to the next being sent (default
    /// 0: no pacing; at most an hour). See
    /// [`ClientConfig::min_request_interval_ms`].
    pub fn min_request_interval_ms(mut self, ms: u64) -> Self {
        self.config.min_request_interval_ms = ms;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    ///
    /// Values 0 and 1 fail at build time. Other finite non-rung values are
    /// conservatively rounded down in Confirmed-Request headers.
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
    /// Set the independent event notification broadcast channel capacity.
    pub fn event_channel_capacity(mut self, capacity: usize) -> Self {
        self.options.event_channel_capacity = capacity;
        self
    }

    /// Set the number of APDU retries before a confirmed request times out.
    pub fn apdu_retries(mut self, retries: u8) -> Self {
        self.config.apdu_retries = retries;
        self
    }

    /// Set the least time, in milliseconds, from the latest confirmed
    /// request to a destination finishing to the next being sent (default
    /// 0: no pacing; at most an hour). See
    /// [`ClientConfig::min_request_interval_ms`].
    pub fn min_request_interval_ms(mut self, ms: u64) -> Self {
        self.config.min_request_interval_ms = ms;
        self
    }

    /// Set the maximum number of APDU segments this client accepts.
    ///
    /// Values 0 and 1 fail at build time. Other finite non-rung values are
    /// conservatively rounded down in Confirmed-Request headers.
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
