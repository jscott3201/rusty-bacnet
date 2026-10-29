//! Global broadcasts from the router's own application (#1809; Clauses
//! 6.2.3, 6.3.2, 6.5.3, 6.5.4): one send per port, each that link's
//! broadcast, still global with a full hop count, from the address the
//! request selects.

mod fixture;

use bacnet_router_core::PortId;
use bacnet_router_core::npdu::{Address, GLOBAL_NETWORK, Mac, NetworkPriority, Npdu, NpduLimits};
use bacnet_router_core::router::{
    Action, Cause, Groups, Kind, Options, OriginateError, PortConfig, Request, SourceSelection,
};
use fixture::{
    CONFIRMED, OWN_0, OWN_1, Sent, TestRouter, UNCONFIRMED, addr, at, bytes, drain, port_on,
    router_with,
};

/// The router's MAC on the third port.
const OWN_2: &[u8] = &[0x03];

/// Three ports: network 1 at a six-octet MAC, 2 and 3 at one-octet MACs.
fn three_ports() -> [PortConfig; 3] {
    [port_on(1, OWN_0), port_on(2, OWN_1), port_on(3, OWN_2)]
}

fn router_on(ports: &[PortConfig]) -> TestRouter {
    router_with(ports, Options::HOSTED)
}

/// A global Who-Is at Urgent priority, from `source`.
fn who_is(source: SourceSelection) -> Request<'static> {
    global(UNCONFIRMED, source)
}

/// A global broadcast of `apdu` at Urgent priority, from `source`.
fn global(apdu: &[u8], source: SourceSelection) -> Request<'_> {
    let destination = Address {
        network: GLOBAL_NETWORK,
        mac: &[],
    };
    let mut request = Request::new(destination, apdu);
    request.priority = NetworkPriority::Urgent;
    request.source = source;
    request
}

/// What the router should write for the global Who-Is with `source` as
/// its SNET and SADR.
fn expected(source: Option<Address<'_>>) -> Vec<u8> {
    expected_of(UNCONFIRMED, source)
}

/// The same for a global broadcast of `apdu`.
fn expected_of(apdu: &[u8], source: Option<Address<'_>>) -> Vec<u8> {
    bytes(&Npdu {
        priority: NetworkPriority::Urgent,
        destination: Some(addr(GLOBAL_NETWORK, &[])),
        source,
        ..Npdu::apdu(apdu)
    })
}

/// Every send is the router's own broadcast, at the request's priority.
fn assert_own_broadcasts(sent: &[Sent]) {
    for send in sent {
        assert!(send.is_broadcast(), "port {}", send.port);
        assert_eq!((send.kind, send.cause), (Kind::Originate, Cause::Local));
        assert_eq!(send.priority, NetworkPriority::Urgent);
        let npdu = send.parsed();
        assert_eq!(npdu.destination, Some(addr(GLOBAL_NETWORK, &[])));
        assert_eq!(npdu.hop_count, 255, "a full count, so routers pass it on");
        assert_eq!(npdu.payload, UNCONFIRMED);
    }
}

#[test]
fn from_the_egress_address_each_port_gets_the_bare_broadcast() {
    let mut router = router_on(&three_ports());
    assert_eq!(
        router.originate(&who_is(SourceSelection::EgressPort), at(1)),
        Ok(())
    );
    let sent = drain(&mut router);
    let ports: Vec<u8> = sent.iter().map(|send| send.port).collect();
    assert_eq!(ports, [0, 1, 2], "every port, none left out");
    assert_own_broadcasts(&sent);
    for send in &sent {
        assert_eq!(send.npdu, expected(None), "port {}", send.port);
    }
    assert_eq!(router.counters().originated, 1);
}

#[test]
fn from_a_selected_port_the_others_name_it_and_it_carries_no_source() {
    let ports = three_ports();
    for (selected, mac) in [(0u8, OWN_0), (1, OWN_1), (2, OWN_2)] {
        let mut router = router_on(&ports);
        let network = u16::from(selected) + 1;
        let request = who_is(SourceSelection::Port(PortId(selected)));
        assert_eq!(router.originate(&request, at(1)), Ok(()), "port {selected}");
        let sent = drain(&mut router);
        assert_own_broadcasts(&sent);
        // The others first, then the selected port, last.
        let order: Vec<u8> = sent.iter().map(|send| send.port).collect();
        let mut others: Vec<u8> = (0..3).filter(|&port| port != selected).collect();
        others.push(selected);
        assert_eq!(order, others, "port {selected}");
        for send in &sent[..2] {
            assert_eq!(
                send.npdu,
                expected(Some(addr(network, mac))),
                "port {selected}: the copy on port {}",
                send.port
            );
        }
        assert_eq!(
            sent[2].npdu,
            expected(None),
            "port {selected}: its own copy, where the link source says it"
        );
        assert_eq!(router.counters().originated, 1);
    }
}

#[test]
fn a_one_port_router_sends_one_bare_broadcast() {
    let mut router = router_on(&[port_on(1, OWN_0)]);
    for source in [
        SourceSelection::EgressPort,
        SourceSelection::Port(PortId(0)),
    ] {
        assert_eq!(router.originate(&who_is(source), at(1)), Ok(()));
        let sent = drain(&mut router);
        assert_eq!(sent.len(), 1, "{source:?}");
        assert_eq!((sent[0].port, &sent[0].npdu), (0, &expected(None)));
    }
}

