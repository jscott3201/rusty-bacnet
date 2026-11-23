//! A router's refusal of a routed confirmed request, the reject it sends
//! back (Clause 6.4.4): which reasons end the request, and how a reject,
//! which carries no invoke ID, is tied to one request (#2724).
//!
//! A reject goes to the station that sent the refused message: the router
//! delivers it to that station's MAC, with no DNET. One that came as a
//! broadcast (a global one with DNET 65535, or a broadcast on this link) is
//! for no one station and is dropped before its reason is read: any
//! station could send one, naming any network, to every client at once.
//!
//! A path's lease lets one confirmed request run at a time through one
//! router to one network, so a reject from that router naming that network
//! can be the active request's own. It is taken as its own only when:
//!
//! - it came from the router the request was sent to: the path is keyed by
//!   the reject's link source and its network, so a reject from another
//!   station on the link, or one naming another network, finds no request;
//! - the network layer decoded it after the request was installed on the
//!   path (its ingress sequence is assigned at that decode), so a reject
//!   decoded before the request went out is never its own;
//! - the request has sent a frame;
//! - no answered generation's frames may still draw a reject: a reason 4 is
//!   then judged by size, and any other reason is dropped, since nothing
//!   says which frame it was for.
//!
//! What a reason does with a reject taken that way:
//!
//! | Reason | The request |
//! | --- | --- |
//! | 0 other, 1 no route, 5 (security, before 135-2020), 6 addressing, 7 and up | ends at once with [`Error::NetworkRejected`] |
//! | 2 busy | runs on: its retries go at the normal interval, which honours the router's flow control; if they run out, [`Error::RouterBusyTimeout`] names the router |
//! | 3 unknown message type | runs on: the reason answers a network message, never an APDU |
//! | 4 too long | as before ([`Error::RoutedPathTooLong`], learned limits, #2651, #2665) |
//!
//! A request ended this way after one frame frees its path at once, and
//! the device never received it; after several (a retry or segments) the
//! path is held for the horizon, as for a reason 4, since an earlier frame
//! may still draw a reject that the next request couldn't tell from its
//! own, and an earlier attempt may have reached the device.
//!
//! What stays possible: any station that can send through the router can
//! address a reject to this station (DNET this client's network, DADR its
//! MAC, which the router delivers as its own unicast), and a station on
//! this link can send one from the router's link address. Naming the
//! network of a request in flight, such a reject ends that request early;
//! a forged Abort for its invoke ID does as much. It reaches one client at
//! a time, can't end a request to another network or through another
//! router, a local request, or one not yet sent, and changes no route or
//! limit. A request it ends after several frames holds its path for the
//! horizon, as a timeout would.
//!
//! A request by local broadcast ([`Station::routed_via_local_broadcast`])
//! takes no path, so no reject is tied to it: any router on the link may
//! hear it, and one router's refusal says nothing of another's.

use bacnet_network::layer::ReceivedNetworkControl;
use bacnet_npdu::{Mac, NetworkMessageType, RejectMessageReason, RejectMessageToNetwork};
use bacnet_types::data_link::DataLink;
use bacnet_types::enums::AbortReason;
use bacnet_types::error::NetworkReject;

use super::*;
use crate::tsm::CompletionOutcome;

/// What a reject's reason does to the request it is taken for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    /// Ends the request at once.
    Fail,
    /// Reason 4, judged by size.
    TooLong,
    /// Reason 2: the request keeps its retry timer, and the reject is kept
    /// to name the router if they run out.
    Busy,
    /// Reason 3: never a request's.
    Ignore,
}

impl Action {
    fn of(reason: RejectMessageReason) -> Self {
        match reason {
            RejectMessageReason::MESSAGE_TOO_LONG => Self::TooLong,
            RejectMessageReason::ROUTER_BUSY => Self::Busy,
            RejectMessageReason::UNKNOWN_MESSAGE_TYPE => Self::Ignore,
            // 0, 1, 5, 6 and any reason the standard doesn't define: the
            // router did not pass the request on, and a retry through it
            // is not expected to fare better.
            _ => Self::Fail,
        }
    }
}

