//! A device named by its instance, placed for the first time (#2824): the
//! discovery table is filled by I-Ams nobody asked for, so its claims are
//! only cues. Unless a row the application registered by hand claims the
//! instance, one Who-Is for it goes out first and its answers are given
//! the settle time, so the device itself claims the instance too when it
//! is on the network. Then every claim is asked which device answers
//! there, and the answers decide ([`placed`]). A device's earlier answer,
//! remembered, saves asking its address again, and nothing more: the
//! device it answered for may be elsewhere too. One resolution of an
//! instance runs at a time, and resolutions waiting on it share its
//! Who-Is.

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Instant;

use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::TransportPort;
use tokio::sync::OwnedMutexGuard;

use super::DeviceAddress;
use super::confirm::{Answer, every, left_silence, over_cap, placed};
use super::follow::{FollowKey, Joining, keep_room};
use super::resolve::SETTLE;
use crate::client::{Client, ResolveScope, Scope, lock};
use crate::error::OpsError;

/// The turns of the resolutions of each instance, by follow key, while one
/// runs or waits: the time the last Who-Is for it went out, which a
/// resolution that waited meanwhile shares rather than sending its own.
pub(crate) type Resolving = HashMap<FollowKey, Weak<tokio::sync::Mutex<Option<Instant>>>>;

/// A resolution's turn for its instance: the time the last Who-Is for it
/// went out, set by the resolution that sends one.
pub(super) type Turn = OwnedMutexGuard<Option<Instant>>;

/// Where a watch hub's following point stands after its place is settled
/// ([`Client::settle_place`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Settled {
    /// Its device confirmed the instance at the point's address.
    Here,
    /// Not confirmed at the point's address: the hub's own start goes on
    /// there, asking which device answers after its first read, and its
    /// reads find the device where it is.
    Open,
    /// Two or more addresses may be the device: none is watched as it.
    Ambiguous,
}

