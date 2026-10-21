//! The network-layer wire codes: every octet converts, and only the
//! values the standard defines have names.

use bacnet_npdu::{NetworkMessageType, RejectMessageReason};

/// The message types the crate names, as `ALL_NAMED` lists them: grouped
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

/// The reject reasons the crate names, in order: 5 is withdrawn.
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
    let listed: Vec<(u8, &str)> = NetworkMessageType::ALL_NAMED
        .iter()
        .map(|&(name, kind)| (kind.to_raw(), name))
        .collect();
    assert_eq!(listed, NAMED_TYPES);
    for raw in 0..=u8::MAX {
        let kind = NetworkMessageType::from_raw(raw);
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
                assert_eq!(format!("{kind:?}"), format!("NetworkMessageType::{name}"));
                assert_eq!(kind.to_string(), name);
            }
            None => {
                assert_eq!(format!("{kind:?}"), format!("NetworkMessageType({raw})"));
                assert_eq!(kind.to_string(), raw.to_string());
            }
        }
    }
}

/// The same for reject reasons.
#[test]
fn every_reject_reason_octet_round_trips() {
    let listed: Vec<(u8, &str)> = RejectMessageReason::ALL_NAMED
        .iter()
        .map(|&(name, reason)| (reason.to_raw(), name))
        .collect();
    assert_eq!(listed, NAMED_REASONS);
    for raw in 0..=u8::MAX {
        let reason = RejectMessageReason::from_raw(raw);
        assert_eq!(reason.to_raw(), raw);
        let name = NAMED_REASONS
            .iter()
            .find(|&&(named, _)| named == raw)
            .map(|&(_, name)| name);
        assert_eq!(reason.name(), name, "{raw}");
        assert_eq!(reason.is_defined(), name.is_some(), "{raw}");
        match name {
            Some(name) => {
                assert_eq!(
                    format!("{reason:?}"),
                    format!("RejectMessageReason::{name}")
                );
                assert_eq!(reason.to_string(), name);
            }
            None => {
                assert_eq!(format!("{reason:?}"), format!("RejectMessageReason({raw})"));
                assert_eq!(reason.to_string(), raw.to_string());
            }
        }
    }
}