/// A busy reject tied to the request `owner` ran on its path.
pub(super) struct BusyNote {
    owner: TransactionOwner,
    reject: NetworkReject,
}

/// A network control as this client acts on it: a Reject-Message-To-Network
/// addressed to this station alone.
struct Reject<'a> {
    control: &'a ReceivedNetworkControl,
    message: RejectMessageToNetwork,
    link: Option<DataLink>,
}

impl Reject<'_> {
    /// The reject as reported, from the path's router, for a request that
    /// sent `frames` frames.
    fn report(&self, router: &MacAddr, frames: u32) -> NetworkReject {
        // The path's key holds a MAC that fits, so this never falls back.
        let router = Mac::from_slice(router).unwrap_or_default();
        let mut reject =
            NetworkReject::new(router, self.link, self.message.network, self.message.reason);
        if let Some(source) = self.control.npdu.source.as_ref()
            && let Ok(mac) = Mac::from_slice(&source.mac_address)
        {
            reject = reject.relayed_from(source.network, mac);
        }
        if frames == 1 {
            reject = reject.after_one_frame();
        }
        reject
    }
}

impl RoutedPathLimits {
    /// Act on a network-layer control the client received: a router's
    /// reject (Reject-Message-To-Network) of a routed request in flight, by
    /// its reason. `link` is the client's data link, which names the router
    /// in the error.
    pub(in crate::client) async fn handle_network_control(
        &self,
        tsm: &Arc<SyncMutex<Tsm>>,
        control: ReceivedNetworkControl,
        link: Option<DataLink>,
    ) {
        if control.npdu.message_type != Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK) {
            return;
        }
        let Ok(message) = RejectMessageToNetwork::decode(&control.npdu.payload) else {
            return;
        };
        // Addressed to this station alone, or to no one request.
        if control.npdu.destination.is_some() || control.link_layer_group {
            tracing::debug!(
                router_mac = ?control.source_mac,
                dnet = message.network,
                "Ignored a network reject that came as a broadcast"
            );
            return;
        }
        let reject = Reject {
            control: &control,
            message,
            link,
        };
        match Action::of(message.reason) {
            Action::TooLong => self.too_long(tsm, &control, message.network),
            Action::Fail => self.refused(tsm, &reject),
            Action::Busy => self.busy(&reject),
            Action::Ignore => {}
        }
    }

    fn too_long(&self, tsm: &Arc<SyncMutex<Tsm>>, control: &ReceivedNetworkControl, dnet: u16) {
        let Some((active, attempted_npdu, changed)) =
            self.claim_active(&control.source_mac, dnet, control.ingress_sequence)
        else {
            return;
        };
        let outcome = tsm.lock().complete_network_refusal_for_owner(
            &active.tsm_mac,
            active.invoke_id,
            &active.owner,
            TsmResponse::Failed(Box::new(Error::RoutedPathTooLong { dnet })),
        );
        // A change of the path's limit may run at any point here (and a
        // clear may leave the entry to be reclaimed once the request drops
        // its lease); the commit then learns nothing.
        if outcome == CompletionOutcome::Delivered {
            self.commit_learned_limit(active, attempted_npdu, changed);
        }
    }

    /// End the active request on the path through the reject's source to
    /// its network with the reject, when [`Self::claim_refused`] ties it to
    /// one.
    fn refused(&self, tsm: &Arc<SyncMutex<Tsm>>, reject: &Reject<'_>) {
        let control = reject.control;
        let Some((active, frames)) = self.claim_refused(
            &control.source_mac,
            reject.message.network,
            control.ingress_sequence,
        ) else {
            return;
        };
        let refusal = reject.report(&active.path.immediate_router_mac, frames);
        tracing::debug!(%refusal, "Routed request ended by its router's reject");
        let _ = tsm.lock().complete_network_refusal_for_owner(
            &active.tsm_mac,
            active.invoke_id,
            &active.owner,
            TsmResponse::Failed(Box::new(Error::NetworkRejected(Box::new(refusal)))),
        );
    }

    /// Keep a busy reject the active request on its path can be tied to,
    /// by the rules [`Self::claim_refused`] keeps, to name the router if
    /// the request runs out of retries. The request goes on as it was.
    fn busy(&self, reject: &Reject<'_>) {
        let control = reject.control;
        let key = RoutedPathKey::new(&control.source_mac, reject.message.network);
        let mut state = self.state();
        let Some(entry) = claimable(&mut state, &key, control.ingress_sequence) else {
            return;
        };
        let Some(active) = entry
            .active
            .as_ref()
            .filter(|_| entry.answered_now().is_none())
        else {
            return;
        };
        let note = BusyNote {
            owner: active.owner.clone(),
            reject: reject.report(&key.immediate_router_mac, entry.generation_attempts),
        };
        entry.busy = Some(Box::new(note));
        drop(state);
        tracing::debug!(
            router_mac = ?control.source_mac,
            dnet = reject.message.network,
            "A router is busy for a routed request's network (reject reason 2); the \
             request keeps its retries"
        );
    }

    /// Take the active request a reason 4 for `router_mac` and `dnet` ends,
    /// with the largest forwarded NPDU it has sent and the
    /// [`changed`](RoutedPathEntry::changed) number it was sized under
    /// ([`RoutedPathEntry::sized_under`]). A reject the network layer
    /// decoded before the request was installed is not its own. One that
    /// could be for an earlier answered generation's larger frame is
    /// dropped and counted, and the request runs on to its own answer,
    /// retry or timeout. So is one claimed by a request whose largest frame is
    /// within [`DEFAULT_ROUTED_PATH_MAX_NPDU`] (counted apart): no
    /// conforming router refuses such a frame, so the reject was for other
    /// traffic, and believing it would pin the path below any link's minimum.
    /// The first such drop on a path logs a warning and marks the path,
    /// unless the request was sized before the path's latest limit change:
    /// the mark and the warning are about the limit in force.
    pub(super) fn claim_active(
        &self,
        router_mac: &[u8],
        dnet: u16,
        ingress_sequence: u64,
    ) -> Option<(ActiveRoutedSend, u16, u64)> {
        let key = RoutedPathKey::new(router_mac, dnet);
        let mut guard = self.state();
        let state = &mut *guard;
        let entry = claimable(state, &key, ingress_sequence)?;
        let largest = entry.generation_largest_npdu;
        // This check stays before the arms below: they clear the answered
        // record and mark a one-frame generation refused, which a dropped
        // reject must not do.
        if largest <= DEFAULT_ROUTED_PATH_MAX_NPDU {
            state.below_minimum_drops = state.below_minimum_drops.saturating_add(1);
            let entry = state
                .entries
                .get_mut(&key)
                .expect("claimable found the entry under this lock");
            let first = entry.sized_under == entry.changed
                && !std::mem::replace(&mut entry.below_minimum_warned, true);
            drop(guard);
            if first {
                tracing::warn!(
                    router_mac = ?router_mac,
                    dnet,
                    npdu_octets = largest,
                    minimum_npdu_octets = DEFAULT_ROUTED_PATH_MAX_NPDU,
                    "Ignored a reason-4 (too long) network reject for a frame within \
                     the 228-octet routed minimum of Table 6-1; a router refusing a frame that \
                     small is outside the standard. If this router really limits the path \
                     below that, requests through it time out until the path's limit is set \
                     by hand (BACnetClient::configure_routed_path_max_npdu)"
                );
            }
            return None;
        }
        let entry = state
            .entries
            .get_mut(&key)
            .expect("claimable found the entry under this lock");
        match entry.answered_now() {
            Some(answered) if largest < answered.largest_npdu => {
                state.dropped_rejects = state.dropped_rejects.saturating_add(1);
                return None;
            }
            Some(_) => {}
            None => entry.refuse_generation(),
        }
        let sized_under = entry.sized_under;
        entry
            .active
            .take()
            .map(|active| (active, largest, sized_under))
    }

    /// Take the active request a reject that ends one at once ([`Action::Fail`])
    /// from `router_mac` for `dnet` is for, under the rules [`claimable`]
    /// keeps. While an answered generation's frames may still draw a
    /// reject, none is taken: nothing says it isn't for one of them, and,
    /// unlike a size, a reason for one frame says nothing of another to
    /// the same network. The request then runs on to its own answer,
    /// retry or timeout.
    pub(super) fn claim_refused(
        &self,
        router_mac: &[u8],
        dnet: u16,
        ingress_sequence: u64,
    ) -> Option<(ActiveRoutedSend, u32)> {
        let key = RoutedPathKey::new(router_mac, dnet);
        let mut state = self.state();
        let entry = claimable(&mut state, &key, ingress_sequence)?;
        if entry.answered_now().is_some() {
            drop(state);
            tracing::debug!(
                router_mac = ?router_mac,
                dnet,
                "Kept a routed request going past a network reject that may be for an \
                 answered request's frame"
            );
            return None;
        }
        let frames = entry.generation_attempts;
        entry.refuse_generation();
        entry.active.take().map(|active| (active, frames))
    }
}

