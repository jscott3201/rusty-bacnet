//! [`Reporter::pop`]: the next notification out of the confirmed table's
//! resends and the outbox, addressed, with its invoke ID when confirmed.

use alloc::vec::Vec;

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_time::Instant;
use bacnet_types::constructed::BACnetAddress;
#[cfg(feature = "event-reporting")]
use bacnet_types::constructed::BACnetRecipient;
#[cfg(feature = "event-reporting")]
use bacnet_types::enums::ConfirmedServiceChoice;
use bytes::{Bytes, BytesMut};

use super::super::confirmed::Outstanding;
use super::super::outbox::{Entry, Route};
use super::super::{Bindings, Destination, InvokeIds, Notification, peer_address};
use super::Reporter;
use crate::cov::Delivery;

impl Reporter {
    /// The next notification to send, its APDU appended to `out`: a
    /// confirmed one due again first, then a held event notification that
    /// can go now, then the outbox in order. A confirmed one takes an
    /// invoke ID from `ids`. A COV one that gets none ends unsent, and its
    /// subscription is owed a later look; an event one waits in its place,
    /// held. Each `pop` tries the held ones first, oldest first and each
    /// recipient's oldest only, so none is overtaken by a newer
    /// notification taking a free ID, and an event notification to a
    /// recipient with one held is held behind it: each recipient hears its
    /// events in order. COV notifications aren't held back by them.
    /// `bindings` resolves an event notification's Device recipient, once:
    /// one it can't is dropped and counted
    /// ([`ReporterCounters::unresolved`](super::super::ReporterCounters::unresolved)).
    pub fn pop(
        &mut self,
        bindings: &dyn Bindings,
        ids: &mut dyn InvokeIds,
        now: Instant,
        out: &mut BytesMut,
    ) -> Option<Notification> {
        self.last = now;
        self.release(ids);
        if let Some(resend) = self.confirmed.resend(now, self.timing.timeout) {
            let notification = Notification {
                token: resend.token,
                to: resend.to.clone(),
                service: resend.service,
                invoke_id: Some(resend.invoke_id),
                priority: resend.priority,
            };
            let (invoke_id, params) = (resend.invoke_id, resend.params.clone());
            self.encode_confirmed(invoke_id, notification.service, &params, out);
            self.counters.resent += 1;
            return Some(notification);
        }
        #[cfg(feature = "event-reporting")]
        if let Some(held) = self.pop_held(ids, now, out) {
            return Some(held);
        }
        #[cfg(not(feature = "event-reporting"))]
        let _ = bindings;
        loop {
            let (at, mut entry, params) = self.outbox.pop()?;
            #[cfg(not(feature = "event-reporting"))]
            let _ = at;
            self.room_freed = true;
            let (to, peer, priority) = match &mut entry.route {
                Route::Cov { to, .. } => (
                    Destination::Subscriber(to.clone()),
                    peer_address(to),
                    NetworkPriority::Normal,
                ),
                #[cfg(feature = "event-reporting")]
                Route::Event { to, priority } => {
                    let Some(peer) = resolve(bindings, to) else {
                        self.counters.unresolved += 1;
                        continue;
                    };
                    // Resolved once: it keeps this address if it waits.
                    *to = BACnetRecipient::Address(peer.clone());
                    (Destination::Recipient(peer.clone()), peer, *priority)
                }
            };
            #[cfg(feature = "event-reporting")]
            if entry.key().is_none() && self.outbox.holds_for(&peer) {
                // Behind an older one to the same recipient.
                self.outbox.hold(at, entry, &params, false);
                continue;
            }
            let invoke_id = if entry.confirmed {
                // A COV report popped so late that its answers would be
                // cut off ends as the server's worker cut off at its first
                // poll does: failed, held off. Not NotSent and owed, as
                // below: parity with the server needs the hold-off.
                let cutoff = entry.begun.saturating_add(self.timing.cutoff());
                if !entry.token.is_event() && now >= cutoff {
                    self.end_confirmed(entry.token, Delivery::Failed, now);
                    continue;
                }
                let Some(invoke_id) = ids.take(&peer, entry.service.confirmed()) else {
                    match entry.route {
                        // It ends unsent, and the change is looked at
                        // again later.
                        Route::Cov { key, .. } => {
                            self.end_confirmed(entry.token, Delivery::NotSent, now);
                            self.owe_id(key);
                        }
                        #[cfg(feature = "event-reporting")]
                        Route::Event { .. } => self.hold_event(at, entry, &params),
                    }
                    continue;
                };
                Some(invoke_id)
            } else {
                None
            };
            return Some(self.hand_out(entry, params, (to, peer, priority), invoke_id, now, out));
        }
    }

