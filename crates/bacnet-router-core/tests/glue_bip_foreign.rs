//! The B/IP glue (feature `bip`) on a foreign-device port: trace E
//! (design 5.2), the broadcasts held for the first registration (decision
//! D17), and what the port does when its registration fails or its socket
//! refuses.

#![cfg(feature = "bip")]

mod fixture;
mod glue;

use bacnet_bip_codec::{BvlcFunction, BvlcResultCode, Bvll};
use bacnet_bip_core::node::{Cast, Node, Permissive};
use bacnet_router_core::bip::{BipPort, HeldBroadcast, Ingress, PortCounters};
use bacnet_router_core::message::MessageType;
use bacnet_router_core::router::{Options, SendRefusal};
use bacnet_router_core::Instant;

use fixture::*;
use glue::*;

/// Each datagram the sink sent since the last call, with when.
fn sent_at(out: &mut Wire, now: Instant, trace: &mut Vec<(Instant, Out)>) {
    trace.extend(out.take().into_iter().map(|sent| (now, sent)));
}

/// A foreign-device port holding two broadcasts, on a router with the
/// default options, started at `t0`: its first
/// registration sent, and the router's startup announcements carried out.
fn started_foreign_port<'h>(
    held: &'h mut [HeldBroadcast],
    out: &mut Wire,
) -> (BipPort<'h>, TestRouter) {
    let t0 = Instant::ZERO;
    let mut bip = BipPort::new(BIP_PORT, node(foreign(2), t0), held).unwrap();
    let mut router = router_with(&[bip.config(1), port_on(2, OWN_1)], Options::HOSTED);
    // The registration waits in the node's queue until the port starts,
    // with no deadline to wake it; once sent, its response timeout is one.
    assert_eq!(bip.next_deadline(), None);
    bip.start(t0, out);
    assert_eq!(bip.next_deadline(), Some(Instant::from_secs(3)));
    router.start();
    let results = pump(&mut router, |send| match send.port {
        BIP_PORT => bip.send(send, t0, out),
        _ => Ok(()),
    });
    assert_eq!(
        results,
        [(0, Ok(())), (0, Ok(())), (1, Ok(())), (1, Ok(()))]
    );
    (bip, router)
}

/// What trace E sends at start, and later the two held announcements
/// released at `released`, each as a Distribute-Broadcast-To-Network.
fn trace_e(released: Instant) -> [(Instant, Out); 3] {
    let dbtn = BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK;
    let ttl = 60u16.to_be_bytes();
    let number_is = message(MessageType::NETWORK_NUMBER_IS, &[0, 1, 1]);
    [
        (
            Instant::ZERO,
            Out::new(BBMD, BvlcFunction::REGISTER_FOREIGN_DEVICE, &ttl),
        ),
        (released, Out::new(BBMD, dbtn, &number_is)),
        (released, Out::new(BBMD, dbtn, &i_am(&[2]))),
    ]
}

/// Trace E (design 5.2): a router port that is a foreign device, holding
/// two broadcasts. At start it registers, and holds its two startup
/// announcements, the Network-Number-Is and the I-Am-Router;
/// the BBMD's success 10 ms later releases them, in order (Annex J.7.4;
/// decision D17).
#[test]
fn foreign_port_announces_after_registration() {
    let mut held = [HeldBroadcast::EMPTY, HeldBroadcast::EMPTY];
    let mut out = Wire::default();
    let (mut bip, mut router) = started_foreign_port(&mut held, &mut out);
    let mut trace = Vec::new();
    sent_at(&mut out, Instant::ZERO, &mut trace);
    let view = bip.node().registration(Instant::ZERO).unwrap();
    assert_eq!((view.held, view.first_resolved), (2, false));

    let t10 = Instant::from_millis(10);
    let success = encode(&Bvll::BvlcResult(BvlcResultCode::SUCCESSFUL_COMPLETION));
    let ingress = bip.receive(
        &datagram(&success, BBMD, UNICAST),
        t10,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(ingress, Ingress::Handled);
    sent_at(&mut out, t10, &mut trace);
    assert_eq!(trace, trace_e(t10));
    assert!(bip.node().registration(t10).unwrap().registered);
    let counters = bip.node_mut().take_counters();
    assert_eq!(
        (counters.broadcasts_held, counters.broadcasts_released),
        (2, 2)
    );
    assert_eq!(bip.counters().refused, 0);

    // From now on a broadcast goes at once.
    let mut host = Recorder::default();
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), t10, &mut host);
    let results = pump(&mut router, |send| bip.send(send, t10, &mut out));
    assert_eq!(results, [(0, Ok(()))]);
    assert_eq!(
        out.take()[0].function,
        BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK
    );
}

