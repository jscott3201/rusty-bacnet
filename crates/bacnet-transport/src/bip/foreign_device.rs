use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use bacnet_bip_core::Duration as CoreDuration;
use bacnet_bip_core::node::{ConfigError, Foreign, ForwardedTrust, Groups, Retry};
use bacnet_types::error::Error;

use super::{BipTransport, ForeignBbmd};

/// How many broadcasts a foreign device holds while the first registration
/// attempt after each start, or after a move to another BBMD (#1825), is
/// outstanding (#1622): enough for what a device
/// or router announces at start, such as I-Am and I-Am-Router-To-Network.
/// They go, in order, once that attempt succeeds or goes unanswered, and are
/// dropped if the BBMD refuses it; one more is refused with an error.
pub(super) const HELD_BROADCASTS: u16 = 16;

/// Configuration for automatic foreign-device registration. Its positive TTL
/// is sent unchanged; the independent renewal interval controls local attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignDeviceConfig {
    /// The BBMD to register with: its address, or a host name and port,
    /// looked up at start and again alongside the registrations (#2308).
    /// See [`ForeignBbmd`] for what happens while a name doesn't resolve.
    pub bbmd: ForeignBbmd,
    /// Advertised time-to-live in seconds; must be positive for automatic mode.
    /// The one-shot registration helper separately permits zero-TTL requests.
    pub ttl: u16,
    /// Requested local attempt interval, positive and shorter than `ttl`.
    /// `None` uses half the TTL, including 500 ms for a one-second TTL.
    /// Positive values below 100 ms use 100 ms to bound local retry frequency.
    /// This pacing floor is local policy and does not change the wire TTL.
    pub renewal_interval: Option<Duration>,
}

impl ForeignDeviceConfig {
    /// The BBMD's B/IP address, unless it is known by a name not looked up
    /// yet.
    pub(super) fn bbmd(&self) -> Option<SocketAddrV4> {
        self.bbmd.address()
    }

    /// The renewal interval on the node engine's timeline: the configured
    /// one, or half the TTL, at least 100 ms.
    pub(super) fn interval(&self) -> Result<CoreDuration, Error> {
        let ttl = Duration::from_secs(u64::from(self.ttl));
        let interval = self.renewal_interval.unwrap_or(ttl / 2);
        if self.ttl == 0 || interval.is_zero() || interval >= ttl {
            return Err(Error::Encoding(
                "foreign-device mode needs TTL > 0 and a renewal interval > 0 and < TTL".into(),
            ));
        }
        // Limit fast replies, busy slots and local failures to ten attempts per
        // second. This remains below half of the smallest positive wire TTL.
        let interval = interval.max(Duration::from_millis(100));
        CoreDuration::try_from(interval).map_err(|too_long| {
            Error::Encoding(format!("foreign-device renewal interval: {too_long}"))
        })
    }

    /// Refuse, before any I/O, a BBMD no device could register with: one
    /// at port 0, at the unspecified address, or at a group address of a
    /// link whose broadcast IP is `broadcast` on `interface`, or a host name
    /// not in the RFC 1123 form. One at the transport's own address is
    /// refused once it is bound, when the node starts; a name's address is
    /// checked once it resolves.
    pub(super) fn check_bbmd(&self, broadcast: Ipv4Addr, interface: Ipv4Addr) -> Result<(), Error> {
        let bbmd = match &self.bbmd {
            ForeignBbmd::Address(bbmd) => *bbmd,
            ForeignBbmd::Name { host, port } => {
                return match self.bbmd.name() {
                    Some(_) => Ok(()),
                    None if *port == 0 => Err(Error::Encoding(ConfigError::BbmdPort.to_string())),
                    None => Err(Error::Encoding(format!(
                        "the BBMD's host name '{host}' isn't in the RFC 1123 form"
                    ))),
                };
            }
        };
        // Only the IP test is read, so the broadcast's port doesn't matter.
        let groups = Groups::new(SocketAddrV4::new(broadcast, 0), interface);
        let refusal = if bbmd.port() == 0 {
            ConfigError::BbmdPort
        } else if bbmd.ip().is_unspecified() || groups.contains_ip(*bbmd.ip()) {
            ConfigError::BbmdAddress
        } else {
            return Ok(());
        };
        Err(Error::Encoding(refusal.to_string()))
    }

    /// The node engine's registration, renewed every `interval`
    /// ([`Self::interval`]): an attempt waits for its result at most that
    /// interval or the manual helpers' timeout, whichever is shorter. After
    /// a success the next goes one interval into the lease it proved; after
    /// a refusal, one interval after that attempt started; after a loss,
    /// sooner, backing off from the response timeout up to the interval;
    /// and a Distribute-Broadcast-To-Network NAK from the BBMD pulls it
    /// forward ([`Retry::Backoff`], #1622). Until the first attempt
    /// resolves, up to [`HELD_BROADCASTS`] broadcasts are held. A
    /// Forwarded-NPDU is taken only from the BBMD by unicast, as a BBMD sends
    /// its foreign devices the network's broadcasts
    /// ([`ForwardedTrust::ConfiguredBbmd`]). A BBMD known by a name not
    /// looked up yet leaves the node waiting for it, with no attempt due.
    pub(super) fn engine(&self, interval: CoreDuration) -> Foreign {
        // Three seconds, exactly.
        let response_timeout =
            CoreDuration::saturating_from_core(BipTransport::BVLC_RESPONSE_TIMEOUT);
        let mut foreign = match self.bbmd() {
            Some(bbmd) => Foreign::new(bbmd, self.ttl),
            None => Foreign::waiting(self.ttl),
        };
        foreign.renewal = interval;
        foreign.response_timeout = interval.min(response_timeout);
        foreign.retry = Retry::Backoff;
        foreign.hold = HELD_BROADCASTS;
        foreign.trust = ForwardedTrust::ConfiguredBbmd;
        foreign
    }
}