impl RoutedPathLease {
    /// The busy reject the request now active on this path drew, taken from
    /// the path: `None` when it drew none.
    fn take_busy(&self) -> Option<NetworkReject> {
        let mut state = self.limits.state();
        let entry = state.entries.get_mut(&self.key)?;
        let note = entry.busy.take()?;
        let active = entry.active.as_ref()?;
        active.owner.same_as(&note.owner).then_some(note.reject)
    }
}

impl<T: TransportPort + 'static> BACnetClient<T> {
    /// What a routed request's end leaves behind, before its caller sees
    /// `response`: an answer from the device marks the path answered; a
    /// router's refusal has the dispatch task drop any reassembly of an
    /// answer that was arriving, and sends the device nothing, as for a
    /// request cut off mid-reassembly by its timeout or its caller; a
    /// timeout after the router said it was busy becomes
    /// [`Error::RouterBusyTimeout`], naming that router.
    pub(in crate::client) fn settle_routed(
        &self,
        lease: &RoutedPathLease,
        transaction: (&MacAddr, u8, &TransactionOwner),
        response: &mut Result<TsmResponse, Error>,
    ) {
        let (tsm_mac, invoke_id, owner) = transaction;
        match response {
            Ok(TsmResponse::Failed(error))
                if matches!(
                    **error,
                    Error::NetworkRejected(_) | Error::RoutedPathTooLong { .. }
                ) =>
            {
                self.enqueue_transaction_cleanup(tsm_mac, invoke_id, owner, false, None);
            }
            Err(Error::Timeout(_)) => {
                if let Some(busy) = lease.take_busy() {
                    *response = Err(Error::RouterBusyTimeout(Box::new(busy)));
                }
            }
            Err(Error::Remote(RemoteError::Abort { reason }))
                if *reason == AbortReason::TSM_TIMEOUT =>
            {
                if let Some(busy) = lease.take_busy() {
                    *response = Err(Error::RouterBusyTimeout(Box::new(busy)));
                }
            }
            answered if answered_by_peer(answered) => lease.mark_answered(),
            _ => {}
        }
    }
}

