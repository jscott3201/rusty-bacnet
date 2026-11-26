use bacnet_types::sync::MutexExt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::{Duration, Instant as StdInstant};

use tokio::time::Instant as TokioInstant;

use crate::poll_gate::{PollGate, PollGateGuard};

use super::*;

/// The smallest NPDU, in octets, a router must be able to forward between
/// any two data links, the lowest of the per-link figures in Table 6-1
/// (Clause 6.6). It is the limit of a routed path nothing was set or learned
/// for ([`RoutedPathLimitSource::Default`]),
/// and also the floor of what a path can learn: a conforming router never
/// refuses a frame this small as too long, so the client does not believe a
/// reason-4 reject for a request whose largest frame is no larger.
pub const DEFAULT_ROUTED_PATH_MAX_NPDU: u16 = 228;
/// Hard bound covering gates, configured evidence, and learned evidence.
pub(super) const MAX_ROUTED_PATH_ENTRIES: usize = 256;

/// Whether a router MAC or forwarded source MAC fits a routed path: 1 to
/// [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) octets. The NPDU codec carries no longer SADR
/// or DADR (#1141), and the network layer drops a frame from a longer
/// link-layer source (#1198), so a longer MAC could never complete a routed
/// request (#1267).
fn routed_mac_fits(length: usize) -> bool {
    (1..=bacnet_npdu::MAX_MAC_LEN).contains(&length)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct RoutedPathKey {
    immediate_router_mac: MacAddr,
    dnet: u16,
}

impl RoutedPathKey {
    fn new(immediate_router_mac: &[u8], dnet: u16) -> Self {
        Self {
            immediate_router_mac: MacAddr::from_slice(immediate_router_mac),
            dnet,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ConfiguredLimitProvenance {
    ExplicitApi,
}

struct ConfiguredPathLimit {
    max_npdu: u16,
    provenance: ConfiguredLimitProvenance,
    observed_at: StdInstant,
}

struct LearnedPathLimit {
    exclusive_max_npdu: u16,
    observed_at: StdInstant,
}

struct ActiveRoutedSend {
    path: RoutedPathKey,
    tsm_mac: MacAddr,
    invoke_id: u8,
    owner: TransactionOwner,
    forwarded_npci_len: u16,
    ingress_floor: u64,
}

/// What ended the generation on a path, as far as its frames go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GenerationEnd {
    /// Nothing has: a frame it sent may still draw a reason 4.
    Open,
    /// The device answered, with an acknowledgment or its Error, Reject or
    /// Abort, so the size of each frame crossed the path (Clause 19.4).
    Answered,
    /// A reason 4 ended a one-frame generation while no earlier frame of
    /// this client's confirmed requests on the path could still draw one,
    /// so among those requests the reject was that frame's own. Other
    /// traffic to the network sits outside the lease, as before: this
    /// client's unconfirmed requests, a node's server replies and
    /// notifications, and another router to the same network.
    Refused,
}

/// The largest frame of the answered generations on a path whose frames
/// may still draw a reason 4, and until when. Within that time a reason 4
/// can belong to such a frame, so it fails a request only when that
/// request's largest frame is no smaller: path limits fall with size, so
/// whichever frame the reject was for, the request's own cannot pass.
/// Clause 19.4 attributes a reason 4 the same way.
#[derive(Clone, Copy)]
struct AnsweredFrames {
    largest_npdu: u16,
    until: TokioInstant,
}

struct RoutedPathEntry {
    /// One request at a time on the path, taken only by a request as it is
    /// polled ([`PollGate`], #2759).
    gate: Arc<PollGate>,
    configured: Option<ConfiguredPathLimit>,
    learned: Option<LearnedPathLimit>,
    active: Option<ActiveRoutedSend>,
    /// The TSM key of the request that holds the path's gate, from the
    /// moment it is sized ([`RoutedPathLease::open`]) until it is installed
    /// ([`RoutedPathLimits::install_active`]): the TSM holds a request
    /// before it is installed, and in that time this is what ties the
    /// request to the path for [`RoutedPathLimits::answer_through`]
    /// (#2831). The same copy of the key then moves into the active record.
    opening: Option<MacAddr>,
    last_used: u64,
    quarantine_started: Option<TokioInstant>,
    answered: Option<AnsweredFrames>,
    generation_attempts: u32,
    /// The largest forwarded NPDU the generation has sent, 0 before its
    /// first. A reason 4 may be for any of its frames, so a bound is learned
    /// from this one, never from a smaller frame sent after it.
    generation_largest_npdu: u16,
    generation_end: GenerationEnd,
    /// Whether a below-minimum drop on this path has been logged since its
    /// limit was last configured or cleared.
    below_minimum_warned: bool,
    /// A table-wide sequence number, unique to this entry's latest limit
    /// change: its creation, a configure or a clear. A reason 4 for a frame
    /// sized before a change teaches nothing after it
    /// ([`RoutedPathLimits::commit_learned_limit`]).
    changed: u64,
    /// [`changed`](Self::changed) as it stood when the path's latest
    /// request read its limit ([`RoutedPathLease::open`]). A change
    /// applies at once, even while a request sized before it is in flight
    /// (#2683), so a reason 4 is judged by the limit its request was sized
    /// to, not the one in force when it lands.
    sized_under: u64,
    /// A busy reject (reason 2) tied to a request on the path, which names
    /// the router if that request times out (#2724). Written on the receive
    /// side only; a later request's owner never matches it.
    busy: Option<Box<rejects::BusyNote>>,
}

impl RoutedPathEntry {
    fn new(last_used: u64) -> Self {
        Self {
            gate: Arc::new(PollGate::default()),
            configured: None,
            learned: None,
            active: None,
            opening: None,
            last_used,
            quarantine_started: None,
            answered: None,
            generation_attempts: 0,
            generation_largest_npdu: 0,
            generation_end: GenerationEnd::Open,
            below_minimum_warned: false,
            changed: last_used,
            sized_under: last_used,
            busy: None,
        }
    }

    /// Start a new generation: nothing sent, nothing ended.
    fn reset_generation(&mut self) {
        self.generation_attempts = 0;
        self.generation_largest_npdu = 0;
        self.generation_end = GenerationEnd::Open;
    }

    /// [`Self::answered`] while it can still matter at `now`.
    fn answered_at(&self, now: TokioInstant) -> Option<AnsweredFrames> {
        self.answered.filter(|answered| answered.until > now)
    }
}

struct RoutedPathState {
    entries: HashMap<RoutedPathKey, RoutedPathEntry>,
    next_use: u64,
    capacity: usize,
    /// Reason-4 rejects dropped because they could belong to an earlier
    /// answered generation's frame.
    dropped_rejects: u64,
    /// Reason-4 rejects dropped because the active request's largest frame
    /// is within the conservative limit.
    below_minimum_drops: u64,
    /// Answers dropped for coming by another path than their request took
    /// ([`RoutedPathLimits::answer_through`],
    /// [`RoutedPathLimits::count_off_path_answer`]).
    off_path_answers: u64,
}

pub(super) struct RoutedPathLimits {
    state: StdMutex<RoutedPathState>,
    quarantine_horizon: Duration,
}

impl RoutedPathLimits {
    pub(super) fn new(quarantine_horizon: Duration) -> Self {
        Self::with_capacity(MAX_ROUTED_PATH_ENTRIES, quarantine_horizon)
    }

    pub(super) fn with_capacity(capacity: usize, quarantine_horizon: Duration) -> Self {
        assert!(capacity > 0, "routed path capacity must be nonzero");
        Self {
            state: StdMutex::new(RoutedPathState {
                entries: HashMap::new(),
                next_use: 0,
                capacity,
                dropped_rejects: 0,
                below_minimum_drops: 0,
                off_path_answers: 0,
            }),
            quarantine_horizon,
        }
    }

    fn state(&self) -> StdMutexGuard<'_, RoutedPathState> {
        self.state.locked()
    }

    /// See [`BACnetClient::too_long_reject_drops`].
    pub(super) fn dropped_rejects(&self) -> u64 {
        self.state().dropped_rejects
    }

    /// See [`BACnetClient::below_minimum_reject_drops`].
    pub(super) fn below_minimum_drops(&self) -> u64 {
        self.state().below_minimum_drops
    }

    /// See [`BACnetClient::off_path_answer_drops`].
    pub(super) fn off_path_answers(&self) -> u64 {
        self.state().off_path_answers
    }

    /// Counts an answer refused for the path it came by elsewhere than
    /// [`Self::answer_through`]: one relayed with this network as its SNET
    /// for a request sent straight to the station it names (#2831).
    pub(super) fn count_off_path_answer(&self) {
        let mut state = self.state();
        state.off_path_answers = state.off_path_answers.saturating_add(1);
    }

    /// Whether an answer from link station `from` to transaction
    /// (`tsm_mac`, `invoke_id`), whose SNET names `network`, may complete
    /// it (#2831). Every request through a router is the one active request
    /// on its path (router, DNET) while it is open, so the path it came
    /// through has it active. One open through another router is answered
    /// by someone else: the answer is counted and refused. Any other answer
    /// goes on as before: one for a request sent by local broadcast, which
    /// any router may relay and which takes no path, and one for no open
    /// request, which the TSM drops.
    pub(super) fn answer_through(
        &self,
        from: &[u8],
        network: u16,
        tsm_mac: &MacAddr,
        invoke_id: u8,
    ) -> bool {
        // Before its install the open request's invoke ID isn't on the
        // path yet, only its key: any answer to that key is its.
        let answers = |entry: &RoutedPathEntry| {
            entry.opening.as_ref() == Some(tsm_mac)
                || entry.active.as_ref().is_some_and(|active| {
                    active.invoke_id == invoke_id && active.tsm_mac == *tsm_mac
                })
        };
        let mut state = self.state();
        if state
            .entries
            .get(&RoutedPathKey::new(from, network))
            .is_some_and(answers)
        {
            return true;
        }
        let open_elsewhere = state.entries.values().any(answers);
        if open_elsewhere {
            state.off_path_answers = state.off_path_answers.saturating_add(1);
        }
        !open_elsewhere
    }

    /// What [`BACnetClient::routed_path_limits`] lists: each path with a
    /// configured or learned limit or a below-minimum reject seen, by router
    /// MAC and then network. One lock, as every other reader takes it.
    pub(super) fn listing(&self) -> Vec<super::RoutedPathLimit> {
        let state = self.state();
        let mut paths: Vec<super::RoutedPathLimit> = state
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.configured.is_some() || entry.learned.is_some() || entry.below_minimum_warned
            })
            .map(|(key, entry)| super::RoutedPathLimit {
                router_mac: key.immediate_router_mac.clone(),
                dnet: key.dnet,
                configured_max_npdu: entry.configured.as_ref().map(|limit| limit.max_npdu),
                learned_max_npdu: entry
                    .learned
                    .as_ref()
                    .map(|limit| limit.exclusive_max_npdu.saturating_sub(1)),
                below_minimum_reject_seen: entry.below_minimum_warned,
            })
            .collect();
        drop(state);
        paths.sort_by(|a, b| (&a.router_mac, a.dnet).cmp(&(&b.router_mac, b.dnet)));
        paths
    }

    /// Whether the path's below-minimum warning has fired and not been reset.
    #[cfg(test)]
    pub(super) fn below_minimum_warned(&self, router_mac: &[u8], dnet: u16) -> bool {
        self.state()
            .entries
            .get(&RoutedPathKey::new(router_mac, dnet))
            .is_some_and(|entry| entry.below_minimum_warned)
    }

    /// How many path entries exist, in use or idle.
    #[cfg(test)]
    pub(super) fn entry_count(&self) -> usize {
        self.state().entries.len()
    }

    /// Reserve and lock the path through `router_mac` to `dnet`, and wait
    /// out any hold an earlier request left on it. A router MAC outside
    /// [`routed_mac_fits`] is refused before any entry is reserved.
    pub(super) async fn acquire(
        self: &Arc<Self>,
        router_mac: &[u8],
        dnet: u16,
    ) -> Result<RoutedPathLease, Error> {
        let lease = self.lock_path(router_mac, dnet).await?;
        self.wait_for_quarantine(&lease.key).await;
        Ok(lease)
    }

    /// Reserve the path's entry and take its gate. A router MAC outside
    /// [`routed_mac_fits`] is refused before any entry is reserved. The
    /// gate goes to a request only as it is polled (#2759), so one left
    /// unpolled while it waits its turn holds the path for nobody; the
    /// order is [`PollGate`]'s. A request that has the gate holds it until
    /// its lease drops, polled or not. The wait's handle on the gate keeps
    /// the entry resident.
    async fn lock_path(
        self: &Arc<Self>,
        router_mac: &[u8],
        dnet: u16,
    ) -> Result<RoutedPathLease, Error> {
        check_router_mac(router_mac)?;
        let key = RoutedPathKey::new(router_mac, dnet);
        let gate = self.reserve_gate(&key)?;
        let gate = gate.lock_owned().await;
        Ok(RoutedPathLease {
            limits: Arc::clone(self),
            key,
            _gate: gate,
        })
    }

    fn reserve_gate(&self, key: &RoutedPathKey) -> Result<Arc<PollGate>, Error> {
        let mut state = self.state();
        let last_used = bump_use(&mut state);
        if let Some(entry) = state.entries.get_mut(key) {
            entry.last_used = last_used;
            return Ok(Arc::clone(&entry.gate));
        }

        self.make_room(&mut state)?;
        let entry = RoutedPathEntry::new(last_used);
        let gate = Arc::clone(&entry.gate);
        state.entries.insert(key.clone(), entry);
        Ok(gate)
    }

    /// Room for one more entry: on a full table, reclaim the least recently
    /// used entry that [`is_safely_reclaimable`](Self::is_safely_reclaimable),
    /// or refuse with [`Error::RoutedPathCapacityExceeded`].
    fn make_room(&self, state: &mut RoutedPathState) -> Result<(), Error> {
        if state.entries.len() >= state.capacity {
            let now = TokioInstant::now();
            let reclaim = state
                .entries
                .iter()
                .filter(|(_, entry)| self.is_safely_reclaimable(entry, now))
                .min_by(|(key_a, entry_a), (key_b, entry_b)| {
                    entry_a
                        .last_used
                        .cmp(&entry_b.last_used)
                        .then_with(|| key_a.cmp(key_b))
                })
                .map(|(key, _)| key.clone());
            if let Some(reclaim) = reclaim {
                state.entries.remove(&reclaim);
            }
        }
        if state.entries.len() >= state.capacity {
            return Err(Error::RoutedPathCapacityExceeded {
                capacity: state.capacity,
            });
        }
        Ok(())
    }

    /// Set the path's limit to `max_npdu`, reserving its entry if the table
    /// holds none, and drop what it learned. One lock, no wait: no request
    /// sizes itself while it is held, and a request sized before keeps its
    /// size ([`RoutedPathEntry::sized_under`]). A hold on the path stays as
    /// it is. See [`BACnetClient::configure_routed_path_max_npdu`].
    pub(super) fn configure(
        &self,
        router_mac: &[u8],
        dnet: u16,
        max_npdu: u16,
    ) -> Result<(), Error> {
        check_router_mac(router_mac)?;
        let key = RoutedPathKey::new(router_mac, dnet);
        let mut guard = self.state();
        let state = &mut *guard;
        if !state.entries.contains_key(&key) {
            self.make_room(state)?;
            let created = bump_use(state);
            state
                .entries
                .insert(key.clone(), RoutedPathEntry::new(created));
        }
        let changed = bump_use(state);
        let entry = state
            .entries
            .get_mut(&key)
            .expect("the entry was found or made under this lock");
        entry.last_used = changed;
        entry.configured = Some(ConfiguredPathLimit {
            max_npdu,
            provenance: ConfiguredLimitProvenance::ExplicitApi,
            observed_at: StdInstant::now(),
        });
        entry.learned = None;
        entry.below_minimum_warned = false;
        entry.changed = changed;
        Ok(())
    }

    /// Put the path back on the default: drop its configured and learned
    /// limits. A path the table holds no entry for is left alone, with none
    /// reserved, so a clear never fails for capacity. The check and the
    /// clear take one lock (#2667). See
    /// [`BACnetClient::clear_routed_path_limit`].
    pub(super) fn clear(&self, router_mac: &[u8], dnet: u16) -> Result<(), Error> {
        check_router_mac(router_mac)?;
        let key = RoutedPathKey::new(router_mac, dnet);
        let mut guard = self.state();
        let state = &mut *guard;
        let changed = bump_use(state);
        if let Some(entry) = state.entries.get_mut(&key) {
            entry.configured = None;
            entry.learned = None;
            entry.below_minimum_warned = false;
            entry.changed = changed;
        }
        Ok(())
    }

    /// Whether `entry` holds nothing a later request depends on at `now`,
    /// read once for the whole scan of a full table.
    fn is_safely_reclaimable(&self, entry: &RoutedPathEntry, now: TokioInstant) -> bool {
        entry.configured.is_none()
            && entry.learned.is_none()
            && entry.active.is_none()
            && Arc::strong_count(&entry.gate) == 1
            && entry.quarantine_started.is_none_or(|started| {
                now.saturating_duration_since(started) >= self.quarantine_horizon
            })
            && entry.answered.is_none_or(|answered| answered.until <= now)
    }

    async fn wait_for_quarantine(&self, key: &RoutedPathKey) {
        loop {
            let remaining = {
                let mut state = self.state();
                let last_used = bump_use(&mut state);
                let entry = state
                    .entries
                    .get_mut(key)
                    .expect("a held or waiting gate keeps its path entry resident");
                entry.last_used = last_used;
                match entry.quarantine_started {
                    Some(started) => {
                        let remaining = self.quarantine_horizon.saturating_sub(started.elapsed());
                        if remaining.is_zero() {
                            entry.quarantine_started = None;
                            entry.reset_generation();
                            None
                        } else {
                            Some(remaining)
                        }
                    }
                    None => {
                        entry.reset_generation();
                        None
                    }
                }
            };
            let Some(remaining) = remaining else {
                return;
            };
            tokio::time::sleep(remaining).await;
        }
    }

    /// Make the request the TSM registered under `tsm_mac` and `invoke_id`
    /// the path's active one, taking the key its
    /// [`open`](RoutedPathLease::open) left on the path.
    pub(super) fn install_active(
        &self,
        target: ConfirmedTarget<'_>,
        tsm_mac: &[u8],
        invoke_id: u8,
        owner: TransactionOwner,
        forwarded_npci_len: u16,
        ingress_floor: u64,
    ) {
        let Some((router_mac, dest_network)) = target.routed_path() else {
            return;
        };
        let key = RoutedPathKey::new(router_mac, dest_network);
        let mut state = self.state();
        let entry = state
            .entries
            .get_mut(&key)
            .expect("the routed path lease keeps its entry resident");
        debug_assert!(
            entry.active.is_none(),
            "the per-path gate permits only one active routed request"
        );
        // A request is opened on its path before it is installed; a caller
        // that skipped the open gets its own copy of the key.
        let tsm_mac = entry
            .opening
            .take()
            .unwrap_or_else(|| MacAddr::from_slice(tsm_mac));
        entry.active = Some(ActiveRoutedSend {
            path: key,
            tsm_mac,
            invoke_id,
            owner,
            forwarded_npci_len,
            ingress_floor,
        });
    }

    pub(super) fn authorize_attempt(
        &self,
        target: ConfirmedTarget<'_>,
        tsm_mac: &MacAddr,
        invoke_id: u8,
        owner: &TransactionOwner,
        outgoing_apdu_len: usize,
    ) -> bool {
        let Some((router_mac, dest_network)) = target.routed_path() else {
            return true;
        };
        let Ok(outgoing_apdu_len) = u16::try_from(outgoing_apdu_len) else {
            return false;
        };
        let key = RoutedPathKey::new(router_mac, dest_network);
        let mut state = self.state();
        let Some(entry) = state.entries.get_mut(&key) else {
            return false;
        };
        let Some(active) = entry.active.as_ref() else {
            return false;
        };
        if active.tsm_mac != *tsm_mac
            || active.invoke_id != invoke_id
            || !active.owner.same_as(owner)
        {
            return false;
        }
        let npdu = active.forwarded_npci_len.saturating_add(outgoing_apdu_len);
        entry.generation_largest_npdu = entry.generation_largest_npdu.max(npdu);
        entry.generation_attempts = entry.generation_attempts.saturating_add(1);
        true
    }

    /// Learn from a reason 4 for a request sized while the path's limit was
    /// last changed at `changed`. A path whose limit changed since, or whose
    /// entry is gone, learns nothing: the change reset learned evidence, and
    /// the reject was for a frame sized before it.
    fn commit_learned_limit(&self, active: ActiveRoutedSend, attempted_npdu: u16, changed: u64) {
        let mut state = self.state();
        let Some(entry) = state
            .entries
            .get_mut(&active.path)
            .filter(|entry| entry.changed == changed)
        else {
            drop(state);
            tracing::debug!(
                router_mac = ?active.path.immediate_router_mac,
                dnet = active.path.dnet,
                npdu_octets = attempted_npdu,
                "Learned no routed-path limit from a reason-4 (too long) network reject: \
                 the path's limit was set or cleared after the request was sized, or the \
                 path left the table"
            );
            return;
        };
        // A learned limit stays at the conservative one or above, whatever
        // the caller passes (`claim_active` already drops smaller frames).
        let attempted_npdu = attempted_npdu.max(DEFAULT_ROUTED_PATH_MAX_NPDU + 1);
        let exclusive_max_npdu = entry.learned.as_ref().map_or(attempted_npdu, |learned| {
            learned.exclusive_max_npdu.min(attempted_npdu)
        });
        entry.learned = Some(LearnedPathLimit {
            exclusive_max_npdu,
            observed_at: StdInstant::now(),
        });
    }
}

