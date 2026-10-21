//! Network-layer message payloads: layouts, whole-or-nothing parsing, and
//! messages the router writes decoding in `bacnet-encoding`.

use bacnet_encoding::npdu::decode_npdu;
use bacnet_router_core::message::{
    DataError, DisconnectConnection, EstablishConnection, ICouldBeRouter, NetworkNumberIs,
    Networks, RoutingEntries, RoutingEntry, WhoIsRouter,
};
use bacnet_router_core::npdu::Npdu;
use bacnet_router_core::npdu::{NetworkMessageType, RejectMessageReason, RejectMessageToNetwork};
use bytes::Bytes;
use proptest::prelude::*;

/// An independent walk of a routing-table payload, as the hosted router's
/// tests make it: the count, then each entry, with nothing left over.
fn peer_entries(payload: &[u8]) -> Option<Vec<(u16, u8, Vec<u8>)>> {
    let (&count, mut rest) = payload.split_first()?;
    let mut entries = Vec::new();
    for _ in 0..count {
        if rest.len() < 4 {
            return None;
        }
        let info_len = usize::from(rest[3]);
        let info = rest.get(4..4 + info_len)?.to_vec();
        entries.push((u16::from_be_bytes([rest[0], rest[1]]), rest[2], info));
        rest = &rest[4 + info_len..];
    }
    rest.is_empty().then_some(entries)
}

