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

    /// Set which ways this client segments (default SEGMENTED_BOTH). One
    /// that doesn't receive segments takes no segmented answer and aborts a
    /// segmented notification's first segment; one that doesn't transmit
    /// them fails a request too long for one APDU before sending it. See
    /// [`ClientConfig::segmentation`].
    pub fn segmentation(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation = segmentation;
        self
    }

    /// Set the segment timeout in milliseconds (default: the APDU timeout;
    /// zero fails the build). See [`ClientConfig::apdu_segment_timeout_ms`].
    pub fn apdu_segment_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_segment_timeout_ms = Some(ms);
        self
    }

    /// Set whether a Complex-ACK completes a request whose service takes a
    /// Simple-ACK as a success, for a device in the field that answers so
    /// (default `false`). See
    /// [`ClientConfig::accept_complex_ack_for_simple_services`].
    pub fn accept_complex_ack_for_simple_services(mut self, accept: bool) -> Self {
        self.config.accept_complex_ack_for_simple_services = accept;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded confirmed COV notifications,
    /// ConfirmedCOVNotification and ConfirmedCOVNotificationMultiple alike.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&COVEvent) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
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

    /// Set which ways this client segments (default SEGMENTED_BOTH). One
    /// that doesn't receive segments takes no segmented answer and aborts a
    /// segmented notification's first segment; one that doesn't transmit
    /// them fails a request too long for one APDU before sending it. See
    /// [`ClientConfig::segmentation`].
    pub fn segmentation(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation = segmentation;
        self
    }

    /// Set the segment timeout in milliseconds (default: the APDU timeout;
    /// zero fails the build). See [`ClientConfig::apdu_segment_timeout_ms`].
    pub fn apdu_segment_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_segment_timeout_ms = Some(ms);
        self
    }

    /// Set whether a Complex-ACK completes a request whose service takes a
    /// Simple-ACK as a success, for a device in the field that answers so
    /// (default `false`). See
    /// [`ClientConfig::accept_complex_ack_for_simple_services`].
    pub fn accept_complex_ack_for_simple_services(mut self, accept: bool) -> Self {
        self.config.accept_complex_ack_for_simple_services = accept;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded confirmed COV notifications,
    /// ConfirmedCOVNotification and ConfirmedCOVNotificationMultiple alike.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&COVEvent) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
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

    /// Set which ways this client segments (default SEGMENTED_BOTH). One
    /// that doesn't receive segments takes no segmented answer and aborts a
    /// segmented notification's first segment; one that doesn't transmit
    /// them fails a request too long for one APDU before sending it. See
    /// [`ClientConfig::segmentation`].
    pub fn segmentation(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation = segmentation;
        self
    }

    /// Set the segment timeout in milliseconds (default: the APDU timeout;
    /// zero fails the build). See [`ClientConfig::apdu_segment_timeout_ms`].
    pub fn apdu_segment_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_segment_timeout_ms = Some(ms);
        self
    }

    /// Set whether a Complex-ACK completes a request whose service takes a
    /// Simple-ACK as a success, for a device in the field that answers so
    /// (default `false`). See
    /// [`ClientConfig::accept_complex_ack_for_simple_services`].
    pub fn accept_complex_ack_for_simple_services(mut self, accept: bool) -> Self {
        self.config.accept_complex_ack_for_simple_services = accept;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded confirmed COV notifications,
    /// ConfirmedCOVNotification and ConfirmedCOVNotificationMultiple alike.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&COVEvent) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
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

    /// Set which ways this client segments (default SEGMENTED_BOTH). One
    /// that doesn't receive segments takes no segmented answer and aborts a
    /// segmented notification's first segment; one that doesn't transmit
    /// them fails a request too long for one APDU before sending it. See
    /// [`ClientConfig::segmentation`].
    pub fn segmentation(mut self, segmentation: Segmentation) -> Self {
        self.config.segmentation = segmentation;
        self
    }

    /// Set the segment timeout in milliseconds (default: the APDU timeout;
    /// zero fails the build). See [`ClientConfig::apdu_segment_timeout_ms`].
    pub fn apdu_segment_timeout_ms(mut self, ms: u64) -> Self {
        self.config.apdu_segment_timeout_ms = Some(ms);
        self
    }

    /// Set whether a Complex-ACK completes a request whose service takes a
    /// Simple-ACK as a success, for a device in the field that answers so
    /// (default `false`). See
    /// [`ClientConfig::accept_complex_ack_for_simple_services`].
    pub fn accept_complex_ack_for_simple_services(mut self, accept: bool) -> Self {
        self.config.accept_complex_ack_for_simple_services = accept;
        self
    }

    /// Set the proposed segmented-transfer window size.
    pub fn proposed_window_size(mut self, window_size: u8) -> Self {
        self.config.proposed_window_size = window_size;
        self
    }

    /// Set the response policy for decoded confirmed COV notifications,
    /// ConfirmedCOVNotification and ConfirmedCOVNotificationMultiple alike.
    pub fn confirmed_cov_notification_ack_policy<F>(mut self, policy: F) -> Self
    where
        F: Fn(&COVEvent) -> ConfirmedCOVNotificationResponse + Send + Sync + 'static,
    {
        self.options = self
            .options
            .with_confirmed_cov_notification_ack_policy(policy);
        self
    }
}

impl ClientOptions {
    /// Refuse a channel capacity outside 1 to its maximum, a value the
    /// client keeps (#1833).
    pub(super) fn validate(&self) -> Result<(), Error> {
        let cov = "invalid cov-channel-capacity {}; expected 1..={}";
        let event = "invalid event-channel-capacity {}; expected 1..={}";
        for (label, capacity, max) in [
            (cov, self.cov_channel_capacity, MAX_COV_CHANNEL_CAPACITY),
            (
                event,
                self.event_channel_capacity,
                MAX_EVENT_CHANNEL_CAPACITY,
            ),
        ] {
            if !(1..=max).contains(&capacity) {
                let refusal = Refusal::new(label).with_values(capacity as u64, max as u64);
                return Err(Error::InvalidConfig(refusal));
            }
        }
        Ok(())
    }
}
