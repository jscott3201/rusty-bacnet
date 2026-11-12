//! Each recipient hears its events in order (#2303): a confirmed event
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