impl RoutedPathEntry {
    /// [`Self::answered`] while it can still matter; the clock is read only
    /// for a path with answered frames on record.
    fn answered_now(&self) -> Option<AnsweredFrames> {
        match self.answered {
            Some(_) => self.answered_at(TokioInstant::now()),
            None => None,
        }
    }

    /// A router refused the generation and no answered frames are pending:
    /// forget the stale answered record, and when the generation sent one
    /// frame, the refusal was that frame's own, so nothing of it is in
    /// flight and the path needs no hold.
    fn refuse_generation(&mut self) {
        self.answered = None;
        if self.generation_attempts == 1 {
            self.generation_end = GenerationEnd::Refused;
        }
    }
}

/// The entry at `key` when a reject the network layer decoded at
/// `ingress_sequence` may be its active request's: a request is active on
/// the path, has sent a frame, and was installed before that decode.
fn claimable<'a>(
    state: &'a mut RoutedPathState,
    key: &RoutedPathKey,
    ingress_sequence: u64,
) -> Option<&'a mut RoutedPathEntry> {
    let entry = state.entries.get_mut(key)?;
    let active = entry.active.as_ref()?;
    (entry.generation_attempts > 0
        && active.path == *key
        && ingress_sequence > active.ingress_floor)
        .then_some(entry)
}

#[cfg(test)]
#[path = "routed_path_reject_tests.rs"]
mod tests;