impl<T: TransportPort + 'static> Client<T> {
    /// The turn of one resolution of `key`'s instance; the others wait.
    pub(super) async fn resolving(&self, key: FollowKey) -> Turn {
        let gate = {
            let mut gates = lock(&self.shared.resolving);
            match gates.get(&key).and_then(Weak::upgrade) {
                Some(gate) => gate,
                None => {
                    let gate = Arc::new(tokio::sync::Mutex::new(None));
                    keep_room(&mut gates, |entry| entry.strong_count() > 0);
                    gates.insert(key, Arc::downgrade(&gate));
                    gate
                }
            }
        };
        gate.lock_owned().await
    }

    /// Whether a row registered by hand, never announced, claims
    /// `instance` at `claim`: the application's own word, as an address it
    /// names is, which stands without a Who-Is.
    async fn registered(&self, claim: &DeviceAddress, instance: u32) -> bool {
        claim.row(self.low_level()).await.is_some_and(|row| {
            row.last_announced.is_none() && row.object_identifier.instance_number() == instance
        })
    }

    /// Whether any of `claims` is [`registered`](Self::registered) for
    /// `instance`.
    async fn any_registered(&self, claims: &[DeviceAddress], instance: u32) -> bool {
        for claim in claims {
            if self.registered(claim, instance).await {
                return true;
            }
        }
        false
    }

    /// What `claim` is known to say of being device `instance` without
    /// asking it, and whether that is vouched for: a row registered by hand
    /// is ([`registered`](Self::registered)); the device's earlier answer
    /// to the client, as remembered, isn't, and only saves asking again.
    async fn known_answer(&self, claim: &DeviceAddress, instance: u32) -> Option<(Answer, bool)> {
        if self.registered(claim, instance).await {
            return Some((Answer::Is(instance), true));
        }
        (self.confirmed_as(claim) == Some(instance)).then(|| match self.said(claim) {
            Some(_) => (Answer::Is(instance), false),
            None => (Answer::Has(instance), false),
        })
    }

    /// Where device `instance` is placed among `claims` (#2824): each claim
    /// is asked which device answers there ([`which_at`](Self::which_at),
    /// all at once) unless its answer is known (`known`, or
    /// [`known_answer`](Self::known_answer)), and the answers decide
    /// ([`placed`], [`over_cap`]). A device refusing the wildcard that has
    /// the instance's Device object is remembered so. With what each claim
    /// answered.
    pub(super) async fn confirm(
        &self,
        instance: u32,
        claims: &[DeviceAddress],
        known: Option<&(DeviceAddress, Answer)>,
    ) -> (
        Result<DeviceAddress, OpsError>,
        Vec<(DeviceAddress, Answer)>,
    ) {
        let mut before = Vec::with_capacity(claims.len());
        for claim in claims {
            let (answer, vouched) = match known {
                Some((at, answer)) if at == claim => (Some(*answer), false),
                _ => match self.known_answer(claim, instance).await {
                    Some((answer, vouched)) => (Some(answer), vouched),
                    None => (None, false),
                },
            };
            before.push((claim.clone(), answer, vouched));
        }
        if let Some(decided) = over_cap(instance, &before) {
            return (decided, Vec::new());
        }
        let asks = before.iter().map(|(claim, known, _)| async move {
            match known {
                Some(answer) => *answer,
                None => self.asked(claim, instance).await,
            }
        });
        let answers = every(asks).await;
        let asked: Vec<(DeviceAddress, Answer)> = claims.iter().cloned().zip(answers).collect();
        for (claim, answer) in &asked {
            tracing::debug!("device {instance} is claimed at {claim}, which answered {answer:?}");
        }
        (placed(instance, &asked), asked)
    }

    /// The device at `claim` asked which it is, for device `instance`:
    /// only a timeout is its silence.
    async fn asked(&self, claim: &DeviceAddress, instance: u32) -> Answer {
        let answer = self.which_at(claim, instance, left_silence).await;
        if answer == Answer::Has(instance) {
            self.vouch(claim.clone(), instance);
        }
        answer
    }

    /// The one of `claims` whose device confirmed device `instance` to the
    /// client's own question (#2824): for a front end that checks every
    /// address against its policy before anything goes there, so these
    /// claims alone are asked, and no Who-Is is sent. A claim whose device
    /// confirmed it before, or a row registered by hand, isn't asked again;
    /// over [`MAX_ASKED`](super::MAX_ASKED) claims, only a row registered by
    /// hand places it.
    ///
    /// # Errors
    ///
    /// As [`placed`]: [`OpsError::NotConfirmed`] for one claim that didn't
    /// confirm it, [`OpsError::Ambiguous`] for several.
    pub async fn confirm_among(
        &self,
        instance: u32,
        claims: &[DeviceAddress],
    ) -> Result<DeviceAddress, OpsError> {
        self.confirm(instance, claims, None).await.0
    }

    /// The instance the device at `address` confirmed to one of this
    /// client's own questions, as remembered (#2824), or a row registered
    /// there by hand names. Sends nothing.
    pub fn confirmed_at(&self, address: &DeviceAddress) -> Option<u32> {
        if let Some(instance) = self.confirmed_as(address) {
            return Some(instance);
        }
        let (own, table) = self.low_level().localized_device_snapshot();
        table
            .iter()
            .filter(|row| row.last_announced.is_none())
            .find(|row| DeviceAddress::of(row).is_some_and(|at| at.localized(own) == *address))
            .map(|row| row.object_identifier.instance_number())
    }

    /// Where `key`'s instance (named on `named`) is placed when no place
    /// of the handles following it can be joined (`live`: that place's
    /// address, with what its device answered when asked already): its
    /// claims in the discovery table and `live`'s address. Unless a row
    /// registered by hand is among them, one Who-Is for the instance goes
    /// out first (unless one went out while this resolution waited its
    /// `turn`, `since`), and its answers are given [`SETTLE`]; with no claim
    /// at all, the resolve wait for a first answer ([`Client::find_instance`]).
    /// An answer only remembered sends it all the same. Then
    /// [`confirm`](Self::confirm). With what each claim answered.
    pub(super) async fn resolve_claims(
        &self,
        (key, named): (FollowKey, Option<u16>),
        live: Option<(DeviceAddress, Option<Answer>)>,
        (turn, since): (&mut Turn, Instant),
    ) -> Result<(DeviceAddress, Vec<(DeviceAddress, Answer)>), OpsError> {
        let (network, instance) = key;
        let held = live.as_ref().map(|(here, _)| here.clone());
        let known = live.and_then(|(here, answer)| Some((here, answer?)));
        let mut claims = self.claims_and(instance, network, held.as_ref()).await;
        if !self.any_registered(&claims, instance).await {
            // Shared: a resolution of the same instance sent one while this
            // one waited its turn.
            let shared = turn.is_some_and(|sent| sent >= since);
            if claims.is_empty() && !shared {
                **turn = Some(now());
                self.find_instance(instance, named).await?;
            } else if !shared && self.resolve_wait().is_some() {
                **turn = Some(now());
                self.who_is_for(instance, network).await?;
                tokio::time::sleep(SETTLE).await;
            }
            claims = self.claims_and(instance, network, held.as_ref()).await;
        }
        if claims.is_empty() {
            return Err(self.missing(instance, named));
        }
        let (placed, asked) = self.confirm(instance, &claims, known.as_ref()).await;
        placed.map(|address| (address, asked))
    }

    /// The claims to `instance` on `network`, with `held` among them.
    async fn claims_and(
        &self,
        instance: u32,
        network: Option<u16>,
        held: Option<&DeviceAddress>,
    ) -> Vec<DeviceAddress> {
        let mut claims = self.claims(instance, network).await;
        if let Some(held) = held {
            claims.push(held.clone());
            claims.sort();
            claims.dedup();
        }
        claims
    }

    /// One Who-Is for `instance`, to `network` when the target names one
    /// (this client's own as a local broadcast) and to this clone's
    /// [`resolve_scope`](Self::resolve_scope) otherwise, as a lookup that
    /// finds no claim sends it.
    async fn who_is_for(&self, instance: u32, network: Option<u16>) -> Result<(), OpsError> {
        let range = DeviceInstanceRange::single(instance)?;
        let sent_to = network.map_or(self.resolve_scope(), ResolveScope::on);
        self.who_is(&Scope::from(sent_to), Some(range)).await
    }

    /// Settle the place a watch hub's following point of `key`'s instance
    /// is watched at, `here`, before its first read (#2824), once a place:
    /// a place confirmed already, and still holding the instance, stands;
    /// else, unless a row registered by hand claims it, one Who-Is for the
    /// instance goes first, and its answers are given the settle time. The
    /// place's own address alone claiming it then is left to the hub's
    /// start, which asks it after its first read and takes nothing from
    /// another device; two or more claims are asked as a handle's first
    /// placement asks them, and the place is marked confirmed when its own
    /// device confirms the instance. A place is never moved by this: the
    /// hub's own reads move its points.
    pub(crate) async fn settle_place(&self, key: FollowKey, here: &DeviceAddress) -> Settled {
        let since = now();
        let mut turn = self.resolving(key).await;
        let live = match self.joining(key).await {
            Joining::Join(place) => {
                return match self.place_of(&place) == *here {
                    true => Settled::Here,
                    false => Settled::Open,
                };
            }
            Joining::Ask(live) => live,
        };
        let Some(place) = self.live_place(key) else {
            return Settled::Open;
        };
        if std::mem::replace(&mut lock(&place).settled, true) {
            return Settled::Open;
        }
        let (network, instance) = key;
        let claims = self.claims_and(instance, network, Some(here)).await;
        let registered = self.any_registered(&claims, instance).await;
        if !registered && !turn.is_some_and(|sent| sent >= since) && self.resolve_wait().is_some() {
            *turn = Some(now());
            if self.who_is_for(instance, network).await.is_ok() {
                tokio::time::sleep(SETTLE).await;
            }
        }
        if self.claims_and(instance, network, Some(here)).await == [here.clone()] {
            return Settled::Open;
        }
        let resolved = self
            .resolve_claims((key, key.0), live, (&mut turn, since))
            .await;
        match resolved.and_then(|(address, asked)| self.place_confirmed(key, address, &asked)) {
            Ok(place) if self.place_of(&place) == *here => Settled::Here,
            Err(OpsError::Ambiguous { .. }) => Settled::Ambiguous,
            Ok(_) | Err(_) => Settled::Open,
        }
    }
}

/// Now on the discovery table's clock, which is tokio's.
fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}
