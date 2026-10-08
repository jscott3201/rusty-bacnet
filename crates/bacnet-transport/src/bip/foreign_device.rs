use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use bacnet_types::enums::BvlcFunction;
use bacnet_types::error::Error;
use tokio::time::Instant;

use super::{
    bvlc_response::BvlcResponseKind, client_management::ManagementClient, BipSocket, BipTransport,
};
use crate::bvll::encode_bip_mac;

/// Configuration for automatic foreign-device registration. Its positive TTL
/// is sent unchanged; the independent renewal interval controls local attempts.
#[derive(Debug, Clone)]
pub struct ForeignDeviceConfig {
    /// BBMD IP address to register with.
    pub bbmd_ip: Ipv4Addr,
    /// BBMD UDP port.
    pub bbmd_port: u16,
    /// Advertised time-to-live in seconds; must be positive for automatic mode.
    /// The one-shot registration helper separately permits zero-TTL requests.
    pub ttl: u16,
    /// Local attempt interval, strictly positive and shorter than `ttl`.
    /// `None` uses half the TTL, including 500 ms for a one-second TTL.
    pub renewal_interval: Option<Duration>,
}

impl ForeignDeviceConfig {
    pub(super) fn interval(&self) -> Result<Duration, Error> {
        let ttl = Duration::from_secs(u64::from(self.ttl));
        let interval = self.renewal_interval.unwrap_or(ttl / 2);
        if self.ttl == 0 || interval.is_zero() || interval >= ttl {
            return Err(Error::Encoding(
                "foreign-device mode needs TTL > 0 and a renewal interval > 0 and < TTL".into(),
            ));
        }
        Ok(interval)
    }
}

pub(super) async fn run(
    socket: Arc<BipSocket>,
    management: Arc<ManagementClient>,
    config: ForeignDeviceConfig,
    interval: Duration,
) {
    let target = encode_bip_mac(config.bbmd_ip.octets(), config.bbmd_port);
    loop {
        // One attempt at a time, with no catch-up burst after a delayed poll.
        // Short intervals also bound response waiting, so a silent peer cannot
        // hold the slot for the full manual-request timeout on every renewal.
        let next = Instant::now() + interval;
        management.schedule(next);
        if let Err(error) = management
            .request(
                &socket,
                &target,
                BvlcFunction::REGISTER_FOREIGN_DEVICE,
                BvlcResponseKind::Result,
                &config.ttl.to_be_bytes(),
                interval.min(BipTransport::BVLC_RESPONSE_TIMEOUT),
            )
            .await
        {
            tracing::debug!(bbmd = %SocketAddrV4::new(config.bbmd_ip, config.bbmd_port), %error,
                "Foreign registration attempt did not return a matched result");
        }
        tokio::time::sleep_until(next).await;
    }
}
