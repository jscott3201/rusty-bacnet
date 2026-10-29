//! Node-switch identity ownership across accepted and initiated direct peers.
//!
//! Reservations protect both identities while Accept is in flight. No network
//! I/O runs under this lock. Only commit retires an incumbent; dropping an
//! uncommitted reservation is rollback. Generations never repeat in this process.
use super::direct_egress::{DirectEgress, DirectWrite};
use crate::sc_frame::Vmac;
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::watch;

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);
type Uuid = [u8; 16];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectRole {
    // Accepted connections exist only with the sc-tls direct listener.
    #[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
    Accepted,
    Outbound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    DuplicateVmac,
    Resources,
    Busy,
}

struct Entry {
    generation: u64,
    uuid: Uuid,
    vmac: Vmac,
    role: DirectRole,
    retired: watch::Sender<bool>,
    route: Option<DirectEgress>,
}

#[derive(Default)]
struct State {
    sealed: bool,
    established: HashMap<u64, Entry>,
    reserved: HashMap<u64, Entry>,
}

#[derive(Default)]
pub(crate) struct DirectMembership {
    state: Mutex<State>,
}

impl DirectMembership {
    pub(crate) fn reserve(
        self: &Arc<Self>,
        uuid: Uuid,
        vmac: Vmac,
        local_uuid: Uuid,
        local_vmac: Vmac,
        role: DirectRole,
        cap: usize,
    ) -> Result<Reservation, Refusal> {
        let mut state = self.state.lock().unwrap();
        if state.sealed {
            return Err(Refusal::Resources);
        }
        if uuid == local_uuid
            || vmac == local_vmac
            || state
                .established
                .values()
                .chain(state.reserved.values())
                .any(|e| e.vmac == vmac && e.uuid != uuid)
        {
            return Err(Refusal::DuplicateVmac);
        }
        // A competing attempt for either identity waits by retrying a later
        // connection; bounded admission never queues unlimited reservations.
        if state
            .reserved
            .values()
            .any(|e| e.uuid == uuid || e.vmac == vmac)
        {
            return Err(Refusal::Busy);
        }
        // Count the union by UUID, including reservations replacing an entry
        // which can disappear before commit. Replacing the other role consumes
        // a new slot; that role's quota cannot subsidize this one.
        let mut peers = std::collections::HashSet::new();
        for e in state.established.values().chain(state.reserved.values()) {
            if e.role == role {
                peers.insert(e.uuid);
            }
        }
        peers.insert(uuid);
        if peers.len() > cap {
            return Err(Refusal::Resources);
        }
        let generation = NEXT_GENERATION
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .map_err(|_| Refusal::Resources)?;
        let (retired, _) = watch::channel(false);
        state.reserved.insert(
            generation,
            Entry {
                generation,
                uuid,
                vmac,
                role,
                retired,
                route: None,
            },
        );
        Ok(Reservation {
            owner: Arc::clone(self),
            generation,
        })
    }

    pub(crate) fn route(&self, vmac: &Vmac) -> Option<DirectEgress> {
        self.state
            .lock()
            .unwrap()
            .established
            .values()
            .find(|entry| &entry.vmac == vmac)
            .and_then(|entry| entry.route.clone())
    }
    pub(crate) fn retire_all(&self) {
        let mut state = self.state.lock().unwrap();
        state.sealed = true;
        for (_, entry) in state.established.drain() {
            entry.retired.send_replace(true);
        }
    }

    #[cfg(test)]
    pub(crate) fn current_generations(&self) -> Vec<u64> {
        self.state
            .lock()
            .unwrap()
            .established
            .keys()
            .copied()
            .collect()
    }

    #[cfg(all(test, feature = "sc-tls"))]
    pub(crate) fn counts(&self) -> (usize, usize) {
        let state = self.state.lock().unwrap();
        (
            state
                .established
                .values()
                .filter(|e| e.role == DirectRole::Accepted)
                .count(),
            state.reserved.len(),
        )
    }
}

