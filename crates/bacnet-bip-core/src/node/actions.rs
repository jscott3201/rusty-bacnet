//! What a [`Node`](super::Node) asks its caller to do, and the fixed queue
//! that holds it until polled.

use core::net::SocketAddrV4;

use bacnet_bip_codec::Bvll;

use super::exchange::ExchangeId;
use super::registration::RegistrationEvent;

/// Something for the caller to do. Poll
/// [`Node::poll_action`](super::Node::poll_action) until [`Action::Idle`]
/// after every call into the node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing more to do.
    Idle,
    /// Send a Register-Foreign-Device ([`Action::message`]) to `to`, then
    /// report it with [`Node::exchange_sent`](super::Node::exchange_sent)
    /// or [`Node::exchange_failed`](super::Node::exchange_failed).
    Register {
        /// The attempt's exchange.
        exchange: ExchangeId,
        /// The BBMD.
        to: SocketAddrV4,
        /// The time-to-live to ask for.
        ttl: u16,
    },
    /// A manual exchange's response timeout passed unanswered.
    ExchangeEnded {
        /// The exchange.
        exchange: ExchangeId,
    },
    /// Send the broadcasts held for the first registration attempt, in the
    /// order they were held.
    ReleaseHeld,
    /// Drop the broadcasts held for the first registration attempt: the
    /// BBMD refused it.
    DiscardHeld,
    /// A change in the foreign device's registration.
    Registration(RegistrationEvent),
    /// Send a Delete-Foreign-Device-Table-Entry for `entry`
    /// ([`Action::message`]) to `to`, the BBMD a move just left
    /// ([`Foreign::release_previous`](super::Foreign::release_previous)),
    /// then report it with
    /// [`Node::exchange_sent`](super::Node::exchange_sent) or
    /// [`Node::exchange_failed`](super::Node::exchange_failed). What comes
    /// of it is counted ([`NodeCounters`](super::NodeCounters)'s
    /// `previous_release*` fields) and reported as
    /// [`RegistrationEvent::PreviousRelease`].
    ReleasePrevious {
        /// The release's exchange.
        exchange: ExchangeId,
        /// The BBMD the device left.
        to: SocketAddrV4,
        /// The entry to delete: this node's own B/IP address.
        entry: SocketAddrV4,
    },
}

impl Action {
    /// The message a [`Register`](Action::Register) or a
    /// [`ReleasePrevious`](Action::ReleasePrevious) sends.
    #[must_use]
    pub const fn message(&self) -> Option<Bvll<'static>> {
        match *self {
            Self::Register { ttl, .. } => Some(Bvll::RegisterForeignDevice { ttl }),
            Self::ReleasePrevious { entry, .. } => Some(Bvll::DeleteForeignDeviceTableEntry(entry)),
            _ => None,
        }
    }
}

/// Registration events one call can raise, at most: an attempt's end and
/// the lease's lapse, or the lapse and an answer.
const EVENTS: usize = 3;

/// Pending actions, in fixed storage that can't overflow: one ended mark
/// per exchange slot, a short list of registration events, one verdict on
/// the held broadcasts (the first attempt ends once), one registration
/// attempt (one is in flight at a time), and one release of the BBMD a move
/// left (one move per call, and the caller polls after each). They are
/// polled in that order.
#[derive(Debug, Clone)]
pub(crate) struct Actions<const X: usize> {
    /// Per slot, the generation of an exchange that timed out.
    ended: [Option<u32>; X],
    events: [Option<RegistrationEvent>; EVENTS],
    held: Option<bool>,
    register: Option<(ExchangeId, SocketAddrV4, u16)>,
    /// The release's exchange, the BBMD left and this node's address.
    release: Option<(ExchangeId, SocketAddrV4, SocketAddrV4)>,
}

impl<const X: usize> Actions<X> {
    pub(crate) const fn new() -> Self {
        Self {
            ended: [None; X],
            events: [None; EVENTS],
            held: None,
            register: None,
            release: None,
        }
    }

    pub(crate) fn ended(&mut self, exchange: ExchangeId) {
        self.ended[exchange.slot()] = Some(exchange.generation());
    }

    /// Queue `event`; the newest replaces the last if the list is full.
    pub(crate) fn event(&mut self, event: RegistrationEvent) {
        match self.events.iter_mut().find(|slot| slot.is_none()) {
            Some(slot) => *slot = Some(event),
            None => self.events[EVENTS - 1] = Some(event),
        }
    }

