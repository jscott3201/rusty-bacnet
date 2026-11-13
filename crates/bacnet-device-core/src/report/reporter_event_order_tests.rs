//! First sends in order per address (#2303): a confirmed event
//! notification held for an invoke ID is tried first at every `pop`, and a
//! later one to the same recipient waits behind it, whoever frees the ID.

use super::*;
use bacnet_encoding::constructed::decode_event_notification;
use bacnet_types::enums::EventState;

fn other_address() -> BACnetAddress {
    BACnetAddress {
        network_number: 0,
        mac_address: MacAddr::from_slice(&[0x0A, 0, 0, 8, 0xBA, 0xC0]),
    }
}

/// The state an event notification reports the object went to.
fn to_state(apdu: &Apdu) -> EventState {
    let params = match apdu {
        Apdu::UnconfirmedRequest(request) => &request.service_request,
        Apdu::ConfirmedRequest(request) => &request.service_request,
        other => panic!("not a notification: {other:?}"),
    };
    decode_event_notification(params).unwrap().to_state
}

impl Device {
    /// Bring `object` back to 50, under its high limit.
    fn lower(&mut self, object: ObjectIdentifier, now: Instant) {
        let applied = self
            .reporter
            .update(&mut self.db, object, LocalValue::Real(50.0), now);
        assert!(applied.is_ok(), "{applied:?}");
    }

    /// Answer the confirmed event notification `invoke_id` from `from`.
    fn ack_event(&mut self, from: BACnetAddress, invoke_id: u8, now: Instant) {
        let mut apdu = BytesMut::new();
        let ack = SimpleAck {
            invoke_id,
            service_choice: ConfirmedServiceChoice::CONFIRMED_EVENT_NOTIFICATION,
        };
        encode_apdu(&mut apdu, &Apdu::SimpleAck(ack)).unwrap();
        assert!(self.reporter.reply(&from, &apdu, &mut self.ids, now));
    }

    /// Pop everything at `now`, acknowledging each confirmed event
    /// notification as it goes out; the states, in order sent.
    fn drain_events(&mut self, now: Instant) -> Vec<EventState> {
        let mut states = Vec::new();
        while let Some((sent, apdu)) = self.pop(now) {
            if !is_event(&apdu) {
                continue;
            }
            states.push(to_state(&apdu));
            if let (Some(id), Destination::Recipient(from)) = (sent.invoke_id, sent.to) {
                self.ack_event(from, id, now);
            }
        }
        states
    }
}

/// The reviewer's first ordering: TO-OFFNORMAL held, the value back to
/// normal, and an ID freed in the host's pool, not yet reported: the next
/// `pop` sends the alarm first.
#[test]
fn a_held_event_is_not_overtaken_with_an_id_from_the_host() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    assert!(device.pop(ms(100)).is_none(), "held for an ID");
    device.lower(alarm(), ms(200));
    assert!(device.pop(ms(200)).is_none(), "held behind it");
    // Free in the host's pool; no `invoke_ids_released`, no deadline.
    device.ids.limit = 1;
    let states = device.drain_events(ms(300));
    assert_eq!(states, vec![EventState::HIGH_LIMIT, EventState::NORMAL]);
    assert!(!device.reporter.owes_invoke_id());
}

/// The reviewer's second ordering: the ID comes back with the reporter's
/// own COV acknowledgement.
#[test]
fn a_held_event_is_not_overtaken_with_an_id_a_cov_ack_frees() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    device.ids.limit = 1;
    device.subscribe_to(alarm2(), 1, true, ms(10));
    let (initial, _) = device.pop(ms(10)).expect("the COV report takes the ID");
    device.raise(alarm(), ms(100));
    assert!(device.pop(ms(100)).is_none(), "held for an ID");
    device.lower(alarm(), ms(200));
    assert!(device.pop(ms(200)).is_none(), "held behind it");
    assert!(device.ack(initial.invoke_id.unwrap(), ms(300)));
    let (first, apdu) = device.pop(ms(300)).expect("the held alarm");
    assert_eq!(to_state(&apdu), EventState::HIGH_LIMIT);
    assert!(device.pop(ms(300)).is_none(), "the next waits for an ID");
    let from = recipient_address();
    device.ack_event(from, first.invoke_id.unwrap(), ms(400));
    let states = device.drain_events(ms(400));
    assert_eq!(states, vec![EventState::NORMAL]);
}