pub(super) struct RoutedPathLease {
    limits: Arc<RoutedPathLimits>,
    key: RoutedPathKey,
    _gate: PollGateGuard<Arc<PollGate>>,
}

impl RoutedPathLease {
    /// Open the path for the request whose TSM key is `tsm_mac`, before the
    /// TSM holds it ([`RoutedPathEntry::opening`]), and return the APDU
    /// allowance left on this path after the forwarded NPCI that
    /// [`forwarded_npci_len`] measured. It also records the limit's version
    /// the request is sized under ([`RoutedPathEntry::sized_under`]).
    pub(super) fn open(&self, tsm_mac: &[u8], forwarded_npci_len: u16) -> u16 {
        let mut state = self.limits.state();
        let entry = state
            .entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.sized_under = entry.changed;
        // `from_slice` copies at once; this is the request's one copy of the
        // key on the path, which its install takes.
        entry.opening = Some(MacAddr::from_slice(tsm_mac));
        let configured_or_conservative =
            entry
                .configured
                .as_ref()
                .map_or(DEFAULT_ROUTED_PATH_MAX_NPDU, |configured| {
                    tracing::trace!(
                        max_npdu = configured.max_npdu,
                        provenance = ?configured.provenance,
                        age = ?configured.observed_at.elapsed(),
                        "Using configured routed-path NPDU limit"
                    );
                    configured.max_npdu
                });
        let effective_npdu = entry
            .learned
            .as_ref()
            .map_or(configured_or_conservative, |learned| {
                tracing::trace!(
                    exclusive_max_npdu = learned.exclusive_max_npdu,
                    age = ?learned.observed_at.elapsed(),
                    "Applying learned routed-path upper bound"
                );
                configured_or_conservative.min(learned.exclusive_max_npdu.saturating_sub(1))
            });
        effective_npdu.saturating_sub(forwarded_npci_len)
    }

