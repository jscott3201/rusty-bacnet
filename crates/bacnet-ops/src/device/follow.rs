//! What the client keeps for its handles: where each followed instance is
//! reached now, shared by the handles following it (and the watch hubs
//! following it, #2795), with when they last asked where it went, and the
//! instances read by the wildcard for addresses the discovery table
//! doesn't hold. A following handle checks its address before each
//! request, and looks for its device again here after a request times out
//! or finds no route.
//!
//! The discovery table is filled by I-Ams nobody asked for, which anyone
//! can send, so what it says of a move is only a cue (#2795): before the
//! handles move, the device at the new address is asked which it is, and
//! only one naming the instance takes them there. Before a request, a
//! table saying another device announced itself at the handles' address
//! has that address asked first: still the device, nothing moves. After a
//! request of theirs there failed, only the new address is asked, as the
//! watch hub does after its own request went unanswered (#2812).

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use bacnet_client::client::DeviceEventKind;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::broadcast::error::TryRecvError;

use super::confirm::{Answer, left_silence, silence};
use super::resolve::{ambiguous, unknown};
use super::{ClientEvent, Device, DeviceAddress, Pin, QUIET};
use crate::client::{Client, ResolveScope, Scope, lock, timed_out};
use crate::watch::WatchError;

/// Where the handles following one instance on one network reach it, and
/// when they last asked where it went. They share it, so one that moves
/// takes the others with it, and one asking speaks for all of them.
#[derive(Debug)]
pub(crate) struct Place {
    /// Where the instance is reached now.
    pub(crate) at: DeviceAddress,
    /// When the last Who-Is for the instance went out after a timeout
    /// ([`Client::ask`]), on the discovery table's clock.
    asked: Option<Instant>,
}

/// A [`Place`], shared by the handles following one instance on one
/// network, and by the watch hubs following it.
pub(crate) type Followed = Arc<Mutex<Place>>;

/// What a following handle follows: the network it was named on (`2:1234`)
/// or any (`None`, for `1234`), and the instance.
pub(crate) type FollowKey = (Option<u16>, u32);

/// The client's follow table: each followed instance's [`Place`], for as
/// long as a handle holds it.
pub(crate) type Follows = HashMap<FollowKey, Weak<Mutex<Place>>>;

/// The most entries the client keeps in its follow table, and in its
/// memory of instances read by the wildcard: as many as the low-level
/// client's discovery table holds rows. When every entry is in use, making
/// room drops an arbitrary one, live or not: a remembered instance is read
/// again, and the handles of a dropped follow entry keep working and keep
/// sharing their place, but handles made after it share another.
pub(crate) const REMEMBERED: usize = 4096;

/// What the discovery table says of an instance besides one address.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Elsewhere {
    /// No other address claims it.
    Nowhere,
    /// One other address claims it, and no row at the one asked about
    /// does any more: its row is gone, or names another instance. The
    /// device moved there.
    Moved(DeviceAddress),
    /// One other address, `to`, claims it, and so does the one asked
    /// about: two devices sharing it, or one that moved while its old row
    /// stays. Only what each was last heard saying can tell them apart.
    Shared {
        /// The other address.
        to: DeviceAddress,
        /// When the one asked about was last heard from: its I-Am, or an
        /// answer to one of the client's requests.
        seen: Instant,
        /// When it last announced itself, if ever.
        announced: Option<Instant>,
        /// When `to` last announced itself, if ever.
        other: Option<Instant>,
    },
    /// Two or more other addresses claim it: every address claiming it.
    Contested(Vec<DeviceAddress>),
}

impl Elsewhere {
    /// Where the device moved, by the discovery table's announcements
    /// alone: the other address, when the one asked about has no row
    /// claiming the instance, or last announced itself at least [`QUIET`]
    /// before the other did.
    pub(super) fn by_announcements(self) -> Option<DeviceAddress> {
        match self {
            Self::Moved(to) => Some(to),
            Self::Shared {
                to,
                announced,
                other,
                ..
            } if announced_before(announced, other) => Some(to),
            Self::Shared { .. } | Self::Nowhere | Self::Contested(_) => None,
        }
    }
}