#[test]
fn an_unconfirmed_event_waits_behind_a_held_one_to_its_recipient() {
    let both = vec![address_destination(true), address_destination(false)];
    let mut device = device(OutboxLimits::default(), 0, both, 1);
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    assert!(
        device.pop(ms(100)).is_none(),
        "the unconfirmed one waits too"
    );
    // Still behind while the head is refused again.
    assert!(device.pop(ms(150)).is_none(), "behind the refused head");
    device.ids.limit = 1;
    let (confirmed, _) = device.pop(ms(200)).expect("the held one first");
    assert!(confirmed.invoke_id.is_some());
    let (unconfirmed, _) = device.pop(ms(200)).expect("then the one behind it");
    assert_eq!(unconfirmed.invoke_id, None);
}

#[test]
fn a_held_event_holds_back_only_its_own_recipient() {
    let mut other = address_destination(false);
    other.recipient = BACnetRecipient::Address(other_address());
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true), other],
        1,
    );
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    let (sent, _) = device.pop(ms(100)).expect("the other recipient's");
    assert_eq!(sent.to, Destination::Recipient(other_address()));
    assert!(device.pop(ms(100)).is_none());
    assert!(device.reporter.owes_invoke_id());
}

#[test]
fn a_cov_report_does_not_take_the_id_a_held_event_waits_for() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    device.subscribe_to(ai(), 1, true, ms(10));
    let (initial, _) = device.pop(ms(10)).expect("the initial report");
    assert!(device.ack(initial.invoke_id.unwrap(), ms(20)));
    device.run_until(ms(20));
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    assert!(device.pop(ms(100)).is_none(), "held for an ID");
    // A confirmed COV report queued after it, then one ID free.
    let applied = device
        .reporter
        .update(&mut device.db, ai(), LocalValue::Real(20.0), ms(200));
    assert!(applied.is_ok());
    device.ids.limit = 1;
    let (first, apdu) = device.pop(ms(200)).expect("one goes");
    assert!(
        is_event(&apdu),
        "the held event, not the COV report: {apdu:?}"
    );
    assert_eq!(first.service, NotifyService::Event);
    assert!(
        device.pop(ms(200)).is_none(),
        "the COV report is owed an ID"
    );
    assert!(device.reporter.owes_invoke_id());
}

/// The first send of each keeps the order; a resend of an older confirmed
/// one can land after a newer one, each on its own invoke ID, and the
/// receiver orders them by their timestamps.
#[test]
fn a_resend_can_follow_a_newer_event_to_its_recipient() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    device.ids.limit = 4;
    device.raise(alarm(), ms(100));
    let (first, apdu) = device.pop(ms(100)).expect("the alarm");
    assert_eq!(to_state(&apdu), EventState::HIGH_LIMIT);
    device.lower(alarm(), ms(200));
    let (_, apdu) = device
        .pop(ms(200))
        .expect("the return, while the alarm is unanswered");
    assert_eq!(to_state(&apdu), EventState::NORMAL);
    let mut wire = Vec::new();
    while let Some(at) = device
        .reporter
        .next_deadline()
        .filter(|at| *at <= ms(3_500))
    {
        device
            .reporter
            .on_deadline(&mut device.db, &mut device.ids, at);
        while let Some((sent, apdu)) = device.pop(at) {
            if is_event(&apdu) {
                wire.push((sent.invoke_id, to_state(&apdu)));
            }
        }
    }
    assert_eq!(
        wire.first(),
        Some(&(first.invoke_id, EventState::HIGH_LIMIT))
    );
}

