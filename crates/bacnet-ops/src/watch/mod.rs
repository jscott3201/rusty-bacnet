//! Watching points: one managed BACnet subscription or poller per point,
//! however many listeners share it (#2067, #2560).
//!
//! An application calls [`Client::watch`](crate::Client::watch) with
//! points from [`Device::point`](crate::Device::point) and reads the
//! [`Watch`], a stream of [`Update`]s: each point's latest value, its
//! Status_Flags and when it came, and its [`State`], which holds the
//! reason ([`Why`]) whenever the point isn't subscribed. That is the one
//! way an application watches. Under it is [`hub`], for a front end that
//! runs its own hub (the `bacnet` tool's watch list and MCP server keep a
//! history, serve several sessions and pick their own settings); one that
//! picks its own settings but reads updates as an application does takes
//! a [`Watch`] from its hub ([`Hub::stream`](hub::Hub::stream)).
//!
//! A point names its device by address: the hub keys its entries, its
//! per-device schedulers and its subscriptions' targets on it, so two
//! devices sharing an instance are two devices, and one announcing itself
//! again at its address keeps its entries. Each point is read once, then
//! subscribed for a finite lifetime (SubscribeCOV for an object,
//! SubscribeCOVProperty for one property) through the client's managed
//! subscriptions, which renew it before it lapses. Points the device won't
//! subscribe are polled instead, all of a device's together in batched
//! ReadPropertyMultiple requests, and a device that stops answering is
//! polled after a backoff.
//!
//! A device that refuses the COV service itself is remembered by its
//! address ([`CovSupport`]), as an RPM refusal is: its later points are
//! polled without asking, until the discovery table loses its row there
//! or another device announces itself there. A full subscription table is
//! no refusal: the point is polled and asked again after a backoff
//! ([`Why::Full`]). A subscription a device lost (it restarted, say) is
//! made again by its next renewal, which goes before each lifetime ends,
//! or sooner when the device announces itself or sends a restart
//! notification and its Time_Of_Device_Restart (read once when it is first
//! subscribed) moved (#2603). After
//! an outage, the points' new subscriptions and polls take the device's
//! gate and the client's rate like every other request, so they come back
//! no faster than the limits allow.
//!
//! A point's device is its address and an instance, which only an answer
//! to a request the hub sent may set once known, confirm, or change: the
//! instance the device there last named itself in one, else the one an
//! answer named for the address's other points, else the discovery
//! table's row there, else the first sender its notifications name. Until
//! an answer names it, the first one naming another replaces it. A
//! notification naming another sender is dropped, an I-Am of another
//! device there is a cue, and a poll by ReadPropertyMultiple asks the
//! device which it is in the same request. A cue is checked by asking the
//! device there, at once and then after a wait that doubles from 60 s up to
//! a lifetime and stays there: when it is another than an answer had
//! named, the point is given up ([`Why::Replaced`]) until its instance
//! answers there again; found at one other address, and named there by
//! the device's own answer, it is reported [`Why::Moved`]. A device that
//! never announces itself taking the address of a point polled by
//! ReadProperty alone goes unnoticed, as does one taking it before any
//! answer named the watched device.
//!
//! When the device behind a point stops answering (a poll or a first read
//! fails; a point whose renewal failed goes to the next poll), the
//! scheduler asks where its instance went: the one other address
//! claiming it when the row at the address names another instance now, or
//! where a following handle moved by its own rule (re-resolving after a
//! timeout, at most one Who-Is every [`QUIET`](crate::device::QUIET)).
//! That address, or one a following handle moved to for a request of its
//! own (on the discovery table's rows, which I-Ams nobody asked for fill),
//! is only a cue (#2812): the device there is asked which it is, by one
//! read of its Device object by the wildcard instance, paced as a claim's
//! asks, and the points are given up as moved ([`Why::Moved`]) only when
//! that answer names their instance. Another instance, no answer, or a
//! device refusing that read changes nothing. A row merely gone from the
//! discovery table (aged out of a quiet device) is no move, nor are
//! announcement times: a duplicate starting up while the device misses a
//! request looks the same. The point stays unreachable, polled after its
//! backoff. A move doesn't
//! stick: a point started there later starts as any other, and the
//! given-up points start again once the address announces itself for the
//! instance, or answers a start while the discovery table names it there.
//! The hub doesn't follow a device to its new address; watch the point
//! there.
//!
//! Lifetimes are finite by construction ([`hub::Lifetime`] can't be zero):
//! a subscription that never lapses is a write in the `bacnet` tool's
//! command classes (`bacnet_tool::class`), and the hub never sends one.

pub mod hub;

mod device;
mod dispatch;
mod jobs;
mod options;
mod point;
mod sample;
mod shared;
mod stream;
mod support;

pub use self::point::{Point, Watched, Why};
pub use self::sample::Via;
pub(crate) use self::stream::OwnHub;
pub use self::stream::{MAX_WATCHED, State, Update, Watch};
pub use self::support::CovSupport;
pub(crate) use self::support::{CovMemory, services};

/// Why a watch wasn't started: nothing was watched, and nothing sent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WatchError {
    /// The points would take the hub past the most it watches at once,
    /// points still cancelling included ([`MAX_WATCHED`] for a client's
    /// own hub, [`hub::WatchOptions::max_points`] for another).
    Full {
        /// The most.
        max: usize,
    },
    /// The hub has shut down.
    Shut,
    /// Called outside a Tokio runtime, which the hub's tasks need.
    NoRuntime,
    /// [`Hub::new`](hub::Hub::new) was given settings
    /// [`WatchOptions::validate`](hub::WatchOptions::validate) refuses, for
    /// this reason.
    Settings(String),
    /// Every range of subscriber process identifiers on the client is held:
    /// this many hubs live on it, the client's own included, or are still
    /// cancelling. Drop one first.
    Hubs {
        /// The most.
        max: usize,
    },
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full { max } => write!(
                f,
                "the hub would watch more than its {max} points, those still cancelling included"
            ),
            Self::Shut => f.write_str("the hub has shut down"),
            Self::NoRuntime => f.write_str("a watch needs a Tokio runtime"),
            Self::Settings(reason) => write!(f, "the hub's settings are refused: {reason}"),
            Self::Hubs { max } => write!(
                f,
                "{max} watch hubs live on this client already, each with its own range of \
                 subscriber process identifiers: end one first"
            ),
        }
    }
}

impl std::error::Error for WatchError {}

#[cfg(test)]
mod identity_tests;
#[cfg(test)]
mod lifecycle_tests;
#[cfg(test)]
mod move_cue_tests;
#[cfg(test)]
mod moves_tests;
#[cfg(test)]
mod naming_tests;
#[cfg(test)]
mod ranges_tests;
#[cfg(test)]
mod restart_tests;
#[cfg(test)]
mod scheduler_tests;
#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod stream_tests;
#[cfg(test)]
mod takeover_tests;
#[cfg(test)]
mod test_lab;
#[cfg(test)]
mod tests;