/// Trace E's second run: the BBMD never answers, so the held announcements
/// go out when the first attempt's response timeout passes, 3 s after it
/// (D17), and the next attempt comes later.
#[test]
fn foreign_port_announces_after_the_response_timeout() {
    let mut held = [HeldBroadcast::EMPTY, HeldBroadcast::EMPTY];
    let mut out = Wire::default();
    let (mut bip, _router) = started_foreign_port(&mut held, &mut out);
    let mut trace = Vec::new();
    sent_at(&mut out, Instant::ZERO, &mut trace);

    let timeout = bip.next_deadline().unwrap();
    assert_eq!(timeout, Instant::from_secs(3));
    bip.on_deadline(timeout, &mut out);
    sent_at(&mut out, timeout, &mut trace);
    assert_eq!(trace, trace_e(timeout));
    let view = bip.node().registration(timeout).unwrap();
    assert!(view.first_resolved && !view.registered);
    assert!(view.next_attempt.is_some_and(|next| next > timeout));
}

/// The BBMD's success, received at `now`.
fn success_at<const N: usize>(bip: &mut BipPort<'_, 4, N>, now: Instant, out: &mut Wire) {
    let success = encode(&Bvll::BvlcResult(BvlcResultCode::SUCCESSFUL_COMPLETION));
    let ingress = bip.receive(
        &datagram(&success, BBMD, UNICAST),
        now,
        &mut Permissive,
        out,
    );
    assert_eq!(ingress, Ingress::Handled);
}

/// Slots as long as the link's broadcast limit hold what it sends: with a
/// send budget of 510 octets the link's broadcasts are at most 500 octets,
/// and slots of 500 serve trace E as full-size ones do.
#[test]
fn slots_as_long_as_the_links_broadcasts_serve_trace_e() {
    let t0 = Instant::ZERO;
    let link = with(link(foreign(2)), |link| link.budget = Some(510));
    assert_eq!(link.max_npdu(Cast::Broadcast), 500);
    let mut held = [HeldBroadcast::<500>::EMPTY; 2];
    let mut out = Wire::default();
    let node = Node::<4>::new(link, t0).unwrap();
    let mut bip = BipPort::new(BIP_PORT, node, &mut held).unwrap();
    let mut router = router_with(&[bip.config(1), port_on(2, OWN_1)], Options::HOSTED);
    bip.start(t0, &mut out);
    router.start();
    let results = pump(&mut router, |send| match send.port {
        BIP_PORT => bip.send(send, t0, &mut out),
        _ => Ok(()),
    });
    assert!(
        results.iter().all(|(_, result)| result.is_ok()),
        "{results:?}"
    );
    let mut trace = Vec::new();
    sent_at(&mut out, t0, &mut trace);
    let t10 = Instant::from_millis(10);
    success_at(&mut bip, t10, &mut out);
    sent_at(&mut out, t10, &mut trace);
    assert_eq!(trace, trace_e(t10));
}

/// A broadcast the router sends after the first registration's response
/// timeout has passed, with no `on_deadline` call since, still leaves
/// after the broadcasts held for that registration: the port catches the
/// node up first, and they go in the order they were held.
#[test]
fn held_broadcasts_leave_before_a_later_one() {
    let mut held = [HeldBroadcast::EMPTY, HeldBroadcast::EMPTY];
    let mut out = Wire::default();
    let (mut bip, mut router) = started_foreign_port(&mut held, &mut out);
    out.take();

    let late = Instant::from_millis(3_500);
    let mut host = Recorder::default();
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), late, &mut host);
    let results = pump(&mut router, |send| bip.send(send, late, &mut out));
    assert_eq!(results, [(0, Ok(()))]);
    let sent = out.take();
    let dbtn = BvlcFunction::DISTRIBUTE_BROADCAST_TO_NETWORK;
    assert!(sent
        .iter()
        .all(|sent| (sent.to, sent.function) == (BBMD, dbtn)));
    let controls: Vec<u8> = sent.iter().map(|sent| sent.npdu[1]).collect();
    assert_eq!(
        controls,
        [0x80, 0x80, 0x28],
        "the Number-Is, the I-Am-Router, then the Who-Is"
    );
    assert_eq!(
        sent[0].npdu,
        message(MessageType::NETWORK_NUMBER_IS, &[0, 1, 1])
    );
    assert_eq!(sent[1].npdu, i_am(&[2]));
}

