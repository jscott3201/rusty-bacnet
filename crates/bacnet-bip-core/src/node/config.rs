//! A node's fixed configuration: its own address, its subnet, its mode and
//! the largest datagram it sends, checked once before the engine runs.

use core::fmt;
use core::net::SocketAddrV4;

use bacnet_time::Duration;

use crate::LocalSubnet;

/// The largest NPDU a B/IP data link carries (Table 6-1).
pub const MAX_NPDU: usize = 1497;

/// The largest datagram B/IP sends a [`MAX_NPDU`]-octet NPDU in: a
/// Forwarded-NPDU, whose 4-octet header and 6-octet originating address
/// come before the NPDU. It is the default send budget, and a node's
/// receive buffer should hold at least this much.
pub const MAX_DATAGRAM: usize = 4 + 6 + MAX_NPDU;

/// The shortest interval between automatic registration attempts: a
/// shorter configured renewal is raised to it, so a node tries at most ten
/// times a second.
pub const MIN_RENEWAL: Duration = Duration::from_millis(100);

/// The longest response timeout [`Foreign::new`] picks.
pub const DEFAULT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

/// The smallest datagram budget [`Link::validate`] accepts, in octets.
pub const MIN_BUDGET: u16 = 64;

/// How a B/IP node takes part in its network. It is fixed for the life of
/// a [`Node`](super::Node); changing it means building a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// A device on one of the network's subnets: local broadcasts go to the
    /// subnet's broadcast address (Annex J.4.1).
    Normal,
    /// A foreign device, registered with a BBMD that distributes its
    /// broadcasts and forwards it the network's (Annex J.5).
    Foreign(Foreign),
    /// A BBMD: the receive decisions hand the BBMD's share of each message
    /// to the caller, which runs it through [`bbmd::Tables`](crate::bbmd::Tables).
    Bbmd,
}

/// When the next automatic registration attempt goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Retry {
    /// One renewal interval after the attempt before started, whatever
    /// came of it. A Distribute-Broadcast-To-Network NAK from the BBMD
    /// changes nothing.
    AtRenewal,
    /// After a failure, sooner: one response timeout after it, doubling
    /// with each failure in a row, and never later than
    /// [`Retry::AtRenewal`] would. After a success, one renewal interval
    /// into the lease it proved, which is at once when that lease started
    /// with an earlier, unanswered attempt. A Distribute-Broadcast-To-Network
    /// NAK from the BBMD that answers nothing pulls the next attempt forward
    /// ([`Node::resolve`](super::Node::resolve)).
    Backoff,
}

/// Which Forwarded-NPDUs a foreign device takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ForwardedTrust {
    /// Any sender's, by unicast or broadcast.
    AnySender,
    /// Only those its own BBMD sent it by unicast, as a BBMD sends a foreign
    /// device the network's broadcasts (Annex J.4.5). No one else has a
    /// reason to send it one.
    ConfiguredBbmd,
}

/// A foreign device's registration with its BBMD (Annex J.5.2).
///
/// It is `#[non_exhaustive]`, so a new setting isn't a breaking change:
/// outside this crate, start from [`Foreign::new`] and set the fields to
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Foreign {
    /// The BBMD, at its B/IP address. Replies are matched against this
    /// address and port.
    pub bbmd: SocketAddrV4,
    /// The time-to-live sent with each registration, in seconds; above 0.
    pub ttl: u16,
    /// How often to register again: above zero and below the time-to-live.
    /// A shorter one than [`MIN_RENEWAL`] is raised to it. Registering more
    /// often than the time-to-live needs keeps a NAT mapping open (Annex
    /// J.7.5).
    pub renewal: Duration,
    /// How long an attempt waits for its result: above zero. One longer
    /// than the renewal interval is cut to it. It also sets the answer
    /// window the lease counts on ([`Lease`](super::Lease)).
    pub response_timeout: Duration,
    /// When a failed attempt is tried again.
    pub retry: Retry,
    /// How many broadcasts the caller can keep while the first attempt is
    /// outstanding. [`Node::admit_broadcast`](super::Node::admit_broadcast)
    /// asks it to hold them until that attempt is answered or times out.
    /// With 0 they are sent at once.
    pub hold: u16,
    /// Which Forwarded-NPDUs are taken.
    pub trust: ForwardedTrust,
}

impl Foreign {
    /// Registration with `bbmd` for `ttl` seconds: renewed every half
    /// time-to-live (at least [`MIN_RENEWAL`]), a response timeout of that
    /// or [`DEFAULT_RESPONSE_TIMEOUT`] if shorter, [`Retry::Backoff`], no
    /// broadcasts held, and Forwarded-NPDUs taken only from `bbmd`.
    #[must_use]
    pub const fn new(bbmd: SocketAddrV4, ttl: u16) -> Self {
        // Half the time-to-live, exactly: a whole number of milliseconds.
        let renewal = Duration::from_millis(ttl as u64 * 500).max(MIN_RENEWAL);
        Self {
            bbmd,
            ttl,
            renewal,
            response_timeout: renewal.min(DEFAULT_RESPONSE_TIMEOUT),
            retry: Retry::Backoff,
            hold: 0,
            trust: ForwardedTrust::ConfiguredBbmd,
        }
    }

