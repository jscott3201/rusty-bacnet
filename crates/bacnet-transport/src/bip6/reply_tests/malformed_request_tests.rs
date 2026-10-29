//! A malformed Register-Foreign-Device, Delete-Foreign-Device-Table-Entry or
//! Distribute-Broadcast-To-Network, one whose payload doesn't fit its
//! function, gets the NAK a well-formed one gets (#1638): from this node's
//! VMAC, at the datagram's source, under the same guards and the same quota,
//! and counted once as a malformed payload. Annex U doesn't ask for it; B/IP
//! does the same.

use super::super::frame::raw_frame;
use super::*;

/// The request each case malforms, its payload, and the NAK it draws. A
/// registration holds exactly a VMAC and a time-to-live, an FDT entry
/// deletion exactly a VMAC and an 18-octet address, and a broadcast
/// distribution at least a VMAC.
fn malformed_requests() -> [(Bvlc6Function, Vec<u8>, Bvlc6ResultCode); 6] {
    let vmac = ASKER_VMAC.octets();
    let with = |extra: &[u8]| [&vmac[..], extra].concat();
    let register = Bvlc6Function::REGISTER_FOREIGN_DEVICE;
    let delete = Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY;
    let distribute = Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK;
    let register_nak = Bvlc6ResultCode::REGISTER_FOREIGN_DEVICE_NAK;
    let delete_nak = Bvlc6ResultCode::DELETE_FOREIGN_DEVICE_TABLE_ENTRY_NAK;
    let distribute_nak = Bvlc6ResultCode::DISTRIBUTE_BROADCAST_TO_NETWORK_NAK;
    [
        (register, with(&[0x3C]), register_nak),
        (register, with(&[0x00, 0x3C, 0x00]), register_nak),
        (delete, with(&[0x20; 17]), delete_nak),
        (delete, with(&[0x20; 19]), delete_nak),
        (distribute, Vec::new(), distribute_nak),
        (distribute, vmac[..2].to_vec(), distribute_nak),
    ]
}

/// What `receiver` replies to `frame` from `peer`, sent to `destination`.
async fn reply_to(
    receiver: &Receiver,
    frame: &[u8],
    peer: SocketAddrV6,
    destination: Ipv6Addr,
) -> Option<Reply> {
    receiver
        .handle_datagram(frame, &datagram(frame, peer, destination))
        .await
}

/// Counts with `malformed` malformed payloads and `suppressed` NAKs held
/// back, and nothing else.
fn malformed_counts(malformed: u64, suppressed: u64) -> Bip6ReceiveCounters {
    Bip6ReceiveCounters {
        malformed_payload: malformed,
        naks_suppressed: suppressed,
        ..Bip6ReceiveCounters::default()
    }
}

/// Each malformed request gets its own NAK at its source, in either mode.
/// The octets are written out from U.2.1.1.
#[tokio::test]
async fn each_malformed_bbmd_request_gets_its_nak_at_its_source() {
    for foreign in [false, true] {
        let receiver = receiver(foreign).await;
        let requests = malformed_requests();
        for (function, payload, code) in &requests {
            let frame = raw_frame(function.to_raw(), payload);
            assert!(decode_bvll6(&frame).is_err(), "{function} is malformed");
            let nak = reply_to(&receiver, &frame, sender(), local_ip()).await;
            let expected = Reply {
                to: sender().into(),
                message: Bvll6::BvlcResult {
                    source: LOCAL_VMAC,
                    code: *code,
                },
            };
            assert_eq!(nak, Some(expected), "{function} foreign={foreign}");
            let [high, low] = match *function {
                Bvlc6Function::REGISTER_FOREIGN_DEVICE => [0x00, 0x90],
                Bvlc6Function::DELETE_FOREIGN_DEVICE_TABLE_ENTRY => [0x00, 0xA0],
                Bvlc6Function::DISTRIBUTE_BROADCAST_TO_NETWORK => [0x00, 0xC0],
                other => unreachable!("{other} is not a BBMD request"),
            };
            let wire = encoded(&expected.message).unwrap();
            assert_eq!(
                wire[..],
                [0x82, 0x00, 0x00, 0x09, 0x00, 0x06, 0x32, high, low]
            );
        }
        let counted = u64::try_from(requests.len()).unwrap();
        assert_eq!(receiver.drops.snapshot(), malformed_counts(counted, 0));
    }
}

