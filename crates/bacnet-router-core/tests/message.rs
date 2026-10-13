//! Network-layer message payloads: layouts, whole-or-nothing parsing, and
//! agreement with `bacnet-types` and `bacnet-encoding` where they have the
//! same thing.

use bacnet_encoding::npdu::{decode_npdu, decode_reject_message_to_network};
use bacnet_router_core::message::{
    DataError, DisconnectConnection, EstablishConnection, ICouldBeRouter, MessageType,
    NetworkNumberIs, Networks, Reject, RejectReason, RoutingEntries, RoutingEntry, WhoIsRouter,
};
use bacnet_router_core::npdu::Npdu;
use bacnet_types::enums::{NetworkMessageType, RejectMessageReason};
use bytes::Bytes;
use proptest::prelude::*;

/// The message types the core names, as `ALL_NAMED` lists them: grouped
/// by what a router does with them, by name within a group.
const NAMED_TYPES: [(u8, &str); 12] = [
    (0x01, "I_AM_ROUTER_TO_NETWORK"),
    (0x02, "I_COULD_BE_ROUTER_TO_NETWORK"),
    (0x03, "REJECT_MESSAGE_TO_NETWORK"),
    (0x00, "WHO_IS_ROUTER_TO_NETWORK"),
    (0x05, "ROUTER_AVAILABLE_TO_NETWORK"),
    (0x04, "ROUTER_BUSY_TO_NETWORK"),
    (0x06, "INITIALIZE_ROUTING_TABLE"),
    (0x07, "INITIALIZE_ROUTING_TABLE_ACK"),
    (0x09, "DISCONNECT_CONNECTION_TO_NETWORK"),
    (0x08, "ESTABLISH_CONNECTION_TO_NETWORK"),
    (0x13, "NETWORK_NUMBER_IS"),
    (0x12, "WHAT_IS_NETWORK_NUMBER"),
];

/// The reject reasons the core names, in order: 5 is withdrawn.
const NAMED_REASONS: [(u8, &str); 6] = [
    (0, "OTHER"),
    (1, "NOT_DIRECTLY_CONNECTED"),
    (2, "ROUTER_BUSY"),
    (3, "UNKNOWN_MESSAGE_TYPE"),
    (4, "MESSAGE_TOO_LONG"),
    (6, "ADDRESSING_ERROR"),
];

/// Every octet converts and comes back as a message type; only the twelve
/// named ones have names; 0x80 and up are proprietary and 0x0A to 0x11 the
/// removed security types; Debug and Display print the name, or else the
/// octet in decimal.
#[test]
fn every_message_type_octet_round_trips() {
    let listed: Vec<(u8, &str)> = MessageType::ALL_NAMED
        .iter()
        .map(|&(name, kind)| (kind.to_raw(), name))
        .collect();
    assert_eq!(listed, NAMED_TYPES);
    for raw in 0..=u8::MAX {
        let kind = MessageType::from_raw(raw);
        assert_eq!(kind.to_raw(), raw);
        let name = NAMED_TYPES
            .iter()
            .find(|&&(named, _)| named == raw)
            .map(|&(_, name)| name);
        assert_eq!(kind.name(), name, "{raw}");
        assert_eq!(kind.is_defined(), name.is_some(), "{raw}");
        assert_eq!(kind.is_proprietary(), raw >= 0x80, "{raw}");
        assert_eq!(kind.is_removed_security(), (0x0A..=0x11).contains(&raw));
        match name {
            Some(name) => {
                assert_eq!(format!("{kind:?}"), format!("MessageType::{name}"));
                assert_eq!(kind.to_string(), name);
            }
            None => {
                assert_eq!(format!("{kind:?}"), format!("MessageType({raw})"));
                assert_eq!(kind.to_string(), raw.to_string());
            }
        }
    }
}

/// The same for reject reasons.
#[test]
fn every_reject_reason_octet_round_trips() {
    let listed: Vec<(u8, &str)> = RejectReason::ALL_NAMED
        .iter()
        .map(|&(name, reason)| (reason.to_raw(), name))
        .collect();
    assert_eq!(listed, NAMED_REASONS);
    for raw in 0..=u8::MAX {
        let reason = RejectReason::from_raw(raw);
        assert_eq!(reason.to_raw(), raw);
        let name = NAMED_REASONS
            .iter()
            .find(|&&(named, _)| named == raw)
            .map(|&(_, name)| name);
        assert_eq!(reason.name(), name, "{raw}");
        assert_eq!(reason.is_defined(), name.is_some(), "{raw}");
        match name {
            Some(name) => {
                assert_eq!(format!("{reason:?}"), format!("RejectReason::{name}"));
                assert_eq!(reason.to_string(), name);
            }
            None => {
                assert_eq!(format!("{reason:?}"), format!("RejectReason({raw})"));
                assert_eq!(reason.to_string(), raw.to_string());
            }
        }
    }
}