    /// The time-to-live as a duration.
    #[must_use]
    pub const fn ttl_duration(&self) -> Duration {
        Duration::from_secs(self.ttl as u64)
    }

    /// The renewal interval in use: [`renewal`](Self::renewal), at least
    /// [`MIN_RENEWAL`].
    #[must_use]
    pub const fn effective_renewal(&self) -> Duration {
        self.renewal.max(MIN_RENEWAL)
    }

    /// The response timeout in use: [`response_timeout`](Self::response_timeout),
    /// at most the [effective renewal](Self::effective_renewal).
    #[must_use]
    pub const fn effective_response_timeout(&self) -> Duration {
        self.response_timeout.min(self.effective_renewal())
    }
}

/// A B/IP node's link: everything its receive and send decisions read.
/// They are pure, so a host can share one copy among threads and decide
/// without a lock.
///
/// It is `#[non_exhaustive]`, so a new setting isn't a breaking change:
/// outside this crate, start from [`Link::new`] and set the fields to
/// change. [`Node::new`](super::Node::new) checks the result with
/// [`Link::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Link {
    /// This node's own B/IP address, the MAC address its peers know it by.
    pub address: SocketAddrV4,
    /// Where its local broadcasts go (the subnet's broadcast address at the
    /// network's port), and the subnet mask a BBMD uses.
    pub subnet: LocalSubnet,
    /// How it takes part in the network.
    pub mode: Mode,
    /// The longest datagram it sends, in octets, or `None` to send any NPDU
    /// the codec can carry. See [`Link::max_npdu`].
    pub budget: Option<u16>,
}

impl Link {
    /// A link in `mode`, with the default budget of [`MAX_DATAGRAM`]
    /// octets, checked by [`Link::validate`].
    pub const fn new(
        address: SocketAddrV4,
        subnet: LocalSubnet,
        mode: Mode,
    ) -> Result<Self, ConfigError> {
        let link = Self {
            address,
            subnet,
            mode,
            budget: Some(MAX_DATAGRAM as u16),
        };
        match link.validate() {
            Ok(()) => Ok(link),
            Err(error) => Err(error),
        }
    }

    /// Check the configuration: a nonzero own port, a usable foreign-device
    /// registration, and a budget of at least [`MIN_BUDGET`] octets.
    pub const fn validate(&self) -> Result<(), ConfigError> {
        if self.address.port() == 0 {
            return Err(ConfigError::OwnPort);
        }
        if let Some(budget) = self.budget {
            if budget < MIN_BUDGET {
                return Err(ConfigError::Budget);
            }
        }
        if let Mode::Foreign(foreign) = &self.mode {
            let ttl = foreign.ttl_duration().as_nanos();
            let renewal = foreign.renewal.as_nanos();
            if foreign.ttl == 0 || renewal == 0 || renewal >= ttl {
                return Err(ConfigError::Renewal);
            }
            if foreign.response_timeout.as_nanos() == 0 {
                return Err(ConfigError::ResponseTimeout);
            }
            if foreign.bbmd.port() == 0 {
                return Err(ConfigError::BbmdPort);
            }
            if foreign.bbmd.ip().is_unspecified() || self.groups().contains(foreign.bbmd) {
                return Err(ConfigError::BbmdAddress);
            }
            let (bbmd, own) = (foreign.bbmd, self.address);
            if bbmd.ip().to_bits() == own.ip().to_bits() && bbmd.port() == own.port() {
                return Err(ConfigError::BbmdIsSelf);
            }
        }
        Ok(())
    }
}

/// Why a [`Link`] can't run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// The node's own UDP port is 0.
    OwnPort,
    /// A foreign device's time-to-live is 0, or its renewal interval isn't
    /// above zero and below the time-to-live.
    Renewal,
    /// A foreign device's response timeout is zero.
    ResponseTimeout,
    /// A foreign device's BBMD has UDP port 0.
    BbmdPort,
    /// A foreign device's BBMD is the unspecified address or a group
    /// address ([`Groups::contains`](super::Groups::contains)), which no
    /// BBMD answers from.
    BbmdAddress,
    /// A foreign device's BBMD is the node itself.
    BbmdIsSelf,
    /// The datagram budget is below [`MIN_BUDGET`] octets.
    Budget,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::OwnPort => "the node's own UDP port is 0",
            Self::Renewal => {
                "foreign-device mode needs TTL > 0 and a renewal interval > 0 and < TTL"
            }
            Self::ResponseTimeout => "foreign-device mode needs a response timeout > 0",
            Self::BbmdPort => "the BBMD's UDP port is 0",
            Self::BbmdAddress => "the BBMD's address is unspecified or a group address",
            Self::BbmdIsSelf => "the BBMD is this node's own address",
            Self::Budget => "the datagram budget is below 64 octets",
        })
    }
}

impl core::error::Error for ConfigError {}