/// A routing-table payload: usually a count and entries that agree, any of
/// it cut short or run on.
fn routing_payload() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::collection::vec(
            (
                any::<u16>(),
                any::<u8>(),
                prop::collection::vec(any::<u8>(), 0..4),
            ),
            0..6,
        ),
        prop_oneof![3 => Just(None), 1 => any::<u8>().prop_map(Some)],
        prop::collection::vec(any::<u8>(), 0..3),
        any::<bool>(),
    )
        .prop_map(|(entries, count, tail, run_on)| {
            let mut payload = vec![count.unwrap_or(entries.len() as u8)];
            for (network, port_id, info) in entries {
                payload.extend_from_slice(&network.to_be_bytes());
                payload.extend_from_slice(&[port_id, info.len() as u8]);
                payload.extend_from_slice(&info);
            }
            if run_on {
                payload.extend_from_slice(&tail);
            }
            payload
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// The routing-table view accepts what the independent walk accepts,
    /// reads the same entries, and writes them back as it read them when
    /// they carry no Port Info.
    #[test]
    fn routing_entries_agree_with_an_independent_walk(payload in routing_payload()) {
        let view = RoutingEntries::parse(&payload);
        let walked = peer_entries(&payload);
        prop_assert_eq!(view.is_ok(), walked.is_some());
        if view.is_err() {
            prop_assert_eq!(view, Err(DataError::Length { length: payload.len() }));
        }
        let view = view.ok();
        let (Some(view), Some(walked)) = (view, walked) else {
            return Ok(());
        };
        prop_assert_eq!(usize::from(view.count()), walked.len());
        let read: Vec<_> = view
            .iter()
            .map(|entry| (entry.network, entry.port_id, entry.port_info.to_vec()))
            .collect();
        prop_assert_eq!(&read, &walked);
        if walked.iter().all(|(_, _, info)| info.is_empty()) {
            let mut out = vec![0u8; payload.len()];
            let len = RoutingEntries::write(
                walked.iter().map(|(network, port_id, _)| (*network, *port_id)),
                &mut out,
            );
            prop_assert_eq!(len, Ok(payload.len()));
            prop_assert_eq!(out, payload);
        }
    }

    /// A network list reads back what was written, and an odd length is no
    /// list at all.
    #[test]
    fn networks_round_trip(networks in prop::collection::vec(any::<u16>(), 0..8), odd in any::<u8>()) {
        let mut out = vec![0u8; networks.len() * 2];
        prop_assert_eq!(Networks::write(networks.iter().copied(), &mut out), Ok(out.len()));
        let view = Networks::parse(&out).unwrap();
        prop_assert_eq!(view.len(), networks.len());
        prop_assert_eq!(view.is_empty(), networks.is_empty());
        prop_assert_eq!(view.as_bytes(), &out[..]);
        prop_assert_eq!(view.iter().collect::<Vec<_>>(), networks.clone());
        for network in &networks {
            prop_assert!(view.contains(*network));
        }
        out.push(odd);
        let length = out.len();
        prop_assert_eq!(Networks::parse(&out), Err(DataError::Length { length }));
    }
}

#[test]
fn writers_refuse_what_does_not_fit() {
    // A refusal writes nothing at all, not even the part that would fit.
    let untouched = |out: &[u8]| out.iter().all(|&octet| octet == 0xA5);
    let no_room = |needed, available| Err(DataError::NoRoom { needed, available });
    let mut out = [0xA5u8; 5];
    assert_eq!(Networks::write([1, 2, 3], &mut out), no_room(6, 5));
    assert!(untouched(&out));
    assert_eq!(
        RoutingEntries::write([(1, 1), (2, 1)], &mut out),
        no_room(9, 5)
    );
    assert!(untouched(&out));
    assert_eq!(RoutingEntries::write([], &mut []), no_room(1, 0));

    assert_eq!(Networks::write([1, 2], &mut out), Ok(4));
    assert_eq!(out, [0x00, 0x01, 0x00, 0x02, 0xA5]);
    assert_eq!(RoutingEntries::write([(1, 1)], &mut out), Ok(5));
    assert_eq!(out, [1, 0x00, 0x01, 1, 0]);
    assert_eq!(RoutingEntries::write([], &mut out[..1]), Ok(1));
    assert_eq!(out[0], 0);

    // The count octet names at most 255 entries.
    let mut big = vec![0xA5u8; 1 + 256 * RoutingEntries::ENTRY_LEN];
    assert_eq!(
        RoutingEntries::write((0..256).map(|n| (n, 1)), &mut big),
        Err(DataError::TooManyEntries { count: 256 })
    );
    assert!(untouched(&big));
    let len = RoutingEntries::write((0..255).map(|n| (n, 1)), &mut big).unwrap();
    assert_eq!(RoutingEntries::parse(&big[..len]).unwrap().count(), 255);
}

#[test]
fn routing_entries_tell_a_query_from_an_update_ack() {
    let query = RoutingEntries::parse(&[0]).unwrap();
    assert_eq!(query.count(), 0);
    assert_eq!(query.iter().count(), 0);
    // An Ack to an update has no data at all, not even a count.
    assert_eq!(
        RoutingEntries::parse(&[]),
        Err(DataError::Length { length: 0 })
    );
    let entries = RoutingEntries::parse(&[1, 0x0B, 0xB8, 0x02, 0x01, 0xAA]).unwrap();
    assert_eq!(
        entries.iter().collect::<Vec<_>>(),
        [RoutingEntry {
            network: 3000,
            port_id: 2,
            port_info: &[0xAA],
        }]
    );
}

#[test]
fn fixed_payloads_take_exactly_their_length() {
    let wrong = |payload: &[u8]| DataError::Length {
        length: payload.len(),
    };
    assert_eq!(WhoIsRouter::parse(&[]), Ok(WhoIsRouter::All));
    assert_eq!(
        WhoIsRouter::parse(&[0x0B, 0xB8]),
        Ok(WhoIsRouter::Network(3000))
    );
    for bad in [&[0x0B][..], &[0x0B, 0xB8, 0x00]] {
        // A bad tail never widens a scoped question to every network.
        assert_eq!(WhoIsRouter::parse(bad), Err(wrong(bad)));
    }

    assert_eq!(
        ICouldBeRouter::parse(&[0x0B, 0xB8, 7]),
        Ok(ICouldBeRouter {
            network: 3000,
            performance_index: 7,
        })
    );
    assert_eq!(
        EstablishConnection::parse(&[0x0B, 0xB8, 0]),
        Ok(EstablishConnection {
            network: 3000,
            termination_time: 0,
        })
    );
    assert_eq!(
        DisconnectConnection::parse(&[0x0B, 0xB8]),
        Ok(DisconnectConnection { network: 3000 })
    );
    let number = NetworkNumberIs::parse(&[0x00, 0x05, 1]).unwrap();
    assert_eq!((number.network, number.is_configured()), (5, true));
    assert_eq!(number.to_bytes(), [0x00, 0x05, 1]);
    assert!(!NetworkNumberIs::parse(&[0x00, 0x05, 0])
        .unwrap()
        .is_configured());

    for short in [&[][..], &[0x0B], &[0x0B, 0xB8], &[0x0B, 0xB8, 1, 2]] {
        let refused = wrong(short);
        assert_eq!(ICouldBeRouter::parse(short), Err(refused));
        assert_eq!(EstablishConnection::parse(short), Err(refused));
        assert_eq!(NetworkNumberIs::parse(short), Err(refused));
    }
    for bad in [&[][..], &[0x0B], &[0x0B, 0xB8, 0]] {
        assert_eq!(DisconnectConnection::parse(bad), Err(wrong(bad)));
    }
}

#[test]
fn data_errors_say_what_was_wrong() {
    use std::string::ToString;
    assert_eq!(
        DataError::Length { length: 2 }.to_string(),
        "2 octets of message data don't fit its layout"
    );
    assert_eq!(
        DataError::NoRoom {
            needed: 6,
            available: 5
        }
        .to_string(),
        "message data of 6 octets doesn't fit in 5"
    );
    assert_eq!(
        DataError::TooManyEntries { count: 256 }.to_string(),
        "256 routing entries, more than a count can state"
    );
}

#[test]
fn messages_written_in_an_npdu_decode_in_bacnet_encoding() {
    let reject = RejectMessageToNetwork {
        reason: RejectMessageReason::MESSAGE_TOO_LONG,
        network: 3000,
    }
    .to_bytes();
    let npdu = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &reject);
    let mut out = [0u8; 16];
    let len = npdu.encode_into(&mut out).unwrap();
    let decoded = decode_npdu(Bytes::copy_from_slice(&out[..len])).unwrap();
    assert!(decoded.is_network_message());
    assert_eq!(
        decoded.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK)
    );
    let body = RejectMessageToNetwork::decode(&decoded.payload).unwrap();
    assert_eq!(body.reason, RejectMessageReason::MESSAGE_TOO_LONG);
    assert_eq!(body.network, 3000);

    // A header with no payload, then the payload written after it.
    let header = Npdu::message(NetworkMessageType::I_AM_ROUTER_TO_NETWORK, &[]);
    let at = header.encode_into(&mut out).unwrap();
    let len = at + Networks::write([1, 2], &mut out[at..]).unwrap();
    let decoded = decode_npdu(Bytes::copy_from_slice(&out[..len])).unwrap();
    assert_eq!(
        decoded.payload,
        Bytes::from_static(&[0x00, 0x01, 0x00, 0x02])
    );
}
