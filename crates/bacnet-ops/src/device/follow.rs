//! What the client keeps for its handles: where each followed instance is
//! reached now, shared by the handles following it, with when they last
//! asked where it went, and the instances read by the wildcard for
//! addresses the discovery table doesn't hold. A following handle checks
//! its address before each request, and looks for its device again here
//! after a request times out or finds no route.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use bacnet_client::client::DeviceEventKind;
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::broadcast::error::TryRecvError;

use super::resolve::{ambiguous, unknown};
use super::{ClientEvent, Device, DeviceAddress, Pin, QUIET};
use crate::client::{Client, WhoIsScope, lock, timed_out};

/// Where the handles following one instance on one network reach it, and
/// when they last asked where it went. They share it, so one that moves
/// takes the others with it, and one asking speaks for all of them.
#[derive(Debug)]
pub(crate) struct Place {
    /// Where the instance is reached now.
    pub(super) at: DeviceAddress,
    /// When the last Who-Is for the instance went out after a timeout
    /// ([`Client::ask`]), on the discovery table's clock.
    asked: Option<Instant>,
}

/// A [`Place`], shared by the handles following one instance on one
/// network.
pub(super) type Followed = Arc<Mutex<Place>>;

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
    /// a following handle reaches its instance now. When another device
    /// has announced itself at a following handle's address, the instance
    /// is elsewhere: the handle moves to the one other address claiming it
    /// on its network, telling the events receivers.
    ///
    /// # Errors
    ///
    /// When another device announced itself at a following handle's
    /// address: [`Error::UnknownDevice`] with no other address claiming
    /// the instance, and [`Error::AmbiguousDevice`] with two or more.
    /// Nothing is sent then.
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
        let claimed = self
            .client
            .row_at(&here)
            .await
            .is_none_or(|row| row.object_identifier.instance_number() == *instance);
        if claimed {
            return Ok(here);
        }
        let key = self.client.key(*network, *instance);
        match self.client.elsewhere(*instance, key.0, &here).await {
            Elsewhere::Moved(to) => Ok(self.client.move_to(key, followed, &here, to)),
            // It claims the instance again by now.
            Elsewhere::Shared { .. } => Ok(here),
            Elsewhere::Nowhere => Err(unknown(*instance, key.0)),
            Elsewhere::Contested(claims) => Err(ambiguous(*instance, &claims)),
        }
    }

    /// Where to send a request once more after `result` at `at`: for a
    /// following handle whose request timed out, or found no route to a
    /// routed address its row has left (nothing was sent then), where its
    /// instance went ([`Client::relocate`]); `None` to keep `result`. A
    /// local transport failure says nothing of the device, and a pinned
    /// handle never moves.
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
        // A timeout may ask where the device went; no route goes by the
        // discovery table alone.
        let wait = match result {
            Err(error) if timed_out(error) => self.client.resolve_wait(),
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
    pub(super) fn key(&self, network: Option<u16>, instance: u32) -> FollowKey {
        (self.scope(network), instance)
    }

    /// The shared place of the handles following `key`, which is at
    /// `address` now: a handle still following it there or elsewhere
    /// moves with it, telling the events receivers.
    pub(super) fn follow(&self, key: FollowKey, address: DeviceAddress) -> Followed {
        let mut follows = lock(&self.shared.follows);
        if let Some(followed) = follows.get(&key).and_then(Weak::upgrade) {
            let from = std::mem::replace(&mut lock(&followed).at, address.clone());
            let from = self.localized(from);
            if from != address {
                self.moved(key.1, from, address);
            }
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

    /// Where `key`'s instance is now, for a following handle whose request
    /// at `from` timed out (`wait` its clone's resolve wait, `None` for a
    /// clone that sends no Who-Is) or found no route (`wait` `None`), with
    /// one other address claiming it: there when no row at `from` claims
    /// it any more. While `from` still claims it too, with `wait` `None`
    /// there when `from` last announced itself [`QUIET`] before the other
    /// did; otherwise there once only the other answered the Who-Is the
    /// handles last sent ([`only_other_answered`](Self::only_other_answered)),
    /// and until then the handle stays and asks ([`ask`](Self::ask)). With
    /// no other claim the handle stays, asking when `wait` is set. If
    /// another handle sharing `followed` moved it meanwhile, where it went.
    /// `None` to stay and keep the request's error.
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
        let (network, instance) = key;
        let now = self.localized(lock(followed).at.clone());
        if now != *from {
            return Ok(Some(now));
        }
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
        Ok(Some(self.move_to(key, followed, from, to)))
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

    /// Send one Who-Is for `key`'s instance on its network (on every
    /// network for `None`), unless the handles sharing `followed` sent one
    /// less than [`QUIET`] ago: who answers it decides their next timeout.
    /// A Who-Is that can't be sent is one asked all the same.
    async fn ask(&self, key: FollowKey, followed: &Followed) {
        let (network, instance) = key;
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
        if let Err(error) = self.who_is(WhoIsScope::on(network), Some(range)).await {
            tracing::debug!("asking where device {instance} went: {error}");
        }
    }

    /// Move the handles sharing `followed` from `from` to `to`, telling the
    /// events receivers, unless another handle moved them meanwhile: where
    /// they are now.
    pub(super) fn move_to(
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
        let own = self
            .row_at(here)
            .await
            .filter(|row| row.object_identifier.instance_number() == instance);
        let Some(own) = own else {
            return Elsewhere::Moved(to);
        };
        let other = self.row_at(&to).await.and_then(|row| row.last_announced);
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
    /// instance, and, after a loss, the follow table's entries no handle
    /// holds. After a burst of the table's changes this client missed, it
    /// forgets every remembered instance: a later call reads one again.
    pub(super) fn sweep(&self) {
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
        if missed || changed.iter().any(|(_, named)| named.is_none()) {
            lock(&self.shared.follows).retain(|_, entry| entry.strong_count() > 0);
        }
    }
}

/// Make room in `map` for one more entry under [`REMEMBERED`]: drop the
/// entries `alive` says nothing uses, then any one entry while still full.
pub(super) fn keep_room<K: Eq + Hash + Clone, V>(
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
