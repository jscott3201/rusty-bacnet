use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, MutexGuard as StdMutexGuard};
use std::time::Instant;

use bacnet_encoding::npdu::decode_reject_message_to_network;
use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use super::*;
use crate::tsm::CompletionOutcome;

/// Smallest routed NPDU envelope required across the standard data links.
const CONSERVATIVE_ROUTED_PATH_MAX_NPDU: u16 = 228;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    observed_at: Instant,
}

struct LearnedPathLimit {
    exclusive_max_npdu: u16,
    observed_at: Instant,
}

struct ActiveRoutedSend {
    path: RoutedPathKey,
    tsm_mac: MacAddr,
    invoke_id: u8,
    owner: TransactionOwner,
    forwarded_npci_len: u16,
    outgoing_apdu_len: Option<u16>,
}

struct RoutedPathEntry {
    gate: Arc<AsyncMutex<()>>,
    configured: Option<ConfiguredPathLimit>,
    learned: Option<LearnedPathLimit>,
    active: Option<ActiveRoutedSend>,
}

impl Default for RoutedPathEntry {
    fn default() -> Self {
        Self {
            gate: Arc::new(AsyncMutex::new(())),
            configured: None,
            learned: None,
            active: None,
        }
    }
}

#[derive(Default)]
pub(super) struct RoutedPathLimits {
    entries: StdMutex<HashMap<RoutedPathKey, RoutedPathEntry>>,
}

impl RoutedPathLimits {
    fn entries(&self) -> StdMutexGuard<'_, HashMap<RoutedPathKey, RoutedPathEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) async fn acquire(self: &Arc<Self>, router_mac: &[u8], dnet: u16) -> RoutedPathLease {
        let key = RoutedPathKey::new(router_mac, dnet);
        let gate = {
            let mut entries = self.entries();
            Arc::clone(&entries.entry(key.clone()).or_default().gate)
        };
        let gate = gate.lock_owned().await;
        RoutedPathLease {
            limits: Arc::clone(self),
            key,
            _gate: gate,
        }
    }

    pub(super) fn install_active(
        &self,
        target: ConfirmedTarget<'_>,
        tsm_mac: MacAddr,
        invoke_id: u8,
        owner: TransactionOwner,
        forwarded_npci_len: u16,
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
        let mut entries = self.entries();
        let entry = entries.entry(key.clone()).or_default();
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
        let mut entries = self.entries();
        let Some(active) = entries
            .get_mut(&key)
            .and_then(|entry| entry.active.as_mut())
        else {
            return false;
        };
        if active.tsm_mac != *tsm_mac
            || active.invoke_id != invoke_id
            || !active.owner.same_as(owner)
        {
            return false;
        }
        active.outgoing_apdu_len = Some(outgoing_apdu_len);
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

        let Some(active) = self.claim_active(&control.source_mac, reject.dnet) else {
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

    fn claim_active(&self, router_mac: &[u8], dnet: u16) -> Option<ActiveRoutedSend> {
        let key = RoutedPathKey::new(router_mac, dnet);
        let mut entries = self.entries();
        let entry = entries.get_mut(&key)?;
        if entry
            .active
            .as_ref()
            .is_none_or(|active| active.outgoing_apdu_len.is_none() || active.path != key)
        {
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
        let mut entries = self.entries();
        let entry = entries.entry(active.path).or_default();
        let exclusive_max_npdu = entry.learned.as_ref().map_or(attempted_npdu, |learned| {
            learned.exclusive_max_npdu.min(attempted_npdu)
        });
        entry.learned = Some(LearnedPathLimit {
            exclusive_max_npdu,
            observed_at: Instant::now(),
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
        let entries = self.limits.entries();
        let entry = entries
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

    fn configure(&self, max_npdu: u16) {
        let mut entries = self.limits.entries();
        let entry = entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.configured = Some(ConfiguredPathLimit {
            max_npdu,
            provenance: ConfiguredLimitProvenance::ExplicitApi,
            observed_at: Instant::now(),
        });
        entry.learned = None;
    }

    fn clear(&self) {
        let mut entries = self.limits.entries();
        let entry = entries
            .get_mut(&self.key)
            .expect("path entry remains present while its gate is held");
        entry.configured = None;
        entry.learned = None;
    }
}

impl Drop for RoutedPathLease {
    fn drop(&mut self) {
        let mut entries = self.limits.entries();
        if let Some(entry) = entries.get_mut(&self.key) {
            entry.active = None;
        }
    }
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
        let lease = self.routed_path_limits.acquire(router_mac, dnet).await;
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
        let lease = self.routed_path_limits.acquire(router_mac, dnet).await;
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
mod tests {
    use super::*;
    use bacnet_types::enums::ConfirmedServiceChoice;

    #[test]
    fn conservative_and_configured_npdu_envelopes_use_forwarded_source() {
        let header = forwarded_npci_len(6, 6).unwrap();
        assert_eq!(header, 21);
        assert_eq!(CONSERVATIVE_ROUTED_PATH_MAX_NPDU - header, 207);
        assert_eq!(1497 - header, 1476);

        assert_eq!(forwarded_npci_len(1, 1).unwrap(), 11);
        assert_eq!(forwarded_npci_len(4, 6).unwrap(), 19);
    }

    #[test]
    fn forwarded_npci_rejects_unrepresentable_addresses() {
        assert!(forwarded_npci_len(256, 6).is_err());
        assert!(forwarded_npci_len(6, 0).is_err());
        assert!(forwarded_npci_len(6, 256).is_err());
    }

    #[test]
    fn claimed_control_cannot_complete_reused_invoke_id_for_new_owner() {
        let limits = RoutedPathLimits::default();
        let router = [2];
        let dadr = [3];
        let target = ConfirmedTarget::Routed {
            router_mac: &router,
            dest_network: 100,
            dest_mac: &dadr,
        };
        let tsm_mac = target.transaction_peer().tsm_mac;
        let mut tsm = Tsm::new(TsmConfig::default());
        let old = tsm.register_transaction_with_progress(
            tsm_mac.clone(),
            7,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        limits.install_active(target, tsm_mac.clone(), 7, old.owner.clone(), 11);
        assert!(limits.authorize_attempt(target, &tsm_mac, 7, &old.owner, 100));
        let claimed = limits.claim_active(&router, 100).unwrap();

        assert!(tsm.cancel_transaction_for_owner(&tsm_mac, 7, &old.owner));
        let replacement = tsm.register_transaction_with_progress(
            tsm_mac.clone(),
            7,
            ConfirmedServiceChoice::READ_PROPERTY,
        );
        assert_eq!(
            tsm.complete_network_path_too_long_for_owner(
                &claimed.tsm_mac,
                claimed.invoke_id,
                &claimed.owner,
                100,
            ),
            CompletionOutcome::NoTransaction
        );
        assert!(tsm.owner_is_current(&tsm_mac, 7, &replacement.owner));
        assert_eq!(tsm.pending_count(), 1);
    }
}