/// The BBMD refuses the first registration: the broadcasts held for it are
/// dropped, never sent (D17), and the next one goes at once.
#[test]
fn a_refused_registration_discards_the_held_broadcasts() {
    let mut held = [HeldBroadcast::EMPTY, HeldBroadcast::EMPTY];
    let mut out = Wire::default();
    let (mut bip, mut router) = started_foreign_port(&mut held, &mut out);
    out.take();

    let t10 = Instant::from_millis(10);
    let nak = encode(&Bvll::BvlcResult(
        BvlcResultCode::REGISTER_FOREIGN_DEVICE_NAK,
    ));
    let ingress = bip.receive(
        &datagram(&nak, BBMD, UNICAST),
        t10,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(ingress, Ingress::Handled);
    assert_eq!(out.take(), [], "nothing held goes out");
    let counters = bip.node_mut().take_counters();
    assert_eq!(
        (counters.broadcasts_discarded, counters.broadcasts_released),
        (2, 0)
    );

    let mut host = Recorder::default();
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), t10, &mut host);
    let results = pump(&mut router, |send| bip.send(send, t10, &mut out));
    assert_eq!(results, [(0, Ok(()))]);
    let sent = out.take();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].npdu[1], 0x28, "the Who-Is alone");
}

/// A registration the socket refuses ends its attempt as a local failure,
/// counted by the node, and the next attempt comes one response timeout
/// later; the first attempt is resolved, so nothing is held for it.
#[test]
fn a_registration_the_socket_refuses_ends_its_attempt() {
    let t0 = Instant::ZERO;
    let mut held: [HeldBroadcast; 2] = [HeldBroadcast::EMPTY, HeldBroadcast::EMPTY];
    let mut bip = BipPort::new(BIP_PORT, node(foreign(2), t0), &mut held).unwrap();
    let mut out = Wire {
        refuse: Some(SendRefusal::Full),
        ..Wire::default()
    };
    bip.start(t0, &mut out);
    assert_eq!(out.calls, 1);
    let view = bip.node().registration(t0).unwrap();
    assert!(view.first_resolved && !view.attempt_in_flight);
    let retry = Instant::from_secs(3);
    assert_eq!(bip.next_deadline(), Some(retry));
    let counters = bip.node_mut().take_counters();
    assert_eq!(counters.register_foreign_device.local_errors, 1);
    assert_eq!(bip.counters().refused, 0, "the node counts its own attempt");

    out.refuse = None;
    bip.on_deadline(retry, &mut out);
    let ttl = 60u16.to_be_bytes();
    assert_eq!(
        out.take(),
        [Out::new(BBMD, BvlcFunction::REGISTER_FOREIGN_DEVICE, &ttl)]
    );
}

/// A foreign device holds two broadcasts while its first registration is
/// outstanding; a third is refused, as a full queue.
#[test]
fn a_full_hold_refuses_the_next_broadcast() {
    let mut held = [HeldBroadcast::EMPTY, HeldBroadcast::EMPTY];
    let mut out = Wire::default();
    let (mut bip, mut router) = started_foreign_port(&mut held, &mut out);
    let t1 = Instant::from_millis(1);
    let mut host = Recorder::default();
    let _ = router.receive(frame(1, &[5], true, &GLOBAL_WHO_IS), t1, &mut host);
    let results = pump(&mut router, |send| bip.send(send, t1, &mut out));
    assert_eq!(results, [(0, Err(SendRefusal::Full))]);
    assert_eq!(router.counters().refused, 1);
    let counters = bip.node_mut().take_counters();
    assert_eq!(
        (counters.broadcasts_held, counters.broadcasts_hold_full),
        (2, 1)
    );
}

/// Each datagram of a foreign-device port's own that the socket refuses
/// is counted once: a NAK, and each released broadcast. Taking the counts
/// starts them again from zero.
#[test]
fn refused_datagrams_of_a_foreign_port_are_counted() {
    let mut held = [HeldBroadcast::EMPTY, HeldBroadcast::EMPTY];
    let mut out = Wire::default();
    let (mut bip, _router) = started_foreign_port(&mut held, &mut out);
    out.refuse = Some(SendRefusal::Full);
    let t10 = Instant::from_millis(10);
    let read = encode(&Bvll::ReadBroadcastDistributionTable);
    bip.receive(
        &datagram(&read, PEER, UNICAST),
        t10,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(bip.counters().refused, 1, "the NAK");
    success_at(&mut bip, t10, &mut out);
    assert_eq!(bip.counters().refused, 3, "and the two held broadcasts");
    assert_eq!(bip.take_counters().refused, 3);
    assert_eq!(bip.counters(), PortCounters::default(), "drained");
    bip.receive(
        &datagram(&read, PEER, UNICAST),
        t10,
        &mut Permissive,
        &mut out,
    );
    assert_eq!(bip.take_counters().refused, 1, "counted from zero again");
}
