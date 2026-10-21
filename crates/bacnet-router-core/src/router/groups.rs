//! Link facts for classifying a destination on its directly attached port.

/// What a DADR is on its delivery port, as its [`GroupClassifier`] says.
///
/// The router sends nothing to a group address as a unicast. An
/// Unconfirmed-Request for a [`Broadcast`](Self::Broadcast) one goes out as
/// that port's local broadcast instead, and reaches the router's own
/// application where a DLEN 0 broadcast would: when it came from that
/// network. Any other APDU for one, anything for a [`Refuse`](Self::Refuse)
/// one, and every network message for either, is dropped and counted in
/// [`Counters::group_dadr_drops`](super::Counters::group_dadr_drops), with
/// no reject (#1677, decisions L19, L23 and L24).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Group {
    /// Not a group address there: sent to it as addressed.
    #[default]
    No,
    /// A group address of the port's own: on B/IP, a broadcast or
    /// multicast address at the link's own UDP port.
    Broadcast,
    /// A group address outside the port's own: on B/IP, a broadcast or
    /// multicast address at another UDP port, which the link's broadcast
    /// would not reach.
    Refuse,
}

/// An owned port rule, independent of the host's authorization and events.
///
/// Rules may retain caller-owned shared state, as hosted transports do.
/// Classification runs synchronously inside the router call: it must not
/// block or call back into the router. Cloning preserves the same rule.
/// The core requires neither allocation nor a particular link type.
///
/// A custom link can supply an allocation-free rule alongside its other
/// port facts. Every port of one router uses the same rule type; an enum
/// can represent links with different rules.
///
/// ```
/// use bacnet_router_core::npdu::{Mac, NpduLimits};
/// use bacnet_router_core::router::{Group, GroupClassifier, Options, PortConfig};
/// use bacnet_router_core::{Instant, Router, Storage};
///
/// #[derive(Clone, Copy)]
/// struct CustomLink { broadcast: Option<u8> }
/// impl GroupClassifier for CustomLink {
///     const NONE: Self = Self { broadcast: None };
///     fn classify(&self, mac: &[u8]) -> Group {
///         if self.broadcast.is_some_and(|address| mac == [address]) {
///             Group::Broadcast
///         } else {
///             Group::No
///         }
///     }
/// }
/// let port = PortConfig::new(1, Mac::from_array([1]), NpduLimits::both(1497),
///     CustomLink { broadcast: Some(254) });
/// let mut storage = Storage::<1, 8, 8, 4, 1497, CustomLink>::new();
/// let router = Router::new(&mut storage, &[port], Options::STANDARD, Instant::ZERO)?;
/// assert_eq!(router.ports()[0].groups.classify(&[254]), Group::Broadcast);
/// # Ok::<(), bacnet_router_core::router::ConfigError>(())
/// ```
pub trait GroupClassifier: Clone {
    /// An unused storage slot's rule, which classifies every MAC as
    /// [`Group::No`]. It is replaced when the router installs its ports.
    const NONE: Self;

    /// What `mac` is on this port. Called only for a nonempty DADR on a
    /// directly attached network, never for an indirect route or DLEN 0.
    fn classify(&self, mac: &[u8]) -> Group;
}

/// The portable glue's group rules. B/IP and MS/TP configurations install
/// theirs automatically; a custom link may use its own [`GroupClassifier`]
/// as [`PortConfig`](super::PortConfig)'s type parameter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Groups {
    /// The link has no group addresses.
    #[default]
    None,
    /// B/IP's group destinations and the UDP port its broadcast reaches.
    #[cfg(feature = "bip")]
    Bip {
        /// The link's own group rule, including its interface exception.
        destinations: bacnet_bip_core::node::Groups,
        /// The link's own UDP port.
        port: u16,
    },
    /// MS/TP: station 255 is the local broadcast.
    #[cfg(feature = "mstp")]
    Mstp,
}

impl GroupClassifier for Groups {
    const NONE: Self = Self::None;

    fn classify(&self, mac: &[u8]) -> Group {
        let _ = mac; // Neither glue feature need be enabled.
        match self {
            Self::None => Group::No,
            #[cfg(feature = "bip")]
            Self::Bip { destinations, port } => {
                match bacnet_bip_core::codec::address_from_slice(mac) {
                    Ok(address) if destinations.contains(address) => {
                        if address.port() == *port {
                            Group::Broadcast
                        } else {
                            Group::Refuse
                        }
                    }
                    _ => Group::No,
                }
            }
            #[cfg(feature = "mstp")]
            Self::Mstp if mac == [bacnet_mstp_core::codec::BROADCAST_MAC] => Group::Broadcast,
            #[cfg(feature = "mstp")]
            Self::Mstp => Group::No,
        }
    }
}
