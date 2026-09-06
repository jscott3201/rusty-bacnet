use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::{Duration, Instant as StdInstant};

use bacnet_encoding::npdu::decode_reject_message_to_network;
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio::time::Instant as TokioInstant;

use super::*;
use crate::tsm::CompletionOutcome;

/// Smallest routed NPDU envelope required across the standard data links.
const CONSERVATIVE_ROUTED_PATH_MAX_NPDU: u16 = 228;
/// Hard bound covering gates, configured evidence, and learned evidence.
pub(super) const MAX_ROUTED_PATH_ENTRIES: usize = 256;

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
    outgoing_apdu_len: Option<u16>,
    ingress_floor: u64,
}

struct RoutedPathEntry {
    gate: Arc<AsyncMutex<()>>,
    configured: Option<ConfiguredPathLimit>,
    learned: Option<LearnedPathLimit>,
    active: Option<ActiveRoutedSend>,
    last_used: u64,
    quarantine_started: Option<TokioInstant>,
    generation_attempts: u32,
    generation_terminal_observed: bool,
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
            generation_attempts: 0,
            generation_terminal_observed: false,
        }
    }
}

struct RoutedPathState {
    entries: HashMap<RoutedPathKey, RoutedPathEntry>,
    next_use: u64,
    capacity: usize,
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
            }),
            quarantine_horizon,
        }
    }

    fn state(&self) -> StdMutexGuard<'_, RoutedPathState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) async fn acquire(
        self: &Arc<Self>,
        router_mac: &[u8],
        dnet: u16,
    ) -> Result<RoutedPathLease, Error> {
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
            let reclaim = state
                .entries
                .iter()
                .filter(|(_, entry)| self.is_safely_reclaimable(entry))
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

    fn is_safely_reclaimable(&self, entry: &RoutedPathEntry) -> bool {
        entry.configured.is_none()
            && entry.learned.is_none()
            && entry.active.is_none()
            && Arc::strong_count(&entry.gate) == 1
            && entry
                .quarantine_started
                .is_none_or(|started| started.elapsed() >= self.quarantine_horizon)
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
                            entry.generation_attempts = 0;
                            entry.generation_terminal_observed = false;
                            None
                        } else {
                            Some(remaining)
                        }
                    }
                    None => {
                        entry.generation_attempts = 0;
                        entry.generation_terminal_observed = false;
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
            outgoing_apdu_len: None,
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
        let Some(active) = entry.active.as_mut() else {
            return false;
        };
        if active.tsm_mac != *tsm_mac
            || active.invoke_id != invoke_id
            || !active.owner.same_as(owner)
        {
            return false;
        }
        active.outgoing_apdu_len = Some(outgoing_apdu_len);
        entry.generation_attempts = entry.generation_attempts.saturating_add(1);
        true
    }

    pub(super) async fn handle_network_control(
        &self,
        tsm: &Arc<Mutex<Tsm>>,
        control: ReceivedNetworkControl,
    ) {
        if control.npdu.message_type != Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
        {
            return;
        }
        let Ok(reject) = decode_reject_message_to_network(&control.npdu.payload) else {
            return;
        };
        if reject.reason != RejectMessageReason::MESSAGE_TOO_LONG {
            return;
        }

        let Some(active) =
            self.claim_active(&control.source_mac, reject.dnet, control.ingress_sequence)
        else {
            return;
        };
        let outcome = tsm.lock().await.complete_network_path_too_long_for_owner(
            &active.tsm_mac,
            active.invoke_id,
            &active.owner,
            reject.dnet,
        );
        if outcome == CompletionOutcome::Delivered {
            self.commit_learned_limit(active);
        }
    }

    fn claim_active(
        &self,
        router_mac: &[u8],
        dnet: u16,
        ingress_sequence: u64,
    ) -> Option<ActiveRoutedSend> {
        let key = RoutedPathKey::new(router_mac, dnet);
        let mut state = self.state();
        let entry = state.entries.get_mut(&key)?;
        if entry.active.as_ref().is_none_or(|active| {
            active.outgoing_apdu_len.is_none()
                || active.path != key
                || ingress_sequence <= active.ingress_floor
        }) {
            return None;
        }
        entry.active.take()
    }

    fn commit_learned_limit(&self, active: ActiveRoutedSend) {
        let Some(attempted_npdu) = active
            .outgoing_apdu_len
            .and_then(|apdu| active.forwarded_npci_len.checked_add(apdu))
        else {
            return;
        };
        let mut state = self.state();
        let entry = state
            .entries
            .get_mut(&active.path)
            .expect("the reason-4 claimant still holds its routed path gate");
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
    pub(super) fn max_apdu(
        &self,
        dadr_len: usize,
        local_source_mac_len: usize,
    ) -> Result<u16, Error> {
        let forwarded_npci_len = forwarded_npci_len(dadr_len, local_source_mac_len)?;
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
                configured_or_conservative
                    .min(learned.exclusive_max_npdu.checked_sub(1).unwrap_or(0))
            });
        Ok(effective_npdu.checked_sub(forwarded_npci_len).unwrap_or(0))
    }

    pub(super) fn forwarded_npci_len(
        &self,
        dadr_len: usize,
        local_source_mac_len: usize,
    ) -> Result<u16, Error> {
        forwarded_npci_len(dadr_len, local_source_mac_len)
    }

    /// Mark a source-correlated terminal response for the current generation.
    /// One attempted frame plus its terminal response proves there cannot be a
    /// second reason-4 still in flight for that generation. Multi-frame or
    /// retried generations remain ambiguous and are quarantined on drop.
    pub(super) fn mark_terminal_observed(&self) {
        let mut state = self.limits.state();
        let entry = state
            .entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.generation_terminal_observed = entry.generation_attempts == 1;
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
    }

    fn clear(&self) {
        let mut state = self.limits.state();
        let entry = state
            .entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.configured = None;
        entry.learned = None;
    }
}