    #[cfg(feature = "event-reporting")]
    /// The oldest held event notification that can go now, handed out:
    /// each recipient's oldest is tried in turn, a confirmed one if `ids`
    /// gives it an ID, so a recipient whose oldest can't go sends nothing
    /// newer. One scan of the outbox and at most one `take` per recipient
    /// held, and none while nothing is held.
    fn pop_held(
        &mut self,
        ids: &mut dyn InvokeIds,
        now: Instant,
        out: &mut BytesMut,
    ) -> Option<Notification> {
        // Held unconfirmed ones too: they wait behind a confirmed one.
        self.outbox.held_entries().next()?;
        let held: Vec<(usize, BACnetAddress, bool)> = self
            .outbox
            .held_entries()
            .filter_map(|(at, entry)| match &entry.route {
                Route::Event {
                    to: BACnetRecipient::Address(peer),
                    ..
                } => Some((at, peer.clone(), entry.confirmed)),
                _ => None,
            })
            .collect();
        let mut tried: Vec<BACnetAddress> = Vec::new();
        for (at, peer, confirmed) in held {
            if tried.contains(&peer) {
                continue;
            }
            let invoke_id = if confirmed {
                let service = ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION;
                let Some(invoke_id) = ids.take(&peer, service) else {
                    tried.push(peer);
                    continue;
                };
                Some(invoke_id)
            } else {
                None
            };
            let (entry, params) = self.outbox.take_at(at);
            self.room_freed = true;
            let Route::Event { priority, .. } = entry.route else {
                unreachable!("only event notifications are held")
            };
            let to = (Destination::Recipient(peer.clone()), peer, priority);
            return Some(self.hand_out(entry, params, to, invoke_id, now, out));
        }
        None
    }

    /// Encode `entry` with `params` into `out`, under `invoke_id` when it
    /// is confirmed, which then joins the confirmed table; count it.
    fn hand_out(
        &mut self,
        entry: Entry,
        params: Vec<u8>,
        (to, peer, priority): (Destination, BACnetAddress, NetworkPriority),
        invoke_id: Option<u8>,
        now: Instant,
        out: &mut BytesMut,
    ) -> Notification {
        let (token, service) = (entry.token, entry.service);
        let start = out.len();
        match invoke_id {
            Some(invoke_id) => {
                self.encode_confirmed(invoke_id, service, &params, out);
                // An event notification's answers are cut off from its
                // first send; a COV report's from when the table began it.
                let begun = if token.is_event() { now } else { entry.begun };
                let cutoff = begun.saturating_add(self.timing.cutoff());
                let deadline = now.saturating_add(self.timing.timeout);
                let object = entry.key().map_or(self.device, |key| key.object());
                self.confirmed.insert(Outstanding::new(
                    (token, object),
                    peer,
                    (to.clone(), priority),
                    (invoke_id, service),
                    params,
                    (deadline, cutoff),
                    self.timing.retries,
                ));
            }
            None => {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
                    service_choice: service.unconfirmed(),
                    service_request: Bytes::from(params),
                });
                encode_apdu(out, &pdu).expect("an UnconfirmedRequest always encodes");
            }
        }
        if token.is_event() {
            self.counters.events += 1;
        } else {
            let counters = &mut self.counters.cov;
            counters.notifications_sent += 1;
            if invoke_id.is_some() {
                counters.notifications_confirmed += 1;
            } else {
                counters.notifications_unconfirmed += 1;
            }
            counters.notification_bytes_sent += (out.len() - start) as u64;
        }
        Notification {
            token,
            to,
            service,
            invoke_id,
            priority,
        }
    }
}

#[cfg(feature = "event-reporting")]
/// The address of the recipient `to`: its own, or its Device's as
/// `bindings` resolves it.
fn resolve(bindings: &dyn Bindings, to: &BACnetRecipient) -> Option<BACnetAddress> {
    match to {
        BACnetRecipient::Address(address) => Some(address.clone()),
        BACnetRecipient::Device(device) => bindings.address_of(*device),
    }
}