/// A port whose broadcast limit is `broadcast`, on network 9.
fn narrow(broadcast: u16) -> PortConfig {
    let limits = NpduLimits {
        unicast: 1497,
        broadcast,
    };
    PortConfig::new(9, Mac::from_slice(&[9]).unwrap(), limits, Groups::None)
}

#[test]
fn it_must_fit_every_port_by_the_copy_that_port_gets() {
    // An Unconfirmed-Request long enough for the narrowest limit a port
    // may have.
    let mut long = vec![0; 100];
    long[0] = UNCONFIRMED[0];
    let bare = expected_of(&long, None).len();
    let sourced = expected_of(&long, Some(addr(1, OWN_0))).len();
    // From port 0, the narrow port 1 gets SNET 1 and a six-octet SADR.
    for (limit, fits) in [(sourced, true), (sourced - 1, false)] {
        let mut router = router_on(&[port_on(1, OWN_0), narrow(limit as u16)]);
        let result = router.originate(&global(&long, SourceSelection::Port(PortId(0))), at(1));
        let sent = drain(&mut router);
        if fits {
            assert_eq!(result, Ok(()));
            assert_eq!(sent.len(), 2);
        } else {
            let octets = sourced;
            assert_eq!(result, Err(OriginateError::TooLong { octets, limit }));
            assert!(sent.is_empty(), "refused whole");
            assert_eq!(router.counters().egress_too_long, 1);
            assert_eq!(router.counters().originated, 0);
        }
    }
    // From the narrow port itself, its own copy is the bare one.
    for (limit, fits) in [(bare, true), (bare - 1, false)] {
        let mut router = router_on(&[port_on(1, OWN_0), narrow(limit as u16)]);
        let result = router.originate(&global(&long, SourceSelection::Port(PortId(1))), at(1));
        let sent = drain(&mut router);
        assert_eq!(result.is_ok(), fits, "limit {limit}");
        assert_eq!(sent.len(), if fits { 2 } else { 0 }, "limit {limit}");
        if fits {
            assert_eq!(sent[1].npdu, expected_of(&long, None));
        }
    }
}

#[test]
fn what_a_global_request_may_not_be() {
    let ports = three_ports();
    let with_dadr = Request::new(addr(GLOBAL_NETWORK, &[5]), UNCONFIRMED);
    let confirmed = Request::new(addr(GLOBAL_NETWORK, &[]), CONFIRMED);
    let absent = who_is(SourceSelection::Port(PortId(3)));
    for (request, error) in [
        (with_dadr, OriginateError::GlobalWithDadr),
        (confirmed, OriginateError::BroadcastNotUnconfirmed),
        (absent, OriginateError::UnknownPort),
    ] {
        let mut router = router_on(&ports);
        assert_eq!(router.originate(&request, at(1)), Err(error));
        assert!(drain(&mut router).is_empty(), "{error:?}");
        assert_eq!(router.counters().originated, 0);
    }
}

/// The bare copy's control octet comes from the encoder: it keeps the
/// request's priority and expecting-reply flag, and drops only the SNET
/// flag.
#[test]
fn the_bare_copy_keeps_the_requests_control_bits() {
    let mut router = router_on(&three_ports());
    let mut request = who_is(SourceSelection::Port(PortId(0)));
    request.priority = NetworkPriority::CriticalEquipment;
    request.expecting_reply = true;
    assert_eq!(router.originate(&request, at(1)), Ok(()));
    let sent = drain(&mut router);
    let bare = Npdu {
        priority: NetworkPriority::CriticalEquipment,
        expecting_reply: true,
        destination: Some(addr(GLOBAL_NETWORK, &[])),
        ..Npdu::apdu(UNCONFIRMED)
    };
    assert_eq!(sent[2].port, 0);
    assert_eq!(sent[2].npdu, bytes(&bare));
    // Version, then the control octet: DNET present, expecting a reply,
    // critical-equipment priority.
    assert_eq!(sent[2].npdu[1], 0x20 | 0x04 | 0x02);
    for send in &sent[..2] {
        let npdu = send.parsed();
        assert!(npdu.expecting_reply, "port {}", send.port);
        assert_eq!(npdu.priority, NetworkPriority::CriticalEquipment);
    }
}

/// A port limit lowered between the call and the tail's poll leaves the
/// tail out, counted, as any fan-out send that no longer fits.
#[test]
fn a_limit_lowered_before_the_tail_is_polled_leaves_it_out() {
    let mut long = vec![0; 100];
    long[0] = UNCONFIRMED[0];
    let mut router = router_on(&[port_on(1, OWN_0), port_on(2, OWN_1)]);
    let request = global(&long, SourceSelection::Port(PortId(1)));
    assert_eq!(router.originate(&request, at(1)), Ok(()));
    let Action::Send(first) = router.poll_action() else {
        panic!("port 0's copy");
    };
    assert_eq!(first.port, PortId(0));
    let narrow = NpduLimits {
        unicast: 1497,
        broadcast: 64,
    };
    router.set_port_max_npdu(PortId(1), narrow).unwrap();
    assert_eq!(router.poll_action(), Action::Idle);
    assert_eq!(router.counters().egress_too_long, 1);
}
