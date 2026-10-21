//! The owned encoders refuse an out-of-range header field with a static
//! [`Error::Range`] that names the field and its value (#1777).

use super::*;
use crate::npdu::{encode_npdu, Npdu, NpduAddress};
use bacnet_types::MacAddr;

/// The refusal's text, which a range error builds only when displayed.
fn refusal(result: Result<(), Error>) -> String {
    match result {
        Err(Error::Range(refused)) => refused.to_string(),
        other => panic!("expected a range error, got {other:?}"),
    }
}

fn request(max_segments: Option<u8>, max_apdu_length: u16, window: Option<u8>) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: window.is_some(),
        more_follows: false,
        segmented_response_accepted: true,
        max_segments,
        max_apdu_length,
        invoke_id: 1,
        sequence_number: window.map(|_| 0),
        proposed_window_size: window,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::new(),
    })
}

/// Encode `apdu` after octets already in the buffer; a refusal must leave
/// them, and only them, there.
fn encoded(apdu: &Apdu) -> Result<(), Error> {
    let mut buf = BytesMut::from(&[0xAA, 0xBB][..]);
    let result = encode_apdu(&mut buf, apdu);
    if result.is_err() {
        assert_eq!(
            &buf[..],
            &[0xAA, 0xBB],
            "a refusal leaves the buffer as it was"
        );
    }
    result
}

#[test]
fn header_fields_out_of_range_are_static_range_errors() {
    assert_eq!(
        refusal(encoded(&request(Some(1), 1476, None))),
        "value out of range: invalid max-segments-accepted 1; expected 2..=255 or unspecified"
    );
    assert_eq!(
        refusal(encoded(&request(None, 100, None))),
        "value out of range: invalid max-APDU-length 100; \
         expected one of 50, 128, 206, 480, 1024, 1476"
    );
    assert_eq!(
        refusal(encoded(&request(None, 1476, Some(128)))),
        "value out of range: ConfirmedRequest proposed-window-size 128 \
         outside BACnet range 1..=127"
    );
    let ack = Apdu::SegmentAck(SegmentAck {
        negative_ack: false,
        sent_by_server: false,
        invoke_id: 1,
        sequence_number: 0,
        actual_window_size: 0,
    });
    assert_eq!(
        refusal(encoded(&ack)),
        "value out of range: SegmentACK actual-window-size 0 outside BACnet range 1..=127"
    );
    let segmented_ack = Apdu::ComplexAck(ComplexAck {
        segmented: true,
        more_follows: true,
        invoke_id: 1,
        sequence_number: Some(0),
        proposed_window_size: Some(0),
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_ack: Bytes::new(),
    });
    assert_eq!(
        refusal(encoded(&segmented_ack)),
        "value out of range: ComplexAck proposed-window-size 0 outside BACnet range 1..=127"
    );
    assert_eq!(
        refusal(max_apdu_header_at_or_below(49).map(drop)),
        "value out of range: local APDU capacity 49 is below 50"
    );
}

#[test]
fn npdu_fields_out_of_range_are_static_range_errors() {
    let npdu = |destination, source| Npdu {
        destination,
        source,
        ..Npdu::default()
    };
    let address = |network, octets: usize| NpduAddress {
        network,
        mac_address: MacAddr::from_slice(&[7; 8][..octets]),
    };
    let refused = |npdu: &Npdu| refusal(encode_npdu(&mut BytesMut::new(), npdu));
    assert_eq!(
        refused(&npdu(Some(address(0, 0)), None)),
        "value out of range: NPDU DNET must not be 0"
    );
    assert_eq!(
        refused(&npdu(None, Some(address(0xFFFF, 1)))),
        "value out of range: NPDU SNET must be 1..65534, got 65535"
    );
    assert_eq!(
        refused(&npdu(None, Some(address(5, 0)))),
        "value out of range: NPDU SLEN must not be 0"
    );
}