    /// Mark the current generation answered by the device: an
    /// acknowledgment, or its Error, Reject or Abort, never a router's
    /// refusal ([`answered_by_peer`]). The path is then free as soon as the
    /// lease drops, however many frames the generation sent; the lease's
    /// `Drop` implementation says what a generation with more than one
    /// leaves behind.
    pub(super) fn mark_answered(&self) {
        let mut state = self.limits.state();
        let entry = state
            .entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.generation_end = GenerationEnd::Answered;
        entry.busy = None;
    }
}

/// Ending a generation frees its path at once unless a frame it sent may
/// still draw a reason 4 that nothing could tell from the next request's:
///
/// - Answered by the device after one frame, or refused for its one frame:
///   nothing of it is still in flight.
/// - Answered after several frames (a retry, or segments): each frame's
///   size crossed the path, so the path is free, and the largest frame is
///   kept for the next window of the horizon. A reason 4 within it fails
///   only a request at least that large ([`AnsweredFrames`]).
/// - Anything else that sent a frame (no answer, a send failure, the
///   caller's cancel, or a reason 4 after several frames, which is the
///   router's refusal and not an answer) holds the path for the horizon.
impl Drop for RoutedPathLease {
    fn drop(&mut self) {
        let horizon = self.limits.quarantine_horizon;
        let mut state = self.limits.state();
        if let Some(entry) = state.entries.get_mut(&self.key) {
            entry.active = None;
            entry.opening = None;
            match (entry.generation_end, entry.generation_attempts) {
                (_, 0) | (GenerationEnd::Answered, 1) | (GenerationEnd::Refused, _) => {}
                (GenerationEnd::Answered, _) => {
                    let now = TokioInstant::now();
                    let largest_npdu = entry
                        .answered_at(now)
                        .map_or(entry.generation_largest_npdu, |answered| {
                            answered.largest_npdu.max(entry.generation_largest_npdu)
                        });
                    entry.answered = Some(AnsweredFrames {
                        largest_npdu,
                        until: now + horizon,
                    });
                }
                (GenerationEnd::Open, _) => {
                    entry.quarantine_started = Some(TokioInstant::now());
                }
            }
            entry.reset_generation();
        }
    }
}

