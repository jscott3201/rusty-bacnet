//! [`Reporter::pop`]: the next notification out of the confirmed table's
//! resends and the outbox, addressed, with its invoke ID when confirmed.

use bacnet_encoding::apdu::{Apdu, UnconfirmedRequest, encode_apdu};
use bacnet_encoding::npdu::view::NetworkPriority;
use bacnet_time::Instant;
use bacnet_types::constructed::{BACnetAddress, BACnetRecipient};
use bytes::{Bytes, BytesMut};

use super::super::confirmed::Outstanding;
use super::super::outbox::Route;
use super::super::{Bindings, Destination, InvokeIds, Notification, peer_address};
use super::Reporter;
use crate::cov::Delivery;

impl Reporter {
    /// The next notification to send, its APDU appended to `out`: a
    /// confirmed one due again first, then the outbox in order. A confirmed
    /// one takes an invoke ID from `ids`. A COV one that gets none ends
    /// unsent, and its subscription is owed a later look; an event one
    /// waits in its place until an ID comes back, passed by meanwhile.
    /// `bindings` resolves an event notification's Device recipient; one it
    /// can't is dropped and counted ([`ReporterCounters::unresolved`]).
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
        loop {
            let (at, entry, params) = self.outbox.pop()?;
            self.room_freed = true;
            let (token, (confirmed, service)) = (entry.token, (entry.confirmed, entry.service));
            let (to, peer, priority) = match &entry.route {
                Route::Cov { to, .. } => (
                    Destination::Subscriber(to.clone()),
                    peer_address(to),
                    NetworkPriority::Normal,
                ),
                Route::Event { to, priority } => {
                    let Some(peer) = resolve(bindings, to) else {
                        self.counters.unresolved += 1;
                        continue;
                    };
                    (Destination::Recipient(peer.clone()), peer, *priority)
                }
            };
            let start = out.len();
            let invoke_id = if confirmed {
                // A COV report popped so late that its answers would be
                // cut off ends as the server's worker cut off at its first
                // poll does: failed, held off. Not NotSent and owed, as
                // below: parity with the server needs the hold-off. An
                // event notification's answers are cut off from its first
                // send.
                let begun = if token.is_event() { now } else { entry.begun };
                let cutoff = begun.saturating_add(self.timing.cutoff());
                if now >= cutoff {
                    self.end_confirmed(token, Delivery::Failed, now);
                    continue;
                }
                let Some(invoke_id) = ids.take(&peer, service.confirmed()) else {
                    match entry.route {
                        // It ends unsent, and the change is looked at
                        // again later.
                        Route::Cov { key, .. } => {
                            self.end_confirmed(token, Delivery::NotSent, now);
                            self.owe_id(key);
                        }
                        Route::Event { .. } => self.hold_event(at, entry, &params),
                    }
                    continue;
                };
                self.encode_confirmed(invoke_id, service, &params, out);
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
                Some(invoke_id)
            } else {
                let pdu = Apdu::UnconfirmedRequest(UnconfirmedRequest {
                    service_choice: service.unconfirmed(),
                    service_request: Bytes::from(params),
                });
                encode_apdu(out, &pdu).expect("an UnconfirmedRequest always encodes");
                None
            };
            if token.is_event() {
                self.counters.events += 1;
            } else {
                let counters = &mut self.counters.cov;
                counters.notifications_sent += 1;
                if confirmed {
                    counters.notifications_confirmed += 1;
                } else {
                    counters.notifications_unconfirmed += 1;
                }
                counters.notification_bytes_sent += (out.len() - start) as u64;
            }
            return Some(Notification {
                token,
                to,
                service,
                invoke_id,
                priority,
            });
        }
    }
}

/// The address of the recipient `to`: its own, or its Device's as
/// `bindings` resolves it.
fn resolve(bindings: &dyn Bindings, to: &BACnetRecipient) -> Option<BACnetAddress> {
    match to {
        BACnetRecipient::Address(address) => Some(address.clone()),
        BACnetRecipient::Device(device) => bindings.address_of(*device),
    }
}
