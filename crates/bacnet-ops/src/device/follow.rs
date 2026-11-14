//! What the client keeps for its handles: where each followed instance is
//! reached now, shared by the handles following it, and the instances
//! read by the wildcard for addresses the discovery table doesn't hold. A
//! following handle looks for its device again here after a request times
//! out or finds no route.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use bacnet_client::client::DeviceEventKind;
use bacnet_transport::port::TransportPort;
use bacnet_types::error::Error;
use tokio::sync::broadcast::error::TryRecvError;

use super::resolve::ambiguous;
use super::{ClientEvent, Device, DeviceAddress, Pin, QUIET};
use crate::client::{Client, lock, timed_out};

/// Where the handles following one instance on one network reach it now.
/// They share it, so one that moves takes the others with it.
pub(super) type Followed = Arc<Mutex<DeviceAddress>>;

/// What a following handle follows: the network it was named on (`2:1234`)
/// or any (`None`, for `1234`), and the instance.
pub(crate) type FollowKey = (Option<u16>, u32);

/// The client's follow table: each followed instance's shared address, for
/// as long as a handle holds it.
pub(crate) type Follows = HashMap<FollowKey, Weak<Mutex<DeviceAddress>>>;

/// The most entries the client keeps in its follow table, and in its
/// memory of instances read by the wildcard: as many as the low-level
/// client's discovery table holds rows.
pub(crate) const REMEMBERED: usize = 4096;

/// What the discovery table says of an instance besides one address.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Elsewhere {
    /// No other address claims it.
    Nowhere,
    /// One other address claims it, and the one asked about has gone
    /// quiet: its row is gone, or claims another instance, or it last
    /// announced itself at least [`QUIET`] before the other did. The device
    /// moved there.
    Moved(DeviceAddress),
    /// One other address claims it, and the one asked about still does
    /// and isn't quiet: two devices share the instance.
    Duplicate,
    /// Two or more other addresses claim it: every address claiming it.
    Contested(Vec<DeviceAddress>),
}

impl<T: TransportPort + 'static> Device<T> {
    /// Where to send a request once more after `result` at `at`: for a
    /// following handle whose request timed out, or found no route to a
    /// routed address its row has left (nothing was sent then), the one
    /// other address its instance moved to ([`Client::relocate`]); `None`
    /// to keep `result`. A local transport failure says nothing of the
    /// device, and a pinned handle never moves.
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
        match result {
            Err(error) if timed_out(error) || matches!(error, Error::UnknownDevice { .. }) => {
                self.client
                    .relocate((*network, *instance), followed, at)
                    .await
            }
            _ => Ok(None),
        }
    }
}

impl<T: TransportPort + 'static> Client<T> {
    /// The shared address of the handles following `key`, which is
    /// `address` now: a handle still following it there or elsewhere
    /// moves with it, telling the events receivers.
    pub(super) fn follow(&self, key: FollowKey, address: DeviceAddress) -> Followed {
        let mut follows = lock(&self.shared.follows);
        if let Some(followed) = follows.get(&key).and_then(Weak::upgrade) {
            let from = std::mem::replace(&mut *lock(&followed), address.clone());
            if from != address {
                self.moved(key.1, from, address);
            }
            return followed;
        }
        let followed = Arc::new(Mutex::new(address));
        keep_room(&mut follows, |entry| entry.strong_count() > 0);
        follows.insert(key, Arc::downgrade(&followed));
        followed
    }

    /// Where `key`'s instance is now, for a following handle that found it
    /// gone at `from`: the one other address claiming it, once `from` has
    /// gone quiet ([`Elsewhere::Moved`]), followed from now on; `None` when
    /// there is none, or `from` still holds the instance as one of two. If
    /// another handle sharing `followed` moved it meanwhile, where it went.
    ///
    /// # Errors
    ///
    /// [`Error::AmbiguousDevice`] when two or more other addresses claim it.
    pub(crate) async fn relocate(
        &self,
        key: FollowKey,
        followed: &Followed,
        from: &DeviceAddress,
    ) -> Result<Option<DeviceAddress>, Error> {
        let (network, instance) = key;
        let now = lock(followed).clone();
        if now != *from {
            return Ok(Some(now));
        }
        let to = match self.elsewhere(instance, network, from).await {
            Elsewhere::Moved(to) => to,
            Elsewhere::Nowhere | Elsewhere::Duplicate => return Ok(None),
            Elsewhere::Contested(claims) => return Err(ambiguous(instance, &claims)),
        };
        {
            let mut at = lock(followed);
            if *at != *from {
                return Ok(Some(at.clone()));
            }
            *at = to.clone();
        }
        self.moved(instance, from.clone(), to.clone());
        // Shared again by the next handle made for it, if none holds it.
        let mut follows = lock(&self.shared.follows);
        if follows.get(&key).and_then(Weak::upgrade).is_none() {
            keep_room(&mut follows, |entry| entry.strong_count() > 0);
            follows.insert(key, Arc::downgrade(followed));
        }
        Ok(Some(to))
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
        let low = self.low_level();
        let own = low
            .get_device_at(here.network(), here.mac())
            .await
            .filter(|row| row.object_identifier.instance_number() == instance);
        let quiet = match own {
            None => true,
            Some(own) => {
                let other = low.get_device_at(to.network(), to.mac()).await;
                announced_before(own.last_announced, other.and_then(|row| row.last_announced))
            }
        };
        if quiet {
            Elsewhere::Moved(to)
        } else {
            Elsewhere::Duplicate
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

    /// Forget what the discovery table's lost rows may have made stale:
    /// the instance remembered for each one's address (another device may
    /// answer there now), and the follow table's entries no handle holds.
    /// After a burst of the table's changes this client missed, it forgets
    /// every remembered instance: a later call reads one again.
    pub(super) fn sweep(&self) {
        let mut lost = Vec::new();
        let mut missed = false;
        {
            let mut events = lock(&self.shared.lost);
            loop {
                match events.try_recv() {
                    Ok(event) if event.kind == DeviceEventKind::Lost => {
                        lost.extend(DeviceAddress::of(&event.device));
                    }
                    Ok(_) => {}
                    Err(TryRecvError::Lagged(_)) => missed = true,
                    Err(TryRecvError::Empty | TryRecvError::Closed) => break,
                }
            }
        }
        if lost.is_empty() && !missed {
            return;
        }
        {
            let mut instances = lock(&self.shared.instances);
            if missed {
                instances.clear();
            }
            for address in &lost {
                instances.remove(address);
            }
        }
        lock(&self.shared.follows).retain(|_, entry| entry.strong_count() > 0);
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
/// `other`; a row never announced counts as quiet.
pub(super) fn announced_before(own: Option<Instant>, other: Option<Instant>) -> bool {
    match (own, other) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(own), Some(other)) => other.checked_duration_since(own) >= Some(QUIET),
    }
}
