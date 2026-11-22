//! What [`BACnetClient::routed_path_limits`] lists: each routed path's size
//! limit with where it comes from (#2667).

use bacnet_types::MacAddr;

use super::BACnetClient;
use super::routed_path_limits::DEFAULT_ROUTED_PATH_MAX_NPDU;
use bacnet_transport::port::TransportPort;

/// Where the limit in force on a routed path comes from
/// ([`RoutedPathLimit::source`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoutedPathLimitSource {
    /// Nothing is configured or learned that lowers it: the path uses the
    /// conservative 228 octets every router must forward (Table 6-1).
    Default,
    /// Set by hand with
    /// [`configure_routed_path_max_npdu`](BACnetClient::configure_routed_path_max_npdu).
    Configured,
    /// Learned from a router's Reject-Message-To-Network with the "message
    /// too long" reason, and lower than the configured limit.
    Learned,
}

impl RoutedPathLimitSource {
    /// The source's name in output: `default`, `configured` or `learned`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Configured => "configured",
            Self::Learned => "learned",
        }
    }
}

/// One routed path, the router this client sends through and the
/// destination network, with its size limit as the client holds it
/// ([`BACnetClient::routed_path_limits`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedPathLimit {
    /// The router's MAC on the local link: the next hop to `dnet`.
    pub router_mac: MacAddr,
    /// The destination network.
    pub dnet: u16,
    /// The limit set by hand, in NPDU octets, if one is.
    pub configured_max_npdu: Option<u16>,
    /// The largest NPDU the path's reason-4 rejects leave, if the client
    /// believed one: one octet under the smallest frame a router refused as
    /// too long. It is never below 228, the default.
    pub learned_max_npdu: Option<u16>,
    /// Whether a reason-4 reject on this path was dropped because the
    /// request it landed on sent nothing above 228 octets
    /// ([`BACnetClient::below_minimum_reject_drops`]) since the path's limit
    /// was last configured or cleared. A router that really limits the path
    /// below 228 octets shows here, while its requests end by timeout.
    pub below_minimum_reject_seen: bool,
}

impl RoutedPathLimit {
    /// The limit requests on the path are sized to, in NPDU octets: the
    /// configured limit (228 without one), lowered to the learned one.
    pub fn max_npdu(&self) -> u16 {
        let base = self
            .configured_max_npdu
            .unwrap_or(DEFAULT_ROUTED_PATH_MAX_NPDU);
        self.learned_max_npdu
            .map_or(base, |learned| base.min(learned))
    }

    /// Where [`max_npdu`](Self::max_npdu) comes from: learned when a learned
    /// limit is below the configured one, configured when one is set,
    /// otherwise the default.
    pub fn source(&self) -> RoutedPathLimitSource {
        let base = self
            .configured_max_npdu
            .unwrap_or(DEFAULT_ROUTED_PATH_MAX_NPDU);
        match (self.configured_max_npdu, self.learned_max_npdu) {
            (_, Some(learned)) if learned < base => RoutedPathLimitSource::Learned,
            (Some(_), _) => RoutedPathLimitSource::Configured,
            (None, _) => RoutedPathLimitSource::Default,
        }
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Every routed path that holds something beyond the default: a
    /// configured limit, a learned one, or a below-minimum reject seen
    /// ([`RoutedPathLimit::below_minimum_reject_seen`]), ordered by router
    /// MAC and then network. A path with none of these uses the default 228
    /// octets and is not listed. A path held only by the below-minimum
    /// mark may be reclaimed for another path when the table is full (256
    /// paths), and is then no longer listed: the mark is a hint, and the
    /// table does not grow for it. Sends nothing, and leaves every request
    /// alone: it reads the table once and copies what it lists.
    pub fn routed_path_limits(&self) -> Vec<RoutedPathLimit> {
        self.routed_path_limits.listing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(configured: Option<u16>, learned: Option<u16>) -> RoutedPathLimit {
        RoutedPathLimit {
            router_mac: MacAddr::from_slice(&[2]),
            dnet: 5,
            configured_max_npdu: configured,
            learned_max_npdu: learned,
            below_minimum_reject_seen: false,
        }
    }

    /// The limit in force and its source: the default without anything,
    /// the configured value when set, and a learned one only where it is
    /// lower; a tie is the configured limit's.
    #[test]
    fn the_limit_in_force_names_its_source() {
        use RoutedPathLimitSource::{Configured, Default, Learned};
        for (configured, learned, max, source) in [
            (None, None, 228, Default),
            // A learned limit is never below 228, so without a configured
            // one the default still binds.
            (None, Some(400), 228, Default),
            (Some(128), None, 128, Configured),
            (Some(1497), None, 1497, Configured),
            (Some(1497), Some(479), 479, Learned),
            (Some(479), Some(479), 479, Configured),
            (Some(300), Some(479), 300, Configured),
        ] {
            let path = path(configured, learned);
            assert_eq!(path.max_npdu(), max, "{configured:?} {learned:?}");
            assert_eq!(path.source(), source, "{configured:?} {learned:?}");
        }
        assert_eq!(
            [Default, Configured, Learned].map(RoutedPathLimitSource::name),
            ["default", "configured", "learned"]
        );
    }
}
