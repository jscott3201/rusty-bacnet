//! Watching points: one managed BACnet subscription or poller per point,
//! however many listeners share it (#2067, #2560).
//!
//! An application calls [`Client::watch`](crate::Client::watch) with
//! points from [`Device::point`](crate::Device::point) (or
//! [`Device::watch`](crate::Device::watch) with the points' names) and
//! reads the [`Watch`], a stream of [`Update`]s: each point's latest
//! value, its Status_Flags and when it came, and its [`State`], which
//! holds the reason ([`Why`]) whenever the point isn't subscribed. Its
//! [`WatchControl`] adds and removes points while another task reads it.
//! The client's [`WatchSettings`] (lifetime, poll interval, subscribe or
//! poll only) are its builder's, and
//! [`Client::watch_with`](crate::Client::watch_with) takes others for one
//! watch; watches with equal settings share one hub. That
//! is the one way an application watches. Under it is [`hub`], for a
//! front end that runs its own hub (the `bacnet` tool's watch list and MCP
//! server keep a history, serve several sessions and pick their own
//! settings); one that picks its own settings but reads updates as an
//! application does takes a [`Watch`] from its hub
//! ([`Hub::stream`](hub::Hub::stream)).
//!
//! A point names its device by address or by instance ([`Binding`]): a
//! pinned handle's points are at its address, a following handle's follow
//! its instance (#2795). The hub keys its entries on the point, and runs
//! one scheduler a device address, which a following point is handed to
//! when its device moves; so two devices sharing an instance, each named by
//! its address, are two devices, and one announcing itself again at its
//! address keeps its entries. Each point is read once, then
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
//! where a following handle's rule would take it (after a timeout, at most
//! one Who-Is every [`QUIET`](crate::device::QUIET)), moving no handle.
//! That address, or one a following handle moved to for a request of its
//! own, is only a cue (#2812): the device there is asked which it is, by one
//! read of its Device object by the wildcard instance, paced as a claim's
//! asks, and the points are given up as moved ([`Why::Moved`]) only when
//! that answer names their instance and the device is gone from their
//! address: unless that address is failing already, its device is asked
//! once more, and one still answering as the device there is a second
//! claimant, which changes nothing. Another instance, no answer, or a
//! device refusing that read changes nothing. A row merely gone from the
//! discovery table (aged out of a quiet device) is no move, nor are
//! announcement times: a duplicate starting up while the device misses a
//! request looks the same. The point stays unreachable, polled after its
//! backoff. A move doesn't
//! stick: a point started there later starts as any other, and the
//! given-up points start again once the address announces itself for the
//! instance, or answers a start while the discovery table names it there.
//! A pinned point stays given up; watch it at the new address, or follow
//! the instance.
//!
//! A following point is never given up for a move: on the same answers
//! the hub hands it, with its subscriber process identifier and its
//! values, to the scheduler at the new address, which reads and subscribes
//! it there; its [`Update::moved`] says where it went, and the client's
//! handles following the instance move with it when the hub asked their
//! old address (not when it took the move after its own request there went
//! unanswered). Its renewals at the old
//! address stop and nothing is cancelled there (the device there is
//! another or silent), so that subscription lapses with its lifetime.
//! While another device answers at its address a following point is
//! unreachable ([`Why::Replaced`]) as the hub looks for its device. The
//! hub follows each instance from one place, so following points added
//! meanwhile start where it follows the device then, and a move it took
//! takes them all.
//!
//! Lifetimes are finite by construction ([`hub::Lifetime`] can't be zero)
//! and at most eight hours, the longest every device must take: a
//! subscription that never lapses is a write in the `bacnet` tool's
//! command classes (`bacnet_tool::class`), and the hub never sends one.

pub mod hub;

mod control;
mod device;
mod dispatch;
mod jobs;
mod options;
mod point;
mod pooled;
mod sample;
mod settings;
mod shared;
mod slot;
mod stream;
mod support;