#[test]
fn an_event_held_only_for_order_is_counted_owed_when_refused() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    assert!(device.pop(ms(100)).is_none());
    device.lower(alarm(), ms(200));
    assert!(
        device.pop(ms(200)).is_none(),
        "held behind, not refused yet"
    );
    assert_eq!(device.reporter.counters().owed, 1);
    device.ids.limit = 1;
    assert!(device.pop(ms(300)).is_some(), "the alarm");
    assert!(device.pop(ms(300)).is_none(), "the return, refused");
    assert_eq!(device.reporter.counters().owed, 2, "two waited for an ID");
}

#[test]
fn a_held_device_recipient_keeps_its_resolved_address() {
    let bound = ObjectIdentifier::new(ObjectType::DEVICE, 77).unwrap();
    let to = destination(BACnetRecipient::Device(bound), true);
    let mut device = device(OutboxLimits::default(), 0, vec![to], 1);
    device.ids.limit = 0;
    device.raise(alarm(), ms(100));
    assert!(device.pop_with(&super::OneBinding, ms(100)).is_none());
    device.ids.limit = 1;
    let (sent, _) = device
        .pop_with(&super::OneBinding, ms(200))
        .expect("the held Device recipient's");
    assert_eq!(sent.to, Destination::Recipient(recipient_address()));
    assert!(!device.reporter.owes_invoke_id());
}

/// Invoke IDs one per peer, as a host may count them.
#[derive(Default)]
struct PerPeer {
    taken: Vec<(BACnetAddress, u8)>,
    next: u8,
}

impl InvokeIds for PerPeer {
    fn take(&mut self, to: &BACnetAddress, _: ConfirmedServiceChoice) -> Option<u8> {
        if self.taken.iter().any(|taken| taken.0 == *to) {
            return None;
        }
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        self.taken.push((to.clone(), id));
        Some(id)
    }
    fn release(&mut self, to: &BACnetAddress, id: u8) {
        let at = self
            .taken
            .iter()
            .position(|taken| taken.0 == *to && taken.1 == id);
        self.taken.swap_remove(at.expect("released as taken"));
    }
}

/// An event held for one peer whose ID never comes back doesn't hold back
/// a COV report owed an ID by another peer: it goes as that peer's ID is
/// free.
#[test]
fn an_event_held_for_one_peer_does_not_hold_back_another_peers_cov() {
    let mut device = device(
        OutboxLimits::default(),
        0,
        vec![address_destination(true)],
        1,
    );
    let mut ids = PerPeer::default();
    let pop = |device: &mut Device, ids: &mut PerPeer, now| {
        let mut out = BytesMut::new();
        device.reporter.pop(&NoBindings, ids, now, &mut out)
    };
    device.subscribe_to(ai(), 1, true, ms(10));
    let first = pop(&mut device, &mut ids, ms(10)).expect("the subscriber's one ID");
    device.subscribe_to(ai2(), 2, true, ms(20));
    assert!(pop(&mut device, &mut ids, ms(20)).is_none(), "owed an ID");
    device.raise(alarm(), ms(100));
    let alarm_sent = pop(&mut device, &mut ids, ms(100)).expect("the recipient's one ID");
    assert_eq!(alarm_sent.service, NotifyService::Event);
    device.lower(alarm(), ms(200));
    assert!(
        pop(&mut device, &mut ids, ms(200)).is_none(),
        "held for the recipient"
    );
    let mut apdu = BytesMut::new();
    let ack = SimpleAck {
        invoke_id: first.invoke_id.unwrap(),
        service_choice: ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION,
    };
    encode_apdu(&mut apdu, &Apdu::SimpleAck(ack)).unwrap();
    let subscriber = peer_address(&peer());
    assert!(device.reporter.reply(&subscriber, &apdu, &mut ids, ms(300)));
    let due = device.reporter.next_deadline().expect("the owed look");
    assert_eq!(due, ms(300));
    device.reporter.on_deadline(&mut device.db, &mut ids, due);
    let sent = pop(&mut device, &mut ids, ms(300)).expect("the owed COV report");
    assert_eq!(sent.service, NotifyService::Cov);
}
