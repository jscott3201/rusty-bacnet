//! Requests the router's own application sends, routed by the table with
//! an explicit source (#1699; Clauses 6.1, 6.5, 6.6.3.1).

mod fixture;

use bacnet_router_core::npdu::{Address, NetworkMessageType, NetworkPriority, Npdu};
use bacnet_router_core::router::{
    Action, Cause, Kind, Options, OriginateError, Permissive, Request, SourceSelection,
};
use bacnet_router_core::{Duration, Instant, PortId};
use fixture::{
    addr, at, bytes, deliver, drain, learn_9_via_r7, router, Outcome, Sent, TestRouter, CONFIRMED,
    OWN_0, OWN_1, R7, UNCONFIRMED,
};

/// The standard router with network 9 beyond R7 on port 1.
fn with_9() -> TestRouter {
    let mut router = router();
    learn_9_via_r7(&mut router, Instant::ZERO);
    router
}

/// Originate `request` at `now` and poll what it asked for.
fn originate(
    router: &mut TestRouter,
    request: &Request<'_>,
    now: Instant,
) -> (Result<(), OriginateError>, Vec<Sent>) {
    let result = router.originate(request, now);
    (result, drain(router))
}

fn request<'a>(network: u16, mac: &'a [u8], apdu: &'a [u8]) -> Request<'a> {
    Request::new(Address { network, mac }, apdu)
}

#[test]
fn a_remote_request_goes_to_the_next_router_from_the_egress_address() {
    let mut router = with_9();
    let mut confirmed = request(9, &[5], CONFIRMED);
    confirmed.expecting_reply = true;
    confirmed.priority = NetworkPriority::Urgent;
    // The send's reject origin is none: a late refusal answers no one.
    let _ = router.originate(&confirmed, at(1));
    let Action::Send(send) = router.poll_action() else {
        panic!("the request");
    };
    assert_eq!(send.reject_origin, None);
    assert_eq!(router.poll_action(), Action::Idle);

    let (result, sent) = originate(&mut router, &confirmed, at(1));
    assert_eq!(result, Ok(()));
    assert_eq!(sent.len(), 1);
    let send = &sent[0];
    assert_eq!((send.port, send.to.as_deref()), (1, Some(R7)));
    assert_eq!((send.kind, send.cause), (Kind::Originate, Cause::Local));
    assert_eq!(send.priority, NetworkPriority::Urgent);
    let npdu = send.parsed();
    assert_eq!(npdu.destination, Some(addr(9, &[5])));
    assert_eq!(npdu.source, None, "the next router adds network 2");
    assert_eq!(npdu.hop_count, 255);
    assert!(npdu.expecting_reply);
    assert_eq!(npdu.payload, CONFIRMED);
    assert_eq!(router.counters().originated, 2);
}

#[test]
fn a_selected_port_names_its_address_unless_the_request_leaves_by_it() {
    let mut router = with_9();
    let mut from_0 = request(9, &[5], CONFIRMED);
    from_0.source = SourceSelection::Port(PortId(0));
    let (_, sent) = originate(&mut router, &from_0, at(1));
    assert_eq!(sent[0].parsed().source, Some(addr(1, OWN_0)));
    assert_eq!(sent[0].port, 1);

    let mut from_1 = from_0;
    from_1.source = SourceSelection::Port(PortId(1));
    let (_, sent) = originate(&mut router, &from_1, at(1));
    assert_eq!(sent[0].parsed().source, None, "the link source says it");

    let mut absent = from_0;
    absent.source = SourceSelection::Port(PortId(3));
    let (result, sent) = originate(&mut router, &absent, at(1));
    assert_eq!(result, Err(OriginateError::UnknownPort));
    assert!(sent.is_empty());
}

#[test]
fn onto_a_directly_attached_network_it_goes_to_the_dadr() {
    let mut router = with_9();
    let (result, sent) = originate(&mut router, &request(2, &[5], CONFIRMED), at(1));
    assert_eq!(result, Ok(()));
    assert_eq!((sent[0].port, sent[0].to.as_deref()), (1, Some(&[5][..])));
    let npdu = sent[0].parsed();
    assert_eq!((npdu.destination, npdu.source), (None, None));

    let mut from_0 = request(2, &[], UNCONFIRMED);
    from_0.source = SourceSelection::Port(PortId(0));
    let (result, sent) = originate(&mut router, &from_0, at(1));
    assert_eq!(result, Ok(()));
    assert!(sent[0].is_broadcast());
    assert_eq!(sent[0].parsed().source, Some(addr(1, OWN_0)));
}