fn bump_use(state: &mut RoutedPathState) -> u64 {
    state.next_use = state.next_use.saturating_add(1);
    state.next_use
}

pub(super) fn routed_path_quarantine_horizon(config: &ClientConfig) -> Duration {
    // Reuse the configured complete request/retry horizon instead of adding a
    // separate arbitrary stale-control timer: it is how long a path is held
    // after a generation that may still draw a reason 4, and how long an
    // answered generation's largest frame gates one. A zero timeout still
    // receives one millisecond, the existing configuration's millisecond
    // granularity, so neither is ever skipped entirely.
    let attempts = u64::from(config.apdu_retries).saturating_add(1);
    Duration::from_millis(config.apdu_timeout_ms.saturating_mul(attempts).max(1))
}

/// The NPCI a router adds when it forwards a request to the DADR: the fixed
/// fields plus DADR and the SADR it fills with the local source MAC. A DADR or
/// source MAC outside [`routed_mac_fits`] is refused (#1267): an empty DADR
/// would have the remote router broadcast the request (#1278).
pub(super) fn forwarded_npci_len(
    dadr_len: usize,
    local_source_mac_len: usize,
) -> Result<u16, Error> {
    check_routed_dadr(dadr_len)?;
    if !routed_mac_fits(local_source_mac_len) {
        // The client's own MAC, a value its link keeps.
        return Err(Error::InvalidConfig(
            Refusal::new(
                "local source MAC address for routed forwarding must contain 1..={} octets",
            )
            .with_value(bacnet_npdu::MAX_MAC_LEN as u64),
        ));
    }
    Ok(u16::try_from(9 + dadr_len + local_source_mac_len)
        .expect("two addresses within bacnet_npdu::MAX_MAC_LEN fit a u16"))
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Configure the maximum NPDU for one immediate-router/DNET path.
    ///
    /// The change applies at once and sends nothing: it waits neither for a
    /// request on the path nor out a hold (#2683). After a request with no
    /// answer the path is held for the APDU timeout times the attempts
    /// (#2651); a request made in the hold, or already waiting it out, goes
    /// out when the hold ends, sized to the new limit. The hold is neither
    /// shortened nor restarted, and a reason-4 reject that lands in it is
    /// still discarded. A request sized before the change keeps its size,
    /// and a reject for it teaches nothing about the path. The change
    /// replaces the previous configured value and deliberately clears
    /// learned reason-4 evidence for that path.
    ///
    /// A router MAC outside 1 to [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN)
    /// octets or a DNET outside 1..=65534 is refused with
    /// [`Error::InvalidArgument`], and so is a `max_npdu` that can carry no
    /// request: one under the 50-octet minimum APDU plus the smallest NPCI a
    /// router adds to a request from this client, 10 octets and the
    /// client's own MAC (66 on B/IP, 61 on MS/TP). A limit under 228 octets
    /// is otherwise taken, for a router that forwards less than every router
    /// must. A station whose longer MAC leaves a request less than the
    /// minimum still fails that request, with [`Error::RoutedPathTooLong`].
    /// A path new to a full table of 256 paths, none of them idle, is
    /// refused with [`Error::RoutedPathCapacityExceeded`].
    pub fn configure_routed_path_max_npdu(
        &self,
        router_mac: &[u8],
        dnet: u16,
        max_npdu: u16,
    ) -> Result<(), Error> {
        check_remote_dnet(dnet)?;
        let smallest = smallest_usable_max_npdu(self.local_mac.len());
        if max_npdu < smallest {
            return Err(Error::InvalidArgument(Refusal::text(format!(
                "a routed path limit of {max_npdu} NPDU octets carries no request: after the \
                 {} octets of NPCI a router adds at the least, the APDU would be under the \
                 {}-octet minimum; give at least {smallest}",
                smallest - bacnet_encoding::apdu::MINIMUM_MESSAGE_SIZE,
                bacnet_encoding::apdu::MINIMUM_MESSAGE_SIZE,
            ))));
        }
        self.routed_path_limits
            .configure(router_mac, dnet, max_npdu)
    }

    /// Restore one immediate-router/DNET path to the conservative default.
    ///
    /// It applies at once, as a configure does, and clears both configured
    /// and learned reason-4 evidence; a path the client holds nothing for
    /// is left alone, with no entry reserved for it, so a clear never fails
    /// for capacity. Later requests therefore use the 228-octet
    /// conservative routed NPDU envelope until new evidence is supplied.
    /// The router MAC and DNET are checked as in
    /// [`configure_routed_path_max_npdu`](Self::configure_routed_path_max_npdu).
    pub fn clear_routed_path_limit(&self, router_mac: &[u8], dnet: u16) -> Result<(), Error> {
        check_remote_dnet(dnet)?;
        self.routed_path_limits.clear(router_mac, dnet)
    }
}