pub(crate) struct Reservation {
    owner: Arc<DirectMembership>,
    generation: u64,
}

impl Reservation {
    #[cfg(test)]
    pub(crate) fn commit(self) -> Arc<Membership> {
        self.commit_with_limits((1476, 1476), Duration::from_secs(1))
    }
    pub(crate) fn commit_with_limits(self, limits: (u16, u16), wait: Duration) -> Arc<Membership> {
        let mut state = self.owner.state.lock().unwrap();
        let mut entry = state
            .reserved
            .remove(&self.generation)
            .expect("owned reservation");
        let previous = state
            .established
            .values()
            .find(|e| e.uuid == entry.uuid)
            .map(|e| e.generation);
        if let Some(previous) = previous {
            let old = state.established.remove(&previous).unwrap();
            old.retired.send_replace(true);
        }
        let member = Arc::new_cyclic(|weak| {
            let (egress, writes) = DirectEgress::new(weak.clone(), limits, wait);
            Membership {
                owner: Arc::clone(&self.owner),
                generation: self.generation,
                vmac: entry.vmac,
                retired: entry.retired.clone(),
                egress,
                writes: Mutex::new(Some(writes)),
            }
        });
        entry.route = Some(member.egress.clone());
        if state.sealed {
            member.retired.send_replace(true);
        } else {
            state.established.insert(self.generation, entry);
        }
        drop(state);
        member
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.owner
            .state
            .lock()
            .unwrap()
            .reserved
            .remove(&self.generation);
    }
}

pub(crate) struct Membership {
    owner: Arc<DirectMembership>,
    pub(crate) generation: u64,
    // Read on the sc-tls direct intake and response paths.
    #[cfg_attr(not(feature = "sc-tls"), allow(dead_code))]
    pub(crate) vmac: Vmac,
    pub(crate) egress: DirectEgress,
    writes: Mutex<Option<tokio::sync::mpsc::Receiver<DirectWrite>>>,
    retired: watch::Sender<bool>,
}

impl Membership {
    pub(crate) fn take_writes(&self) -> tokio::sync::mpsc::Receiver<DirectWrite> {
        self.writes
            .lock()
            .unwrap()
            .take()
            .expect("one socket writer")
    }
    pub(crate) fn retirement(&self) -> watch::Receiver<bool> {
        self.retired.subscribe()
    }
    pub(crate) fn is_current(&self) -> bool {
        self.with_current(|| ()).is_some()
    }
    // Admission and replacement have one linearization point. The callback
    // must be synchronous, bounded and must not re-enter this registry.
    pub(crate) fn with_current<T>(&self, f: impl FnOnce() -> T) -> Option<T> {
        let state = self.owner.state.lock().unwrap();
        state.established.contains_key(&self.generation).then(f)
    }
    pub(crate) fn retire(&self) {
        self.owner
            .state
            .lock()
            .unwrap()
            .established
            .remove(&self.generation);
        self.retired.send_replace(true);
    }
}
impl Drop for Membership {
    fn drop(&mut self) {
        self.retire();
    }
}

pub(crate) fn disconnect_request() -> crate::sc_frame::ScMessage {
    crate::sc_frame::ScMessage {
        function: crate::sc_frame::ScFunction::DisconnectRequest,
        message_id: 0,
        originating_vmac: None,
        destination_vmac: None,
        dest_options: Vec::new(),
        data_options: Vec::new(),
        payload: bytes::Bytes::new(),
    }
}

/// Encodes a bare connection-control BVLC: no VMACs, Data Options or payload,
/// the shape AB.2.12 to AB.2.15 give Disconnect and Heartbeat messages.
pub(crate) fn bare_control(
    function: crate::sc_frame::ScFunction,
    message_id: u16,
) -> bytes::BytesMut {
    let mut bytes = bytes::BytesMut::new();
    crate::sc_frame::encode_sc_message(
        &mut bytes,
        &crate::sc_frame::ScMessage {
            function,
            message_id,
            ..disconnect_request()
        },
    );
    bytes
}