    /// Queue `event` if the list has room: a release's outcome never takes
    /// a registration event's place.
    pub(crate) fn event_if_room(&mut self, event: RegistrationEvent) {
        if let Some(slot) = self.events.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(event);
        }
    }

    pub(crate) fn held(&mut self, release: bool) {
        self.held = Some(release);
    }

    pub(crate) fn register(&mut self, exchange: ExchangeId, to: SocketAddrV4, ttl: u16) {
        self.register = Some((exchange, to, ttl));
    }

    /// Drop a queued registration attempt for `exchange`, which ended
    /// before the caller polled it.
    pub(crate) fn forget_register(&mut self, exchange: ExchangeId) {
        if self
            .register
            .is_some_and(|(queued, _, _)| queued == exchange)
        {
            self.register = None;
        }
    }

    pub(crate) fn release(&mut self, exchange: ExchangeId, to: SocketAddrV4, entry: SocketAddrV4) {
        self.release = Some((exchange, to, entry));
    }

    /// Take the release queued and never polled, if it went to a BBMD
    /// `drop` picks.
    pub(crate) fn take_release_if(
        &mut self,
        drop: impl FnOnce(SocketAddrV4) -> bool,
    ) -> Option<ExchangeId> {
        let (exchange, to, _) = self.release?;
        if !drop(to) {
            return None;
        }
        self.release = None;
        Some(exchange)
    }

    pub(crate) fn poll(&mut self) -> Action {
        for (slot, ended) in self.ended.iter_mut().enumerate() {
            if let Some(generation) = ended.take() {
                return Action::ExchangeEnded {
                    exchange: ExchangeId::new(slot, generation),
                };
            }
        }
        if let Some(event) = self.events[0].take() {
            // The emptied slot moves to the back one swap at a time:
            // `rotate_left` compiles to a memmove call, 0.8-1.4 KB of code
            // on a microcontroller.
            for index in 1..EVENTS {
                self.events.swap(index - 1, index);
            }
            return Action::Registration(event);
        }
        if let Some(release) = self.held.take() {
            return if release {
                Action::ReleaseHeld
            } else {
                Action::DiscardHeld
            };
        }
        if let Some((exchange, to, ttl)) = self.register.take() {
            return Action::Register { exchange, to, ttl };
        }
        match self.release.take() {
            Some((exchange, to, entry)) => Action::ReleasePrevious {
                exchange,
                to,
                entry,
            },
            None => Action::Idle,
        }
    }
}

#[cfg(test)]
mod tests {
    use bacnet_time::Instant;

    use super::{Action, Actions, EVENTS, RegistrationEvent};

    const fn renewed(secs: u64) -> RegistrationEvent {
        RegistrationEvent::Renewed {
            until: Instant::from_secs(secs),
        }
    }

    /// Every event `actions` holds, in the order polled.
    fn polled(actions: &mut Actions<1>) -> [Option<RegistrationEvent>; EVENTS + 1] {
        let mut out = [None; EVENTS + 1];
        for slot in &mut out {
            match actions.poll() {
                Action::Registration(event) => *slot = Some(event),
                Action::Idle => break,
                other => panic!("{other:?}"),
            }
        }
        out
    }

    #[test]
    fn events_are_polled_in_order() {
        let mut actions = Actions::<1>::new();
        for secs in 1..=3 {
            actions.event(renewed(secs));
        }
        assert_eq!(
            polled(&mut actions),
            [Some(renewed(1)), Some(renewed(2)), Some(renewed(3)), None]
        );
        // Polling frees the slots for the next ones.
        actions.event(RegistrationEvent::Lapsed);
        actions.event(RegistrationEvent::Unanswered);
        let next = [RegistrationEvent::Lapsed, RegistrationEvent::Unanswered];
        assert_eq!(
            polled(&mut actions),
            [Some(next[0]), Some(next[1]), None, None]
        );
    }

    #[test]
    fn the_newest_event_replaces_the_last_when_the_list_is_full() {
        let mut actions = Actions::<1>::new();
        for secs in 1..=4 {
            actions.event(renewed(secs));
        }
        assert_eq!(
            polled(&mut actions),
            [Some(renewed(1)), Some(renewed(2)), Some(renewed(4)), None]
        );
    }
}
