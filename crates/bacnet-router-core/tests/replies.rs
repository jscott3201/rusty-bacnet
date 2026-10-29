//! Explicit reply identities, independent byte fixtures, and scratch lifetime.
mod fixture;
use bacnet_router_core::npdu::{Address, EncodeError, NetworkPriority, Npdu};
use bacnet_router_core::router::{Action, Frame, Permissive, Received, ReplyPlan, ReplyPlanError};
use bacnet_router_core::{Instant, PortId};
use fixture::{NODE_A, OWN_0, OWN_1, router};

#[test]
fn r7_owned_reply_keeps_target_return_peer_and_remote_origin_after_scratch_reuse() {
    let mut router = router();
    let request = [1, 0x2c, 0, 2, 1, 1, 0, 9, 2, 7, 8, 255, 0, 5, 0x37, 12];
    let Received::Local(local) = router.receive(
        Frame {
            port: PortId(0),
            source: NODE_A,
            link_group: false,
            npdu: &request,
        },
        Instant::ZERO,
        &mut Permissive,
    ) else {
        panic!("local")
    };
    assert_eq!(local.apdu.as_ptr(), request[12..].as_ptr());
    let reply = local.reply;
    assert_eq!(reply.port(), PortId(0));
    assert_eq!(reply.peer(), NODE_A);
    assert_eq!(
        reply.source(),
        Some(Address {
            network: 2,
            mac: OWN_1
        })
    );
    assert_eq!(
        reply.destination(),
        Some(Address {
            network: 9,
            mac: &[7, 8]
        })
    );
    router.start();
    while !matches!(router.poll_action(), Action::Idle) {}
    drop(router);
    let mut output = [0; 32];
    let prepared = reply.npdu(&[0x20, 0x37, 12], false, NetworkPriority::CriticalEquipment);
    assert_eq!(prepared.encoded_len(), 15);
    let length = prepared.encode_into(&mut output).unwrap();
    assert_eq!(
        &output[..length],
        &[1, 0x2a, 0, 9, 2, 7, 8, 0, 2, 1, 1, 255, 0x20, 0x37, 12]
    );
}

#[test]
fn r7_no_destination_and_groups_have_no_specific_reply_source_but_own_unicasts_do() {
    let mut router = router();
    let cases = [
        (vec![1, 0, 0x10, 8], false, None),
        (vec![1, 0, 0x10, 8], true, None),
        (vec![1, 0x20, 0xff, 0xff, 0, 255, 0x10, 8], false, None),
        (vec![1, 0x20, 0, 1, 0, 255, 0x10, 8], false, None),
        (
            {
                let mut bytes = vec![1, 0x20, 0, 1, 6];
                bytes.extend(OWN_0);
                bytes.extend([255, 0, 5, 7, 12]);
                bytes
            },
            false,
            Some((1, OWN_0)),
        ),
        (
            vec![1, 0x20, 0, 2, 1, 1, 255, 0, 5, 7, 12],
            false,
            Some((2, OWN_1)),
        ),
    ];
    for (bytes, link_group, expected) in cases {
        let Received::Local(local) = router.receive(
            Frame {
                port: PortId(0),
                source: NODE_A,
                link_group,
                npdu: &bytes,
            },
            Instant::ZERO,
            &mut Permissive,
        ) else {
            panic!("local")
        };
        assert_eq!(local.reply.source().map(|a| (a.network, a.mac)), expected);
        while !matches!(router.poll_action(), Action::Idle) {}
    }
}

#[test]
fn r7_explicit_invalid_source_is_refused_and_short_output_is_atomic() {
    for (network, mac, error) in [
        (
            0,
            &[1][..],
            ReplyPlanError::Npdu(EncodeError::ReservedNetwork),
        ),
        (
            65535,
            &[1][..],
            ReplyPlanError::Npdu(EncodeError::ReservedNetwork),
        ),
        (2, &[][..], ReplyPlanError::Npdu(EncodeError::EmptySource)),
    ] {
        assert_eq!(
            ReplyPlan::new(PortId(0), NODE_A, Some(Address { network, mac }), None),
            Err(error)
        );
    }
    for (network, mac, error) in [
        (
            0,
            &[1][..],
            ReplyPlanError::Npdu(EncodeError::ReservedNetwork),
        ),
        (
            65535,
            &[1][..],
            ReplyPlanError::Npdu(EncodeError::ReservedNetwork),
        ),
        (2, &[][..], ReplyPlanError::EmptyDestination),
    ] {
        assert_eq!(
            ReplyPlan::new(PortId(0), NODE_A, None, Some(Address { network, mac })),
            Err(error)
        );
    }
    let plan = ReplyPlan::new(
        PortId(0),
        NODE_A,
        Some(Address {
            network: 2,
            mac: OWN_1,
        }),
        None,
    )
    .unwrap();
    let prepared = plan.npdu(&[0x20, 7, 12], false, NetworkPriority::Normal);
    let mut out = [0xa5; 8];
    assert!(matches!(
        prepared.encode_into(&mut out),
        Err(EncodeError::NoRoom {
            needed: 9,
            available: 8
        })
    ));
    assert_eq!(out, [0xa5; 8]);
    // Independent parser confirms the source field, not merely the plan getter.
    let mut out = [0; 9];
    prepared.encode_into(&mut out).unwrap();
    assert_eq!(Npdu::decode(&out).unwrap().source.unwrap().network, 2);
}