pub use self::control::WatchControl;
pub use self::point::{Binding, Moved, Point, Watched, Why};
pub use self::sample::Via;
pub(crate) use self::settings::Pool;
pub use self::settings::WatchSettings;
pub use self::stream::{MAX_WATCHED, Mark, State, Update, Watch};
pub use self::support::CovSupport;
pub(crate) use self::support::{CovMemory, services};

use bacnet_types::error::Refusal;

/// Why a watch, or a change to one, was refused: nothing was watched or
/// changed, and nothing sent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WatchError {
    /// The points would take the hub past the most it watches at once,
    /// points still cancelling included ([`MAX_WATCHED`] for a client's
    /// pooled hubs, [`hub::WatchOptions::max_points`] for another).
    Full {
        /// The most.
        max: usize,
    },
    /// The hub has shut down.
    Shut,
    /// Called outside a Tokio runtime, which the hub's tasks need.
    NoRuntime,
    /// Settings a hub doesn't take, which
    /// [`WatchOptions::validate`](hub::WatchOptions::validate) (or
    /// [`Lifetime::new`](hub::Lifetime::new)) refuses: the text says which
    /// and why. It is for a person; the limits a caller acts on are
    /// constants ([`Lifetime::MAX`](hub::Lifetime::MAX),
    /// [`hub::MAX_POLL`], [`hub::POLL_FLOOR`]).
    Settings(Refusal),
    /// Every range of subscriber process identifiers on the client is held:
    /// this many hubs live on it, its pooled ones included (one a set of
    /// watch settings in use), or are still cancelling. End one first.
    Hubs {
        /// The most.
        max: usize,
    },
    /// The watch has ended: dropped, closed
    /// ([`WatchControl::close`]), its stream ended, or its hub shut down.
    /// A [`WatchControl`] of it can no longer change it.
    Closed,
    /// A following point's device can't be found (#2795): no handle on the
    /// client follows its instance and no address claims it in the
    /// discovery table, on its network when it names one. Make a handle on
    /// the device first ([`Client::device`](crate::Client::device)), which
    /// asks for it.
    UnknownDevice {
        /// The instance.
        instance: u32,
        /// The network named, if any.
        network: Option<u16>,
    },
    /// Two or more addresses claim a following point's instance in the
    /// discovery table, and no handle on the client follows it (#2795):
    /// name the device by one of them.
    Ambiguous {
        /// The instance.
        instance: u32,
        /// Every address claiming it, ordered by network and then MAC.
        candidates: Vec<crate::DeviceAddress>,
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
            Self::Settings(reason) => write!(f, "the watch settings are refused: {reason}"),
            Self::Hubs { max } => write!(
                f,
                "{max} watch hubs live on this client already, one for each set of watch \
                 settings, each with its own range of subscriber process identifiers: end one \
                 first"
            ),
            Self::Closed => f.write_str("the watch has ended"),
            Self::UnknownDevice {
                instance,
                network: Some(network),
            } => write!(f, "no address claims device {network}:{instance}"),
            Self::UnknownDevice { instance, .. } => {
                write!(f, "no address claims device {instance}")
            }
            Self::Ambiguous {
                instance,
                candidates,
            } => {
                let claims: Vec<String> = candidates.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "device {instance} is claimed by {} addresses ({}): name it by one",
                    claims.len(),
                    claims.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for WatchError {}

#[cfg(test)]
mod asked_tests;
#[cfg(test)]
mod claimants_tests;
#[cfg(test)]
mod control_tests;
#[cfg(test)]
mod follow_race_tests;
#[cfg(test)]
mod follow_tests;
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
mod race_tests;
#[cfg(test)]
mod ranges_tests;
#[cfg(test)]
mod restart_tests;
#[cfg(test)]
mod scheduler_tests;
#[cfg(test)]
mod settings_tests;
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