/// The guards a well-formed request passes hold for a malformed one: no NAK
/// for one sent to a group, or from a source no node sends from, or from
/// this node's own address. Each is still counted once, as malformed.
#[tokio::test]
async fn a_malformed_request_draws_no_nak_where_a_well_formed_one_would_not() {
    let receiver = receiver(false).await;
    let own = SocketAddrV6::new(local_ip(), LOCAL_PORT, 0, 0);
    let mut sources = unanswerable_sources().to_vec();
    sources.push(own);
    let mut refused = 0;
    for (function, payload, _) in malformed_requests() {
        let frame = raw_frame(function.to_raw(), &payload);
        let group = BACNET_IPV6_MULTICAST_SITE_LOCAL;
        assert_eq!(reply_to(&receiver, &frame, sender(), group).await, None);
        let mut flagged = datagram(&frame, sender(), local_ip());
        flagged.os_group_delivery = Some(true);
        assert_eq!(receiver.handle_datagram(&frame, &flagged).await, None);
        refused += 2;
        for &source in &sources {
            let nak = reply_to(&receiver, &frame, source, local_ip()).await;
            assert_eq!(nak, None, "{function} from {source}");
            refused += 1;
        }
    }
    assert_eq!(receiver.drops.snapshot(), malformed_counts(refused, 0));
}

/// Malformed and well-formed requests share one NAK quota, per address and
/// per window; a NAK held back is counted as suppressed.
#[tokio::test(start_paused = true)]
async fn malformed_requests_count_against_the_nak_quota() {
    let receiver = receiver(true).await;
    let (function, payload, _) = &malformed_requests()[0];
    let frame = raw_frame(function.to_raw(), payload);
    for _ in 0..REPLIES_PER_DESTINATION {
        let nak = reply_to(&receiver, &frame, sender(), local_ip()).await;
        assert!(nak.is_some());
    }
    assert_eq!(
        reply(&receiver, register(), sender(), local_ip()).await,
        None,
        "the malformed requests spent the quota"
    );
    assert_eq!(
        reply_to(&receiver, &frame, sender(), local_ip()).await,
        None
    );
    let malformed = u64::from(REPLIES_PER_DESTINATION) + 1;
    assert_eq!(receiver.drops.snapshot(), malformed_counts(malformed, 2));

    assert!(
        reply_to(&receiver, &frame, asker(), local_ip())
            .await
            .is_some(),
        "another address has its own"
    );
    tokio::time::advance(REPLY_WINDOW).await;
    assert!(
        reply_to(&receiver, &frame, sender(), local_ip())
            .await
            .is_some()
    );
}

/// Only the three requests a BBMD serves are NAKed when malformed. A
/// resolution request or ACK that doesn't decode gets nothing, though two
/// of the requests have NAK codes, and neither does a malformed BVLC-Result.
#[tokio::test]
async fn other_malformed_messages_get_no_reply() {
    let receiver = receiver(false).await;
    let vmac = ASKER_VMAC.octets();
    let group = BACNET_IPV6_MULTICAST_LINK_LOCAL;
    let cases = [
        (Bvlc6Function::ADDRESS_RESOLUTION, group),
        (Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION_ACK, local_ip()),
        (Bvlc6Function::FORWARDED_ADDRESS_RESOLUTION, group),
        (Bvlc6Function::BVLC_RESULT, local_ip()),
    ];
    for (function, destination) in cases {
        let frame = raw_frame(function.to_raw(), &vmac);
        let answer = reply_to(&receiver, &frame, sender(), destination).await;
        assert_eq!(answer, None, "{function}");
    }
    let virtual_resolution = raw_frame(
        Bvlc6Function::VIRTUAL_ADDRESS_RESOLUTION.to_raw(),
        &[&vmac[..], &[0x00]].concat(),
    );
    let answer = reply_to(&receiver, &virtual_resolution, sender(), local_ip()).await;
    assert_eq!(answer, None);
    assert_eq!(receiver.drops.snapshot(), malformed_counts(5, 0));
}

/// End to end on loopback: a running transport NAKs a malformed registration
/// and a malformed broadcast distribution at the requesting socket.
#[tokio::test]
async fn a_running_transport_naks_a_malformed_request() {
    let mut transport = Bip6Transport::new(Ipv6Addr::LOCALHOST, 0, Some(0x0632));
    let _rx = transport.start().await.unwrap();
    let port = transport.local_address().unwrap().port();
    let to = SocketAddrV6::new(Ipv6Addr::LOCALHOST, port, 0, 0);
    let requester = UdpSocket::bind("[::1]:0").await.unwrap();
    let mut buf = [0u8; 64];
    for index in [0, 4] {
        let (function, payload, code) = &malformed_requests()[index];
        let frame = raw_frame(function.to_raw(), payload);
        requester.send_to(&frame, to).await.unwrap();
        let (len, from) =
            tokio::time::timeout(Duration::from_secs(2), requester.recv_from(&mut buf))
                .await
                .expect("the NAK arrives")
                .unwrap();
        assert_eq!(from, SocketAddr::V6(to));
        assert_eq!(
            decode_bvll6(&buf[..len]).unwrap(),
            Bvll6::BvlcResult {
                source: LOCAL_VMAC,
                code: *code,
            }
        );
    }
    assert_eq!(transport.receive_counters(), malformed_counts(2, 0));
    transport.stop().await.unwrap();
}