impl Drop for RoutedPathLease {
    fn drop(&mut self) {
        let mut state = self.limits.state();
        if let Some(entry) = state.entries.get_mut(&self.key) {
            entry.active = None;
            if entry.generation_attempts > 0 && !entry.generation_terminal_observed {
                entry.quarantine_started = Some(TokioInstant::now());
            }
            entry.generation_attempts = 0;
            entry.generation_terminal_observed = false;
        }
    }
}

fn bump_use(state: &mut RoutedPathState) -> u64 {
    state.next_use = state.next_use.saturating_add(1);
    state.next_use
}

pub(super) fn routed_path_quarantine_horizon(config: &ClientConfig) -> Duration {
    // Reuse the configured complete request/retry horizon instead of adding a
    // separate arbitrary stale-control timer. A zero timeout still receives
    // one millisecond, the existing configuration's millisecond granularity,
    // so ambiguous generations never skip quarantine entirely.
    let attempts = u64::from(config.apdu_retries).saturating_add(1);
    Duration::from_millis(config.apdu_timeout_ms.saturating_mul(attempts).max(1))
}

fn forwarded_npci_len(dadr_len: usize, local_source_mac_len: usize) -> Result<u16, Error> {
    if dadr_len > u8::MAX as usize {
        return Err(Error::Encoding(
            "routed destination MAC address exceeds 255 bytes".into(),
        ));
    }
    if local_source_mac_len == 0 || local_source_mac_len > u8::MAX as usize {
        return Err(Error::Encoding(
            "local source MAC address for routed forwarding must contain 1..=255 bytes".into(),
        ));
    }
    u16::try_from(9usize + dadr_len + local_source_mac_len)
        .map_err(|_| Error::Encoding("forwarded routed NPCI length exceeds u16".into()))
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// Configure the maximum NPDU for one immediate-router/DNET path.
    ///
    /// The update waits for any request already active on the same path. It
    /// replaces the previous configured value and deliberately clears learned
    /// reason-4 evidence for that path.
    pub async fn configure_routed_path_max_npdu(
        &self,
        router_mac: &[u8],
        dnet: u16,
        max_npdu: u16,
    ) -> Result<(), Error> {
        validate_public_path(router_mac, dnet)?;
        let lease = self.routed_path_limits.acquire(router_mac, dnet).await?;
        lease.configure(max_npdu);
        Ok(())
    }

    /// Restore one immediate-router/DNET path to the conservative default.
    ///
    /// The update waits for an active request and clears both configured and
    /// learned reason-4 evidence. Later requests therefore use the 228-octet
    /// conservative routed NPDU envelope until new evidence is supplied.
    pub async fn clear_routed_path_limit(&self, router_mac: &[u8], dnet: u16) -> Result<(), Error> {
        validate_public_path(router_mac, dnet)?;
        let lease = self.routed_path_limits.acquire(router_mac, dnet).await?;
        lease.clear();
        Ok(())
    }
}

fn validate_public_path(router_mac: &[u8], dnet: u16) -> Result<(), Error> {
    if router_mac.is_empty() || router_mac.len() > u8::MAX as usize {
        return Err(Error::Encoding(
            "immediate router MAC address must contain 1..=255 bytes".into(),
        ));
    }
    if dnet == 0 || dnet == 0xffff {
        return Err(Error::Encoding(
            "routed path DNET must be in 1..=65534".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "routed_path_limits_tests.rs"]
mod tests;