impl<T: TransportPort + 'static> Device<T> {
    /// Where to send the next request: a pinned handle's address, or where
    /// a following handle reaches its instance now. When the discovery
    /// table says another device announced itself at a following handle's
    /// address (and the device there hasn't named itself the handle's
    /// instance to the client since), with one other address claiming the
    /// instance on its network, both are the table's word: the device here
    /// is asked which it is (only a timeout is its silence), and still the
    /// instance, the handle stays. Otherwise the device at the other
    /// address is asked, and only when it names the instance does the
    /// handle move there, telling the events receivers (#2795).
    ///
    /// # Errors
    ///
    /// When another device announced itself at a following handle's
    /// address: [`Error::UnknownDevice`] with no other address claiming
    /// the instance, or one that doesn't answer as it, and
    /// [`Error::AmbiguousDevice`] with two or more. Nothing but those
    /// asks is sent then.
    pub(crate) async fn target(&self) -> Result<DeviceAddress, Error> {
        let here = self.address();
        let Pin::Follows {
            instance,
            network,
            at: followed,
        } = &self.pin
        else {
            return Ok(here);
        };
        let claimed = here
            .row(self.client.low_level())
            .await
            .is_none_or(|row| row.object_identifier.instance_number() == *instance)
            || self.client.said(&here) == Some(*instance);
        if claimed {
            return Ok(here);
        }
        let key = self.client.key(*network, *instance);
        match self.client.elsewhere(*instance, key.0, &here).await {
            Elsewhere::Moved(to) => {
                match self.client.which_at(&here, *instance, left_silence).await {
                    // The table's row here is stale or forged: the device's own
                    // answer says so, and is remembered (a read by the wildcard).
                    Answer::Is(found) | Answer::Has(found) if found == *instance => Ok(here),
                    // The client's own failure, or a router's reject: no word of
                    // the device's. The request fails as it would.
                    Answer::Failed => Ok(here),
                    _ => match self.client.answers_as(&to, *instance).await {
                        true => Ok(self.client.move_to(key, followed, &here, to)),
                        false => Err(unknown(*instance, *network)),
                    },
                }
            }
            // It claims the instance again by now.
            Elsewhere::Shared { .. } => Ok(here),
            Elsewhere::Nowhere => Err(unknown(*instance, *network)),
            Elsewhere::Contested(claims) => Err(ambiguous(*instance, &claims)),
        }
    }

    /// Where to send a request once more after `result` at `at`: for a
    /// following handle whose request timed out or was refused by a router
    /// on the way (#2724), or found no route to a routed address its row
    /// has left (nothing was sent then), where its instance went
    /// ([`Client::relocate`]); `None` to keep `result`. A local transport
    /// failure says nothing of the device, and a pinned handle never moves.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn retry<R>(
        &self,
        at: &DeviceAddress,
        result: &Result<R, Error>,
    ) -> Result<Option<DeviceAddress>, Error> {
        let Pin::Follows {
            instance,
            network,
            at: followed,
        } = &self.pin
        else {
            return Ok(None);
        };
        // A timeout, or a router that won't pass the request on, may ask
        // where the device went: an I-Am heard through another router moves
        // its row there. No route goes by the discovery table alone.
        let wait = match result {
            Err(error) if timed_out(error) || matches!(error, Error::NetworkRejected(_)) => {
                self.client.resolve_wait()
            }
            Err(Error::UnknownDevice { .. }) => None,
            _ => return Ok(None),
        };
        let key = self.client.key(*network, *instance);
        self.client.relocate(key, followed, at, wait).await
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// The follow-table key of a handle following `instance` named on
    /// `network`: this client's own network, once known, is the local one.
    pub(crate) fn key(&self, network: Option<u16>, instance: u32) -> FollowKey {
        (self.scope(network), instance)
    }

    /// The shared place of the handles following `key`, which the
    /// discovery table has at `address` now. Handles following it already
    /// stay where they reach it: the table's word moves none of them, only
    /// the device's answer to a request of theirs (#2795).
    pub(super) fn follow(&self, key: FollowKey, address: DeviceAddress) -> Followed {
        let mut follows = lock(&self.shared.follows);
        if let Some(followed) = follows.get(&key).and_then(Weak::upgrade) {
            return followed;
        }
        let followed = Arc::new(Mutex::new(Place {
            at: address,
            asked: None,
        }));
        keep_room(&mut follows, |entry| entry.strong_count() > 0);
        follows.insert(key, Arc::downgrade(&followed));
        followed
    }

    /// The place a watch hub follows `key`'s instance from (#2795): the one
    /// the client's handles share, wherever they reach it, else a new one
    /// at the one address claiming it in the discovery table now. Sends
    /// nothing.
    ///
    /// # Errors
    ///
    /// [`WatchError::UnknownDevice`] when no handle follows it and no
    /// address claims it, [`WatchError::Ambiguous`] when two or more do.
    pub(crate) fn follow_place(&self, key: FollowKey) -> Result<Followed, WatchError> {
        let mut follows = lock(&self.shared.follows);
        if let Some(followed) = follows.get(&key).and_then(Weak::upgrade) {
            return Ok(followed);
        }
        let (network, instance) = key;
        let (own, table) = self.low_level().localized_device_snapshot();
        let mut claims: Vec<DeviceAddress> = table
            .iter()
            .filter(|row| row.object_identifier.instance_number() == instance)
            .filter_map(DeviceAddress::of)
            .map(|address| address.localized(own))
            .filter(|address| network.is_none_or(|network| address.network() == network))
            .collect();
        claims.sort();
        claims.dedup();
        let address = match claims.as_slice() {
            [address] => address.clone(),
            [] => return Err(WatchError::UnknownDevice { instance, network }),
            _ => return Err(WatchError::Ambiguous { instance, claims }),
        };
        let followed = Arc::new(Mutex::new(Place {
            at: address,
            asked: None,
        }));
        keep_room(&mut follows, |entry| entry.strong_count() > 0);
        follows.insert(key, Arc::downgrade(&followed));
        Ok(followed)
    }

    /// Where `followed` reaches its instance now, as this client keys
    /// addresses.
    pub(crate) fn place_of(&self, followed: &Followed) -> DeviceAddress {
        self.localized(lock(followed).at.clone())
    }

    /// Whether the device at `address` answers as device `instance`: by
    /// the wildcard instance, or, for a device that refuses that, by having
    /// that instance's Device object (one request, two for a device refusing
    /// the wildcard for the first time).
    pub(crate) async fn answers_as(&self, address: &DeviceAddress, instance: u32) -> bool {
        self.which_at(address, instance, silence).await.instance() == Some(instance)
    }

    /// Where `key`'s instance is now, for a following handle whose request
    /// at `from` failed, once the device there names the instance
    /// ([`candidate`](Self::candidate) says where to ask): the handles move
    /// there, telling the events receivers. If another handle sharing
    /// `followed` moved it meanwhile, where it went. `None` to stay and keep
    /// the request's error: no candidate, or one whose device doesn't
    /// answer as the instance (#2795). The request at `from` just failed,
    /// so that address isn't asked again.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn relocate(
        &self,
        key: FollowKey,
        followed: &Followed,
        from: &DeviceAddress,
        wait: Option<Duration>,
    ) -> Result<Option<DeviceAddress>, Error> {
        let now = self.localized(lock(followed).at.clone());
        if now != *from {
            return Ok(Some(now));
        }
        let Some(to) = self.candidate(key, followed, from, wait).await? else {
            return Ok(None);
        };
        if !self.answers_as(&to, key.1).await {
            tracing::debug!(
                "device {} is said to answer at {to}, which doesn't answer as it; the handle \
                 stays at {from}",
                key.1
            );
            return Ok(None);
        }
        Ok(Some(self.move_to(key, followed, from, to)))
    }

    /// Where `key`'s instance may be now, for a following handle whose
    /// request at `from` timed out (`wait` its clone's resolve wait, `None`
    /// for a clone that sends no Who-Is) or found no route (`wait` `None`),
    /// with one other address claiming it: there when no row at `from`
    /// claims it any more. While `from` still claims it too, with `wait`
    /// `None` there when `from` last announced itself [`QUIET`] before the
    /// other did; otherwise there once only the other answered the Who-Is
    /// the handles last sent
    /// ([`only_other_answered`](Self::only_other_answered)), and until then
    /// `None`, asking ([`ask`](Self::ask)). With no other claim `None`,
    /// asking when `wait` is set. A candidate only: it moves nothing, and
    /// the device there hasn't said which it is.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn candidate(
        &self,
        key: FollowKey,
        followed: &Followed,
        from: &DeviceAddress,
        wait: Option<Duration>,
    ) -> Result<Option<DeviceAddress>, Error> {
        let (network, instance) = key;
        let to = match (self.elsewhere(instance, network, from).await, wait) {
            (Elsewhere::Contested(claims), _) => return Err(ambiguous(instance, &claims)),
            (Elsewhere::Moved(to), _) => to,
            (shared @ Elsewhere::Shared { .. }, None) => match shared.by_announcements() {
                Some(to) => to,
                None => return Ok(None),
            },
            (
                Elsewhere::Shared {
                    to, seen, other, ..
                },
                Some(wait),
            ) if self.only_other_answered(followed, wait, seen, other) => to,
            (Elsewhere::Shared { .. } | Elsewhere::Nowhere, Some(_)) => {
                self.ask(key, followed).await;
                return Ok(None);
            }
            (Elsewhere::Nowhere, None) => return Ok(None),
        };
        Ok(Some(to))
    }

    /// Whether only the other address answered the Who-Is the handles
    /// sharing `followed` last sent: it went at least `wait` ago, so its
    /// answers are in; the old address, last heard from at `seen`, hasn't
    /// been heard since it went, by an I-Am or an answer to a request; and
    /// the other, which last announced itself at `other`, has announced
    /// itself since.
    fn only_other_answered(
        &self,
        followed: &Followed,
        wait: Duration,
        seen: Instant,
        other: Option<Instant>,
    ) -> bool {
        let Some(asked) = lock(followed).asked else {
            return false;
        };
        let answered = now()
            .checked_duration_since(asked)
            .is_some_and(|since| since >= wait);
        answered && seen < asked && other.is_some_and(|other| other >= asked)
    }

    /// Send one Who-Is for `key`'s instance on the network where the
    /// handles sharing `followed` last reached it, never on every network,
    /// unless they sent one less than [`QUIET`] ago: who answers it decides
    /// their next timeout. A Who-Is that can't be sent is one asked all the
    /// same.
    async fn ask(&self, key: FollowKey, followed: &Followed) {
        let (_, instance) = key;
        let Ok(range) = DeviceInstanceRange::single(instance) else {
            return;
        };
        let asked = now();
        {
            let mut place = lock(followed);
            let recent = place.asked.is_some_and(|last| {
                asked
                    .checked_duration_since(last)
                    .is_none_or(|since| since < QUIET)
            });
            if recent {
                return;
            }
            place.asked = Some(asked);
        }
        // Where it was last heard: a moved device is claimed again there,
        // or, if it went to another network, by its I-Am there.
        let seen = self.localized(lock(followed).at.clone()).network();
        let scope = Scope::from(ResolveScope::on(seen));
        if let Err(error) = self.who_is(&scope, Some(range)).await {
            tracing::debug!("asking where device {instance} went: {error}");
        }
    }

    /// Move the handles sharing `followed` from `from` to `to`, telling the
    /// events receivers, unless another handle moved them meanwhile: where
    /// they are now. Only once the device at `to` named the instance in an
    /// answer to the client: a handle's own ([`relocate`](Self::relocate),
    /// [`Device::target`]) or a watch hub's (#2795).
    pub(crate) fn move_to(
        &self,
        key: FollowKey,
        followed: &Followed,
        from: &DeviceAddress,
        to: DeviceAddress,
    ) -> DeviceAddress {
        {
            let mut place = lock(followed);
            let now = self.localized(place.at.clone());
            if now != *from {
                return now;
            }
            place.at = to.clone();
        }
        self.moved(key.1, from.clone(), to.clone());
        // Shared again by the next handle made for it, if none holds it.
        let mut follows = lock(&self.shared.follows);
        if follows.get(&key).and_then(Weak::upgrade).is_none() {
            keep_room(&mut follows, |entry| entry.strong_count() > 0);
            follows.insert(key, Arc::downgrade(followed));
        }
        to
    }

    /// What the discovery table says of `instance` besides `here`, among
    /// the claims on `network` when one is named. Sends nothing.
    pub(super) async fn elsewhere(
        &self,
        instance: u32,
        network: Option<u16>,
        here: &DeviceAddress,
    ) -> Elsewhere {
        let claims = self.claims(instance, network).await;
        let mut others = claims.iter().filter(|address| *address != here);
        let to = match (others.next(), others.next()) {
            (None, _) => return Elsewhere::Nowhere,
            (Some(to), None) => to.clone(),
            (Some(_), Some(_)) => return Elsewhere::Contested(claims),
        };
        let own = here
            .row(self.low_level())
            .await
            .filter(|row| row.object_identifier.instance_number() == instance);
        let Some(own) = own else {
            return Elsewhere::Moved(to);
        };
        let other = to
            .row(self.low_level())
            .await
            .and_then(|row| row.last_announced);
        Elsewhere::Shared {
            to,
            seen: own.last_seen,
            announced: own.last_announced,
            other,
        }
    }

    /// Tell the events receivers that `instance` moved.
    fn moved(&self, instance: u32, from: DeviceAddress, to: DeviceAddress) {
        let _ = self
            .shared
            .events
            .send(ClientEvent::Moved { instance, from, to });
    }

    /// Remember that the device at `address` named itself `instance`.
    pub(super) fn remember(&self, address: DeviceAddress, instance: u32) {
        self.sweep();
        let mut instances = lock(&self.shared.instances);
        keep_room(&mut instances, |_| true);
        instances.insert(address, instance);
    }

    /// The instance the device at `address` named, as remembered.
    pub(super) fn remembered(&self, address: &DeviceAddress) -> Option<u32> {
        self.sweep();
        lock(&self.shared.instances).get(address).copied()
    }

    /// Forget what the discovery table's changes may have made stale: the
    /// instance remembered for an address whose row was lost (another
    /// device may answer there now) or where a device announced another
    /// instance, the ReadPropertyMultiple size limit learned there
    /// ([`Client::rpm_limit`]) on the same terms, and, after a loss, the
    /// follow table's entries no handle holds. After a burst of the table's
    /// changes this client missed, it forgets every remembered instance and
    /// limit: a later call learns them again.
    pub(crate) fn sweep(&self) {
        // Each address changed, with the instance announced there; `None`
        // for a row lost.
        let mut changed = Vec::new();
        let mut missed = false;
        {
            let mut changes = lock(&self.shared.changes);
            loop {
                match changes.try_recv() {
                    Ok(event) => {
                        let Some(address) = DeviceAddress::of(&event.device) else {
                            continue;
                        };
                        let named = (event.kind != DeviceEventKind::Lost)
                            .then(|| event.device.object_identifier.instance_number());
                        changed.push((self.localized(address), named));
                    }
                    Err(TryRecvError::Lagged(_)) => missed = true,
                    Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                }
            }
        }
        if changed.is_empty() && !missed {
            return;
        }
        {
            let mut instances = lock(&self.shared.instances);
            if missed {
                instances.clear();
            }
            for (address, named) in &changed {
                if instances
                    .get(address)
                    .is_some_and(|remembered| Some(*remembered) != *named)
                {
                    instances.remove(address);
                }
            }
        }
        {
            let mut limits = lock(&self.shared.rpm_limits);
            if missed {
                limits.clear();
            }
            for (address, named) in &changed {
                if limits
                    .get(address)
                    .is_some_and(|&(instance, _)| Some(instance) != *named)
                {
                    limits.remove(address);
                }
            }
        }
        if missed || changed.iter().any(|(_, named)| named.is_none()) {
            lock(&self.shared.follows).retain(|_, entry| entry.strong_count() > 0);
        }

        // And the COV support learned there, on the same terms (#2603).
        self.forget_cov(&changed, missed);
    }
}

/// Make room in `map` for one more entry under [`REMEMBERED`]: drop the
/// entries `alive` says nothing uses, then any one entry while still full.
pub(crate) fn keep_room<K: Eq + Hash + Clone, V>(
    map: &mut HashMap<K, V>,
    alive: impl Fn(&V) -> bool,
) {
    if map.len() < REMEMBERED {
        return;
    }
    map.retain(|_, value| alive(value));
    while map.len() >= REMEMBERED {
        let Some(key) = map.keys().next().cloned() else {
            break;
        };
        map.remove(&key);
    }
}

/// Whether an announcement at `own` came at least [`QUIET`] before one at
/// `other`. A row never announced, registered by hand, says nothing of
/// when its device was last there, so it is never quiet by this alone.
pub(super) fn announced_before(own: Option<Instant>, other: Option<Instant>) -> bool {
    match (own, other) {
        (Some(own), Some(other)) => other.checked_duration_since(own) >= Some(QUIET),
        _ => false,
    }
}

/// Now on the discovery table's clock, which is tokio's: a paused test
/// clock moves it.
fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}
