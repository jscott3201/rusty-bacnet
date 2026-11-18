use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::{Duration, Instant as StdInstant};

use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_npdu::{NetworkMessageType, RejectMessageReason, RejectMessageToNetwork};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio::time::Instant as TokioInstant;

use super::*;
use crate::tsm::CompletionOutcome;

/// The smallest NPDU a router must be able to forward between any two data
/// links, the lowest of the per-link figures in Table 6-1 (Clause 6.6). It is
/// the default path limit, and also the floor of what a path can learn: a
/// conforming router never refuses a frame this small as too long, so a
/// reason 4 claimed by a request whose largest frame is no larger is not
/// about that frame ([`RoutedPathLimits::claim_active`]).
const CONSERVATIVE_ROUTED_PATH_MAX_NPDU: u16 = 228;
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
    gate: Arc<AsyncMutex<()>>,
    configured: Option<ConfiguredPathLimit>,
    learned: Option<LearnedPathLimit>,
    active: Option<ActiveRoutedSend>,
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
}

impl RoutedPathEntry {
    fn new(last_used: u64) -> Self {
        Self {
            gate: Arc::new(AsyncMutex::new(())),
            configured: None,
            learned: None,
            active: None,
            last_used,
            quarantine_started: None,
            answered: None,
            generation_attempts: 0,
            generation_largest_npdu: 0,
            generation_end: GenerationEnd::Open,
            below_minimum_warned: false,
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
}

pub(super) struct RoutedPathLimits {
    state: StdMutex<RoutedPathState>,
    quarantine_horizon: Duration,
}

impl RoutedPathLimits {
    pub(super) fn new(quarantine_horizon: Duration) -> Self {
        Self::with_capacity(MAX_ROUTED_PATH_ENTRIES, quarantine_horizon)
    }

    fn with_capacity(capacity: usize, quarantine_horizon: Duration) -> Self {
        assert!(capacity > 0, "routed path capacity must be nonzero");
        Self {
            state: StdMutex::new(RoutedPathState {
                entries: HashMap::new(),
                next_use: 0,
                capacity,
                dropped_rejects: 0,
                below_minimum_drops: 0,
            }),
            quarantine_horizon,
        }
    }

    fn state(&self) -> StdMutexGuard<'_, RoutedPathState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// See [`BACnetClient::too_long_reject_drops`].
    pub(super) fn dropped_rejects(&self) -> u64 {
        self.state().dropped_rejects
    }

    /// See [`BACnetClient::below_minimum_reject_drops`].
    pub(super) fn below_minimum_drops(&self) -> u64 {
        self.state().below_minimum_drops
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

    /// Reserve and lock the path through `router_mac` to `dnet`. A router MAC
    /// outside [`routed_mac_fits`] is refused before any entry is reserved.
    pub(super) async fn acquire(
        self: &Arc<Self>,
        router_mac: &[u8],
        dnet: u16,
    ) -> Result<RoutedPathLease, Error> {
        if !routed_mac_fits(router_mac.len()) {
            return Err(Error::Encoding(format!(
                "immediate router MAC address must contain 1..={} octets",
                bacnet_npdu::MAX_MAC_LEN
            )));
        }
        let key = RoutedPathKey::new(router_mac, dnet);
        let gate = self.reserve_gate(&key)?;
        let gate = gate.lock_owned().await;
        self.wait_for_quarantine(&key).await;
        Ok(RoutedPathLease {
            limits: Arc::clone(self),
            key,
            _gate: gate,
        })
    }

    fn reserve_gate(&self, key: &RoutedPathKey) -> Result<Arc<AsyncMutex<()>>, Error> {
        let mut state = self.state();
        let last_used = bump_use(&mut state);
        if let Some(entry) = state.entries.get_mut(key) {
            entry.last_used = last_used;
            return Ok(Arc::clone(&entry.gate));
        }

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

        let entry = RoutedPathEntry::new(last_used);
        let gate = Arc::clone(&entry.gate);
        state.entries.insert(key.clone(), entry);
        Ok(gate)
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

    pub(super) fn install_active(
        &self,
        target: ConfirmedTarget<'_>,
        tsm_mac: MacAddr,
        invoke_id: u8,
        owner: TransactionOwner,
        forwarded_npci_len: u16,
        ingress_floor: u64,
    ) {
        let ConfirmedTarget::Routed {
            router_mac,
            dest_network,
            ..
        } = target
        else {
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
        let ConfirmedTarget::Routed {
            router_mac,
            dest_network,
            ..
        } = target
        else {
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

    pub(super) async fn handle_network_control(
        &self,
        tsm: &Arc<SyncMutex<Tsm>>,
        control: ReceivedNetworkControl,
    ) {
        if control.npdu.message_type != Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK) {
            return;
        }
        let Ok(reject) = RejectMessageToNetwork::decode(&control.npdu.payload) else {
            return;
        };
        if reject.reason != RejectMessageReason::MESSAGE_TOO_LONG {
            return;
        }

        let Some((active, attempted_npdu)) = self.claim_active(
            &control.source_mac,
            reject.network,
            control.ingress_sequence,
        ) else {
            return;
        };
        let outcome = tsm.lock().complete_network_path_too_long_for_owner(
            &active.tsm_mac,
            active.invoke_id,
            &active.owner,
            reject.network,
        );
        if outcome == CompletionOutcome::Delivered {
            self.commit_learned_limit(active, attempted_npdu);
        }
    }

    /// Take the active request a reason 4 for `router_mac` and `dnet` ends,
    /// with the largest forwarded NPDU it has sent. A reject that reached
    /// ingress before the request was installed is not its own. One that
    /// could be for an earlier answered generation's larger frame is
    /// dropped and counted, and the request runs on to its own answer,
    /// retry or timeout. So is one claimed by a request whose largest frame is
    /// within [`CONSERVATIVE_ROUTED_PATH_MAX_NPDU`] (counted apart): no
    /// conforming router refuses such a frame, so the reject was for other
    /// traffic, and believing it would pin the path below any link's minimum.
    /// The first such drop on a path logs a warning.
    fn claim_active(
        &self,
        router_mac: &[u8],
        dnet: u16,
        ingress_sequence: u64,
    ) -> Option<(ActiveRoutedSend, u16)> {
        let key = RoutedPathKey::new(router_mac, dnet);
        let mut guard = self.state();
        let state = &mut *guard;
        let entry = state.entries.get_mut(&key)?;
        if entry.generation_attempts == 0
            || entry
                .active
                .as_ref()
                .is_none_or(|active| active.path != key || ingress_sequence <= active.ingress_floor)
        {
            return None;
        }
        let largest = entry.generation_largest_npdu;
        // This check stays before the arms below: they clear the answered
        // record and mark a one-frame generation refused, which a dropped
        // reject must not do.
        if largest <= CONSERVATIVE_ROUTED_PATH_MAX_NPDU {
            state.below_minimum_drops = state.below_minimum_drops.saturating_add(1);
            let first = !std::mem::replace(&mut entry.below_minimum_warned, true);
            drop(guard);
            if first {
                tracing::warn!(
                    router_mac = ?router_mac,
                    dnet,
                    npdu_octets = largest,
                    minimum_npdu_octets = CONSERVATIVE_ROUTED_PATH_MAX_NPDU,
                    "Ignored a reason-4 (too long) network reject for a frame within \
                     the 228-octet routed minimum of Table 6-1; a router refusing a frame that \
                     small is outside the standard. If this router really limits the path \
                     below that, set the limit by hand with configure_routed_path_max_npdu"
                );
            }
            return None;
        }
        // The clock is read only for a path with answered frames on record.
        let answered = match entry.answered {
            Some(_) => entry.answered_at(TokioInstant::now()),
            None => None,
        };
        match answered {
            Some(answered) if largest < answered.largest_npdu => {
                state.dropped_rejects = state.dropped_rejects.saturating_add(1);
                return None;
            }
            Some(_) => {}
            None => {
                entry.answered = None;
                if entry.generation_attempts == 1 {
                    entry.generation_end = GenerationEnd::Refused;
                }
            }
        }
        entry.active.take().map(|active| (active, largest))
    }

    fn commit_learned_limit(&self, active: ActiveRoutedSend, attempted_npdu: u16) {
        let mut state = self.state();
        let entry = state
            .entries
            .get_mut(&active.path)
            .expect("the reason-4 claimant still holds its routed path gate");
        // A learned limit stays at the conservative one or above, whatever
        // the caller passes (`claim_active` already drops smaller frames).
        let attempted_npdu = attempted_npdu.max(CONSERVATIVE_ROUTED_PATH_MAX_NPDU + 1);
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
    _gate: OwnedMutexGuard<()>,
}

impl RoutedPathLease {
    /// The APDU allowance left on this path after the forwarded NPCI that
    /// [`forwarded_npci_len`] measured.
    pub(super) fn max_apdu(&self, forwarded_npci_len: u16) -> u16 {
        let state = self.limits.state();
        let entry = state
            .entries
            .get(&self.key)
            .expect("path entry remains present while its gate is held");
        let configured_or_conservative =
            entry
                .configured
                .as_ref()
                .map_or(CONSERVATIVE_ROUTED_PATH_MAX_NPDU, |configured| {
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
    }

    fn configure(&self, max_npdu: u16) {
        let mut state = self.limits.state();
        let entry = state
            .entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.configured = Some(ConfiguredPathLimit {
            max_npdu,
            provenance: ConfiguredLimitProvenance::ExplicitApi,
            observed_at: StdInstant::now(),
        });
        entry.learned = None;
        entry.below_minimum_warned = false;
    }

    fn clear(&self) {
        let mut state = self.limits.state();
        let entry = state
            .entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.configured = None;
        entry.learned = None;
        entry.below_minimum_warned = false;
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
        return Err(Error::Encoding(format!(
            "local source MAC address for routed forwarding must contain 1..={} octets",
            bacnet_npdu::MAX_MAC_LEN
        )));
    }
    Ok(u16::try_from(9 + dadr_len + local_source_mac_len)
        .expect("two addresses within bacnet_npdu::MAX_MAC_LEN fit a u16"))
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Configure the maximum NPDU for one immediate-router/DNET path.
    ///
    /// The update waits for any request already active on the same path. It
    /// replaces the previous configured value and deliberately clears learned
    /// reason-4 evidence for that path. A router MAC outside 1 to
    /// [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) octets or a DNET outside 1..=65534 is
    /// refused with [`Error::Encoding`].
    pub async fn configure_routed_path_max_npdu(
        &self,
        router_mac: &[u8],
        dnet: u16,
        max_npdu: u16,
    ) -> Result<(), Error> {
        check_remote_dnet(dnet)?;
        let lease = self.routed_path_limits.acquire(router_mac, dnet).await?;
        lease.configure(max_npdu);
        Ok(())
    }

    /// Restore one immediate-router/DNET path to the conservative default.
    ///
    /// The update waits for an active request and clears both configured and
    /// learned reason-4 evidence. Later requests therefore use the 228-octet
    /// conservative routed NPDU envelope until new evidence is supplied. The
    /// router MAC and DNET are checked as in
    /// [`configure_routed_path_max_npdu`](Self::configure_routed_path_max_npdu).
    pub async fn clear_routed_path_limit(&self, router_mac: &[u8], dnet: u16) -> Result<(), Error> {
        check_remote_dnet(dnet)?;
        let lease = self.routed_path_limits.acquire(router_mac, dnet).await?;
        lease.clear();
        Ok(())
    }
}

/// Whether `dnet` names one remote network: 1..=65534. DNET 0 names no
/// network and 65535 is the global broadcast, so neither can key a routed path
/// or address a confirmed request (Clauses 6.2.2.1 and 6.3, #1278). The path
/// APIs and every routed confirmed request share this check;
/// [`RoutedPathLimits::acquire`] checks the router MAC.
pub(super) fn check_remote_dnet(dnet: u16) -> Result<(), Error> {
    if dnet == 0 || dnet == u16::MAX {
        return Err(Error::Encoding("routed DNET must be in 1..=65534".into()));
    }
    Ok(())
}

/// A DADR that names one station: 1 to [`MAX_MAC_LEN`](bacnet_npdu::MAX_MAC_LEN) octets.
/// With no DADR the router on DNET broadcasts the NPDU, and a broadcast
/// network address carries only unconfirmed requests (Clause 6.3).
fn check_routed_dadr(dadr_len: usize) -> Result<(), Error> {
    if !routed_mac_fits(dadr_len) {
        return Err(Error::Encoding(format!(
            "routed destination MAC address must contain 1..={} octets",
            bacnet_npdu::MAX_MAC_LEN
        )));
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

#[cfg(test)]
#[path = "routed_path_limits_tests.rs"]
mod tests;