#[test]
fn codes_and_reasons_match_bacnet_types() {
    let codes = [
        (
            MessageType::WHO_IS_ROUTER_TO_NETWORK,
            NetworkMessageType::WHO_IS_ROUTER_TO_NETWORK,
        ),
        (
            MessageType::I_AM_ROUTER_TO_NETWORK,
            NetworkMessageType::I_AM_ROUTER_TO_NETWORK,
        ),
        (
            MessageType::I_COULD_BE_ROUTER_TO_NETWORK,
            NetworkMessageType::I_COULD_BE_ROUTER_TO_NETWORK,
        ),
        (
            MessageType::REJECT_MESSAGE_TO_NETWORK,
            NetworkMessageType::REJECT_MESSAGE_TO_NETWORK,
        ),
        (
            MessageType::ROUTER_BUSY_TO_NETWORK,
            NetworkMessageType::ROUTER_BUSY_TO_NETWORK,
        ),
        (
            MessageType::ROUTER_AVAILABLE_TO_NETWORK,
            NetworkMessageType::ROUTER_AVAILABLE_TO_NETWORK,
        ),
        (
            MessageType::INITIALIZE_ROUTING_TABLE,
            NetworkMessageType::INITIALIZE_ROUTING_TABLE,
        ),
        (
            MessageType::INITIALIZE_ROUTING_TABLE_ACK,
            NetworkMessageType::INITIALIZE_ROUTING_TABLE_ACK,
        ),
        (
            MessageType::ESTABLISH_CONNECTION_TO_NETWORK,
            NetworkMessageType::ESTABLISH_CONNECTION_TO_NETWORK,
        ),
        (
            MessageType::DISCONNECT_CONNECTION_TO_NETWORK,
            NetworkMessageType::DISCONNECT_CONNECTION_TO_NETWORK,
        ),
        (
            MessageType::WHAT_IS_NETWORK_NUMBER,
            NetworkMessageType::WHAT_IS_NETWORK_NUMBER,
        ),
        (
            MessageType::NETWORK_NUMBER_IS,
            NetworkMessageType::NETWORK_NUMBER_IS,
        ),
    ];
    for (core, types) in codes {
        assert_eq!(core.to_raw(), types.to_raw());
        let named = NetworkMessageType::ALL_NAMED
            .iter()
            .find(|(_, named)| *named == types);
        assert_eq!(core.name(), named.map(|(name, _)| *name));
    }
    // The security types the standard no longer defines: 0x0A to 0x11.
    let removed = [
        NetworkMessageType::CHALLENGE_REQUEST,
        NetworkMessageType::SET_MASTER_KEY,
    ];
    for types in removed {
        let core = MessageType::from_raw(types.to_raw());
        assert!(core.is_removed_security() && !core.is_defined());
    }
    let reasons = [
        (RejectReason::OTHER, RejectMessageReason::OTHER),
        (
            RejectReason::NOT_DIRECTLY_CONNECTED,
            RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        ),
        (RejectReason::ROUTER_BUSY, RejectMessageReason::ROUTER_BUSY),
        (
            RejectReason::UNKNOWN_MESSAGE_TYPE,
            RejectMessageReason::UNKNOWN_MESSAGE_TYPE,
        ),
        (
            RejectReason::MESSAGE_TOO_LONG,
            RejectMessageReason::MESSAGE_TOO_LONG,
        ),
        (
            RejectReason::ADDRESSING_ERROR,
            RejectMessageReason::ADDRESSING_ERROR,
        ),
    ];
    for (core, types) in reasons {
        assert_eq!(core.to_raw(), types.to_raw());
    }
}

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

    /// The reject view takes exactly what bacnet-encoding's does, and writes
    /// it back unchanged.
    #[test]
    fn reject_agrees_with_bacnet_encoding(payload in prop::collection::vec(any::<u8>(), 0..5)) {
        let view = Reject::parse(&payload).ok();
        let decoded = decode_reject_message_to_network(&payload).ok();
        prop_assert_eq!(view.is_some(), decoded.is_some());
        if let (Some(view), Some(decoded)) = (view, decoded) {
            prop_assert_eq!(view.reason.to_raw(), decoded.reason.to_raw());
            prop_assert_eq!(view.network, decoded.dnet);
            prop_assert_eq!(&view.to_bytes()[..], &payload[..]);
        }
    }

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
        assert_eq!(Reject::parse(short), Err(refused));
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
    let reject = Reject {
        reason: RejectReason::MESSAGE_TOO_LONG,
        network: 3000,
    }
    .to_bytes();
    let npdu = Npdu::message(MessageType::REJECT_MESSAGE_TO_NETWORK, &reject);
    let mut out = [0u8; 16];
    let len = npdu.write(&mut out).unwrap();
    let decoded = decode_npdu(Bytes::copy_from_slice(&out[..len])).unwrap();
    assert!(decoded.is_network_message);
    assert_eq!(
        decoded.message_type,
        Some(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK.to_raw())
    );
    let body = decode_reject_message_to_network(&decoded.payload).unwrap();
    assert_eq!(body.reason, RejectMessageReason::MESSAGE_TOO_LONG);
    assert_eq!(body.dnet, 3000);

    // A header with no payload, then the payload written after it.
    let header = Npdu::message(MessageType::I_AM_ROUTER_TO_NETWORK, &[]);
    let at = header.write(&mut out).unwrap();
    let len = at + Networks::write([1, 2], &mut out[at..]).unwrap();
    let decoded = decode_npdu(Bytes::copy_from_slice(&out[..len])).unwrap();
    assert_eq!(
        decoded.payload,
        Bytes::from_static(&[0x00, 0x01, 0x00, 0x02])
    );
}
