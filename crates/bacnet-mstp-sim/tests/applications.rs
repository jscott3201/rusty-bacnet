//! Application callbacks (#1796): what an application sees, and where its
//! answers and its own sends go.

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use bacnet_mstp_codec::FrameType;
use bacnet_mstp_sim::{Answer, Fault, MasterSpec, Outbox, Received, Sim, SimConfig, BROADCAST};

use common::*;

/// What a callback saw: source, destination, NPDU and whether a reply was
/// expected.
type Seen = Rc<RefCell<Vec<(u8, u8, Vec<u8>, bool)>>>;

/// An application that notes each NPDU, answers an unconfirmed one to its
/// source and sends a unicast and a broadcast of its own for it.
fn noting(seen: &Seen) -> impl FnMut(&Received<'_>, &mut Outbox) -> Option<Answer> {
    let seen = Rc::clone(seen);
    move |received, outbox| {
        seen.borrow_mut().push((
            received.source,
            received.destination,
            received.npdu.to_vec(),
            received.expects_reply,
        ));
        if received.expects_reply || received.npdu != npdu(false, 0x11) {
            return None;
        }
        outbox.send(5, &npdu(false, 0x55));
        outbox.broadcast(&npdu(false, 0xBB));
        Some(Answer::Reply {
            delay: millis(4),
            npdu: npdu(false, 0xA0),
        })
    }
}

/// An answer to an NPDU that expects none can't go in a reply window: it
/// waits in the queue for the answering node's token, as do the unicast
/// and the broadcast the application sends of its own accord. Every NPDU
/// the core delivers reaches the application, broadcasts included.
#[test]
fn answers_to_unconfirmed_npdus_and_own_sends_wait_for_the_token() {
    let seen = Seen::default();
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 5, BAUD));
    sim.add_master(MasterSpec::new(3, 5, BAUD).application(noting(&seen)));
    sim.add_master(MasterSpec::new(5, 5, BAUD));
    sim.enqueue(ms(1_000), 0, 3, &npdu(false, 0x11));
    sim.enqueue(ms(1_000), 5, BROADCAST, &npdu(false, 0x22));
    sim.run_until(ms(2_000));
    sim.assert_no_violations();

    assert_eq!(
        *seen.borrow(),
        [
            (5, BROADCAST, npdu(false, 0x22), false),
            (0, 3, npdu(false, 0x11), false),
        ]
    );
    let from_3 = sent_by(sim.frames(), 3, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY);
    let sent: Vec<_> = from_3
        .iter()
        .map(|frame| (frame.destination(), frame.data.clone()))
        .collect();
    assert_eq!(sent.len(), 3, "{}", log(sim.frames()));
    for expected in [
        (Some(0), npdu(false, 0xA0)),
        (Some(5), npdu(false, 0x55)),
        (Some(BROADCAST), npdu(false, 0xBB)),
    ] {
        assert!(sent.contains(&expected), "{expected:?} not in {sent:?}");
    }
    let request = sent_by(sim.frames(), 0, FrameType::BACNET_DATA_NOT_EXPECTING_REPLY)[0];
    // Each goes out while station 3 holds the token, after the request.
    for frame in from_3 {
        assert!(frame.start > request.end);
    }
    assert!(sim
        .deliveries(0)
        .iter()
        .any(|delivery| delivery.source == 3 && delivery.data == npdu(false, 0xA0)));
}

/// A request the application leaves unanswered stays undecided, so the
/// core sends Reply Postponed when its decision deadline passes.
#[test]
fn a_request_the_application_leaves_unanswered_is_postponed() {
    let seen = Seen::default();
    let mut sim = Sim::new(SimConfig::new(BAUD));
    sim.add_master(MasterSpec::new(0, 3, BAUD));
    sim.add_master(MasterSpec::new(3, 3, BAUD).application(noting(&seen)));
    sim.enqueue(ms(1_000), 0, 3, &npdu(true, 0x33));
    sim.run_until(ms(2_000));
    sim.assert_no_violations();

    assert_eq!(*seen.borrow(), [(0, 3, npdu(true, 0x33), true)]);
    assert_eq!(
        sent_by(sim.frames(), 3, FrameType::REPLY_POSTPONED).len(),
        1
    );
}

/// An answer the application planned before its node lost power is gone
/// with the application's state: whether the node is back on when the
/// answer falls due (30 ms) or still off (300 ms), it never sends it, and
/// nothing is refused at its queue in the meantime.
#[test]
fn a_planned_answer_does_not_survive_a_power_cycle() {
    for back_on in [30, 300] {
        let answer_late = |received: &Received<'_>, _: &mut Outbox| {
            (received.npdu == npdu(false, 0x44)).then(|| Answer::Reply {
                delay: millis(200),
                npdu: npdu(false, 0xA4),
            })
        };
        let mut sim = Sim::new(SimConfig::new(BAUD));
        sim.add_master(MasterSpec::new(0, 3, BAUD));
        sim.add_master(MasterSpec::new(3, 3, BAUD).application(answer_late));
        sim.enqueue(ms(1_000), 0, 3, &npdu(false, 0x44));
        while sim.deliveries(3).is_empty() {
            assert!(sim.now() < ms(2_000), "no delivery");
            sim.run_for(millis(1));
        }
        let delivered = sim.deliveries(3)[0].at;
        sim.inject(delivered + millis(10), Fault::PowerOff { station: 3 });
        sim.inject(delivered + millis(back_on), Fault::PowerOn { station: 3 });
        sim.run_until(ms(3_000));
        sim.assert_no_violations();

        assert!(
            sim.frames()
                .iter()
                .all(|frame| frame.data != npdu(false, 0xA4)),
            "back on after {back_on} ms:\n{}",
            log(sim.frames())
        );
        assert!(
            sim.rejects(3).is_empty(),
            "back on after {back_on} ms: {:?}",
            sim.rejects(3)
        );
    }
}