/// The smallest path limit that leaves a request the minimum APDU
/// ([`MINIMUM_MESSAGE_SIZE`](bacnet_encoding::apdu::MINIMUM_MESSAGE_SIZE))
/// once a router adds its NPCI ([`forwarded_npci_len`]) for the shortest
/// DADR, one octet, and the client's own MAC of `local_mac_len` octets as
/// SADR. Below it every request through the path would fail.
pub(super) fn smallest_usable_max_npdu(local_mac_len: usize) -> u16 {
    let npci = 10u16.saturating_add(u16::try_from(local_mac_len).unwrap_or(u16::MAX));
    bacnet_encoding::apdu::MINIMUM_MESSAGE_SIZE.saturating_add(npci)
}

/// A router MAC that [`routed_mac_fits`], or [`Error::InvalidArgument`].
fn check_router_mac(router_mac: &[u8]) -> Result<(), Error> {
    if !routed_mac_fits(router_mac.len()) {
        return Err(Error::InvalidArgument(
            Refusal::new("immediate router MAC address must contain 1..={} octets")
                .with_value(bacnet_npdu::MAX_MAC_LEN as u64),
        ));
    }
    Ok(())
}

/// Whether `dnet` names one remote network: 1..=65534. DNET 0 names no
/// network and 65535 is the global broadcast, so neither can key a routed path
/// or address a confirmed request (Clauses 6.2.2.1 and 6.3, #1278). The path
/// APIs and every routed confirmed request share this check;
/// [`check_router_mac`] checks the router MAC, from
/// [`RoutedPathLimits::lock_path`], [`RoutedPathLimits::configure`] and
/// [`RoutedPathLimits::clear`].
pub(super) fn check_remote_dnet(dnet: u16) -> Result<(), Error> {
    if dnet == 0 || dnet == u16::MAX {
        return Err(Error::invalid_argument("routed DNET must be in 1..=65534"));
    }
    Ok(())
}