#[test]
fn what_it_refuses_sends_nothing() {
    let mut first = with_9();
    let mut busy = router();
    learn_9_via_r7(&mut busy, Instant::ZERO);
    busy.table_mut().mark_busy(9, at(30));
    let mut gone = router();
    learn_9_via_r7(&mut gone, Instant::ZERO);
    gone.table_mut().mark_unreachable(9);
    let long = vec![0x10; 1_495];
    for (router, request, error) in [
        (
            &mut first,
            request(0, &[5], CONFIRMED),
            OriginateError::ReservedNetwork,
        ),
        (
            &mut with_9(),
            request(0xFFFF, &[], UNCONFIRMED),
            OriginateError::ReservedNetwork,
        ),
        (
            &mut with_9(),
            request(1, OWN_0, CONFIRMED),
            OriginateError::OwnAddress,
        ),
        (
            &mut with_9(),
            request(2, OWN_1, CONFIRMED),
            OriginateError::OwnAddress,
        ),
        (
            &mut with_9(),
            request(9, &[], CONFIRMED),
            OriginateError::BroadcastNotUnconfirmed,
        ),
        (&mut busy, request(9, &[5], CONFIRMED), OriginateError::Busy),
        (
            &mut gone,
            request(9, &[5], CONFIRMED),
            OriginateError::Unreachable,
        ),
        (
            &mut with_9(),
            request(9, &[5], &long),
            OriginateError::TooLong {
                octets: 1_502,
                limit: 1_497,
            },
        ),
    ] {
        let (result, sent) = originate(router, &request, at(1));
        assert_eq!(result, Err(error));
        assert!(sent.is_empty(), "{error:?}");
        assert_eq!(router.counters().originated, 0);
    }
}

#[test]
fn no_route_searches_on_every_port_once_per_cooldown() {
    let mut router = with_9();
    let (result, sent) = originate(&mut router, &request(77, &[5], CONFIRMED), at(1));
    assert_eq!(result, Err(OriginateError::NoRoute));
    let ports: Vec<u8> = sent.iter().map(|sent| sent.port).collect();
    assert_eq!(ports, [0, 1], "no arrival port to leave out");
    for send in &sent {
        assert_eq!(send.kind, Kind::Solicit);
        let npdu = send.parsed();
        assert_eq!(
            npdu.message_type,
            Some(NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK)
        );
        assert_eq!(npdu.payload, 77u16.to_be_bytes());
    }
    let (result, sent) = originate(&mut router, &request(77, &[5], CONFIRMED), at(2));
    assert_eq!(result, Err(OriginateError::NoRoute));
    assert!(sent.is_empty(), "within the cooldown");
    let cooldown = Options::HOSTED.discovery_cooldown;
    let later = Instant::from_secs(1) + cooldown + Duration::from_secs(1);
    let (_, sent) = originate(&mut router, &request(77, &[5], CONFIRMED), later);
    assert_eq!(sent.len(), 2);
}

/// The reply to a request from the egress address comes back with no DNET
/// on that port, from the next router, and is the router's own.
#[test]
fn the_reply_reaches_the_routers_application() {
    let mut router = with_9();
    let (result, _) = originate(&mut router, &request(9, &[5], CONFIRMED), at(1));
    assert_eq!(result, Ok(()));
    let mut reply = Npdu::apdu(&[0x30, 0x05, 0x0C]);
    reply.source = Some(addr(9, &[5]));
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 1, R7, &bytes(&reply), at(2));
    assert!(sent.is_empty());
    let Outcome::Local { source, .. } = outcome else {
        panic!("for the router: {outcome:?}");
    };
    assert_eq!(source, Some((9, vec![5])));

    // From a selected port, it comes addressed to that port's address.
    let mut routed = reply;
    routed.destination = Some(addr(1, OWN_0));
    let (outcome, sent) = deliver(&mut router, &mut Permissive, 1, R7, &bytes(&routed), at(2));
    assert!(sent.is_empty());
    assert!(matches!(outcome, Outcome::Local { .. }));
}

#[test]
fn originating_drops_what_an_earlier_call_left_unpolled() {
    let mut router = with_9();
    router.start();
    let _ = router.originate(&request(9, &[5], CONFIRMED), at(1));
    assert!(router.counters().undrained > 0);
    let sent = drain(&mut router);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].kind, Kind::Originate);
}
