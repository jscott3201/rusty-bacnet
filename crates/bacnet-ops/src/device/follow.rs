//! What the client keeps for its handles: where each followed instance is
//! reached now, shared by the handles following it (and the watch hubs
//! following it, #2795), with when they last asked where it went, and the
//! instances read by the wildcard for addresses the discovery table
//! doesn't hold. A following handle checks its address before each
//! request, and looks for its device again here after a request times out
//! or finds no route.
//!
//! The discovery table is filled by I-Ams nobody asked for, which anyone
//! can send, so what it says of a move is only a cue (#2795). A read moves
//! the handles only once the device at their own address was asked which
//! it is and isn't the instance (silent, by a timeout alone, or another
//! device), and the device at the new address names it; a read of the
//! instance's own Device object there counts only then. A write never
//! moves them: it goes to their place while the device there is still
//! taken for the instance, or nowhere.

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
use crate::error::OpsError;
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
    /// Where to send the next read: a pinned handle's address, or where a
    /// following handle reaches its instance now. When the discovery table
    /// says another device announced itself at a following handle's
    /// address (and the device there hasn't named itself the handle's
    /// instance to the client since), that is the table's word, which
    /// I-Ams nobody asked for fill: the device here is asked which it is
    /// first, whatever else the table says (#2795). Still the instance, or
    /// no word of the device's (it answered with another error, or the
    /// client failed the ask), the handle stays. Otherwise only one other
    /// address claiming the instance whose device answers as it takes the
    /// handle there, telling the events receivers; its answer by the
    /// instance's own Device object counts as the handle's own address was
    /// asked first.
    ///
    /// # Errors
    ///
    /// When the device here isn't the instance:
    /// [`Error::UnknownDevice`] with no other address claiming it, or one
    /// that doesn't answer as it, and [`Error::AmbiguousDevice`] with two
    /// or more. Nothing but those asks is sent then.
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
        if self.holds(&here, *instance).await || self.client.stays(&here, *instance).await {
            return Ok(here);
        }
        let key = self.client.key(*network, *instance);
        match self.client.elsewhere(*instance, key.0, &here).await {
            Elsewhere::Moved(to) | Elsewhere::Shared { to, .. } => {
                match self.client.answers_as(&to, *instance).await {
                    true => Ok(self.client.move_to(key, followed, &here, to)),
                    false => Err(unknown(*instance, *network)),
                }
            }
            Elsewhere::Nowhere => Err(unknown(*instance, *network)),
            Elsewhere::Contested(claims) => Err(ambiguous(*instance, &claims)),
        }
    }

    /// Where to send the next write or relinquish: a pinned handle's
    /// address, or the place a following handle already holds, only while
    /// the device there is taken for its instance: its table row is absent
    /// or names it (neither the device's own answer, nor aged), or the
    /// device's last own answer there named it; otherwise it is asked once
    /// here, and an answer as the instance, an unclear one or the client's
    /// own failure keeps the write there. A write never moves a handle
    /// (#2795): else nothing is sent, and the error names where the table
    /// has the instance now, unconfirmed.
    ///
    /// # Errors
    ///
    /// [`OpsError::NotConfirmed`] when the device at the held place, asked,
    /// is silent or another device. Nothing but the ask is sent then.
    pub(crate) async fn write_target(&self) -> Result<DeviceAddress, OpsError> {
        let here = self.address();
        let Pin::Follows { instance, .. } = &self.pin else {
            return Ok(here);
        };
        if self.holds(&here, *instance).await || self.client.stays(&here, *instance).await {
            return Ok(here);
        }
        Err(self.not_confirmed(here).await)
    }

    /// The error of a write a following handle won't send from `here`:
    /// with the one other address the discovery table has its instance at,
    /// as a hint. Sends nothing.
    pub(crate) async fn not_confirmed(&self, here: DeviceAddress) -> OpsError {
        let (instance, network) = match &self.pin {
            Pin::Follows {
                instance, network, ..
            } => (*instance, *network),
            Pin::At(_) => (self.instance.unwrap_or_default(), None),
        };
        let key = self.client.key(network, instance);
        let candidate = match self.client.elsewhere(instance, key.0, &here).await {
            Elsewhere::Moved(to) | Elsewhere::Shared { to, .. } => Some(to),
            Elsewhere::Nowhere | Elsewhere::Contested(_) => None,
        };
        OpsError::NotConfirmed {
            instance,
            at: here,
            candidate,
        }
    }

    /// Whether the handle's place `here` is still the instance's by what
    /// the client holds, sending nothing: the discovery table has no other
    /// device there, or the device there last named itself the instance
    /// to the client.
    async fn holds(&self, here: &DeviceAddress, instance: u32) -> bool {
        here.row(self.client.low_level())
            .await
            .is_none_or(|row| row.object_identifier.instance_number() == instance)
            || self.client.said(here) == Some(instance)
    }

    /// Where to send a read once more after `result` at `at`: for a
    /// following handle whose request timed out or was refused by a router
    /// on the way (#2724), or found no route to a routed address its row
    /// has left (nothing was sent then), where its instance went
    /// ([`Client::relocate`]); `None` to keep `result`. A local transport
    /// failure says nothing of the device, and a pinned handle never moves.
    /// Writes never come here (#2795).
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
        // its row there. No route asks for the station again first.
        let (wait, routeless) = match result {
            Err(error) if timed_out(error) || matches!(error, Error::NetworkRejected(_)) => {
                (self.client.resolve_wait(), false)
            }
            Err(Error::UnknownDevice { .. }) => (None, true),
            _ => return Ok(None),
        };
        let key = self.client.key(*network, *instance);
        self.client
            .relocate(key, followed, at, (wait, routeless))
            .await
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
            _ => {
                return Err(WatchError::Ambiguous {
                    instance,
                    candidates: claims,
                });
            }
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
    /// the wildcard for the first time). The second only counts for a place
    /// a handle moves to once its own place was asked and didn't answer as
    /// the instance (#2795): its callers ask that first.
    pub(crate) async fn answers_as(&self, address: &DeviceAddress, instance: u32) -> bool {
        self.which_at(address, instance, silence).await.instance() == Some(instance)
    }

    /// Whether a handle following `instance` stays at `here`, the device
    /// there asked which it is (only a timeout is its silence): it answers
    /// as the instance, or says nothing of which it is (another error, a
    /// Reject or an Abort), or the client failed the ask, a router's reject
    /// included. Only silence or another device moves a handle (#2795).
    pub(crate) async fn stays(&self, here: &DeviceAddress, instance: u32) -> bool {
        match self.which_at(here, instance, left_silence).await {
            Answer::Is(found) | Answer::Has(found) => found == instance,
            Answer::Unclear | Answer::Failed => true,
            Answer::Not | Answer::Silent => false,
        }
    }

    /// Where `key`'s instance is now, for a following handle whose read at
    /// `from` failed (`failure`: the clone's resolve wait for a timeout or
    /// a router's reject, `None` for a clone that sends no Who-Is, and
    /// whether no route to `from` was known): the handles move to the
    /// candidate ([`candidate`](Self::candidate)) only once the device at
    /// `from` was asked and doesn't answer as the instance
    /// ([`stays`](Self::stays)) and the candidate's does
    /// ([`answers_as`](Self::answers_as)), telling the events receivers.
    /// With no route to `from`, its station is asked for first, by a
    /// Who-Is for the instance addressed to it, then one to its network:
    /// found as the instance, the read goes there again; silent, or
    /// another device answering, that is its answer. If another handle
    /// sharing `followed` moved it meanwhile, where it went. `None` to stay
    /// and keep the read's error (#2795).
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn relocate(
        &self,
        key: FollowKey,
        followed: &Followed,
        from: &DeviceAddress,
        (wait, routeless): (Option<Duration>, bool),
    ) -> Result<Option<DeviceAddress>, Error> {
        let now = self.localized(lock(followed).at.clone());
        if now != *from {
            return Ok(Some(now));
        }
        if routeless {
            let range = DeviceInstanceRange::single(key.1).ok();
            match self.find_station(from, range).await {
                // Found as the instance: the read goes there again.
                Ok(row) if row.object_identifier.instance_number() == key.1 => {
                    return Ok(Some(from.clone()));
                }
                // Another device answers there, or none, asked: look
                // elsewhere.
                Ok(_) | Err(OpsError::UnknownDevice { asked: Some(_), .. }) => {}
                // Nothing asked (a clone that sends no Who-Is), or the
                // client failed: no word of the device's.
                Err(_) => return Ok(None),
            }
        }
        let Some(to) = self.candidate(key, followed, from, wait).await? else {
            return Ok(None);
        };
        if !routeless && self.stays(from, key.1).await {
            tracing::debug!(
                "device {} is said to answer at {to}, and still answers at {from}; the handle \
                 stays",
                key.1
            );
            return Ok(None);
        }
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