/// A DADR that names one station: 1 to [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) octets.
/// With no DADR the router on DNET broadcasts the NPDU, and a broadcast
/// network address carries only unconfirmed requests (Clause 6.3).
fn check_routed_dadr(dadr_len: usize) -> Result<(), Error> {
    if !routed_mac_fits(dadr_len) {
        return Err(Error::InvalidArgument(
            Refusal::new("routed destination MAC address must contain 1..={} octets")
                .with_value(bacnet_npdu::MAX_MAC_LEN as u64),
        ));
    }
    Ok(())
}

/// Refuse a routed confirmed destination that does not name exactly one
/// device: a DNET [`check_remote_dnet`] refuses or a DADR outside
/// [`routed_mac_fits`]. The client's own request path runs it on the target
/// as named, before a target on the local network is sent locally, and
/// measures the forwarded NPCI with [`forwarded_npci_len`] only for a target
/// still routed.
pub(crate) fn check_routed_unicast(dnet: u16, dadr_len: usize) -> Result<(), Error> {
    check_remote_dnet(dnet)?;
    check_routed_dadr(dadr_len)
}

#[path = "routed_path_rejects.rs"]
mod rejects;

#[cfg(test)]
#[path = "routed_path_limits_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "routed_path_gate_tests.rs"]
mod gate_tests;
