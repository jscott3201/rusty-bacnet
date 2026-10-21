//! A Reject-Message-To-Network's data: the reason and network it reads,
//! the lengths it refuses, and that writing undoes reading.

use bacnet_npdu::{
    NetworkMessageType, Npdu, RejectLengthError, RejectMessageReason, RejectMessageToNetwork,
};
use proptest::prelude::*;

#[test]
fn decode_reads_the_reason_then_the_network_high_octet_first() {
    let reject = RejectMessageToNetwork::decode(&[
        RejectMessageReason::MESSAGE_TOO_LONG.to_raw(),
        0x12,
        0x34,
    ])
    .unwrap();
    assert_eq!(reject.reason, RejectMessageReason::MESSAGE_TOO_LONG);
    assert_eq!(reject.network, 0x1234);
}

#[test]
fn a_reason_the_standard_does_not_name_is_kept() {
    for raw in [5u8, 7, 0x80, 0xFF] {
        let reject = RejectMessageToNetwork::decode(&[raw, 0x00, 0x01]).unwrap();
        assert_eq!(reject.reason.to_raw(), raw);
        assert_eq!(reject.to_bytes(), [raw, 0x00, 0x01]);
    }
}

#[test]
fn any_length_but_three_is_refused_with_that_length() {
    for payload in [&[][..], &[4], &[4, 0], &[4, 0, 1, 2], &[4, 0, 1, 2, 3]] {
        let refused = RejectMessageToNetwork::decode(payload).unwrap_err();
        assert_eq!(refused.length, payload.len(), "{payload:02X?}");
    }
    assert_eq!(RejectMessageToNetwork::LEN, 3);
}

#[test]
fn to_bytes_writes_the_reason_then_the_network() {
    let reject = RejectMessageToNetwork {
        reason: RejectMessageReason::NOT_DIRECTLY_CONNECTED,
        network: 3000,
    };
    assert_eq!(reject.to_bytes(), [1, 0x0B, 0xB8]);
}

#[test]
fn a_reject_in_an_npdu_reads_back() {
    let data = RejectMessageToNetwork {
        reason: RejectMessageReason::ROUTER_BUSY,
        network: 0xFFFE,
    }
    .to_bytes();
    let npdu = Npdu::message(NetworkMessageType::REJECT_MESSAGE_TO_NETWORK, &data);
    let mut frame = [0u8; 8];
    let len = npdu.encode_into(&mut frame).unwrap();
    assert_eq!(&frame[..len], &[0x01, 0x80, 0x03, 2, 0xFF, 0xFE]);
    let read = Npdu::decode(&frame[..len]).unwrap();
    assert_eq!(
        RejectMessageToNetwork::decode(read.payload),
        Ok(RejectMessageToNetwork {
            reason: RejectMessageReason::ROUTER_BUSY,
            network: 0xFFFE,
        })
    );
}

#[test]
fn the_length_error_says_what_was_wrong() {
    let refused: RejectLengthError = RejectMessageToNetwork::decode(&[0, 1]).unwrap_err();
    assert_eq!(refused.to_string(), "a reject's data is 2 octets, not 3");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// Three octets always read, and write back unchanged; any other length
    /// never reads.
    #[test]
    fn decode_takes_exactly_three_octets_and_round_trips(
        payload in prop::collection::vec(any::<u8>(), 0..6),
    ) {
        match RejectMessageToNetwork::decode(&payload) {
            Ok(reject) => {
                prop_assert_eq!(payload.len(), 3);
                prop_assert_eq!(&reject.to_bytes()[..], &payload[..]);
            }
            Err(refused) => {
                prop_assert_ne!(payload.len(), 3);
                prop_assert_eq!(refused.length, payload.len());
            }
        }
    }

    /// Writing then reading gives back the same reason and network.
    #[test]
    fn reading_undoes_writing(raw in any::<u8>(), network in any::<u16>()) {
        let reject = RejectMessageToNetwork {
            reason: RejectMessageReason::from_raw(raw),
            network,
        };
        prop_assert_eq!(RejectMessageToNetwork::decode(&reject.to_bytes()), Ok(reject));
    }
}
