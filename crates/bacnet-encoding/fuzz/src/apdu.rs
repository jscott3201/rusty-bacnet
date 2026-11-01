//! `apdu_decode`: the owned APDU decoder, every PDU type with segmented
//! headers included, against its encoder and against the allocation-free
//! header view; then the segment reassembler, fed segments decoded from the
//! same octets.
//!
//! - `decode_apdu` and `encode_apdu` round-trip as [`round_trip`] states,
//!   and a decoded APDU that encodes decodes back to an equal one.
//! - `ApduHeader::decode` and `decode_apdu` agree. Where the view accepts,
//!   the owned decoder accepts with the same fields and the same service
//!   octets after the header (an Error only when its body decodes too: the
//!   view reads no further than the header). Where the owned decoder
//!   accepts a PDU type the view reads, unsegmented, the view accepts.
//! - Read as a count and a run of length-prefixed APDUs, the input's
//!   segmented Confirmed-Requests and ComplexACKs reassemble to exactly
//!   their service octets in sequence order (the last of a repeated
//!   sequence number counting), or the reassembler refuses a gap; it always
//!   refuses a count past 256.
//!
//! [`round_trip`]: crate::round_trip

use bacnet_encoding::apdu::header::ApduHeader;
use bacnet_encoding::apdu::{Apdu, decode_apdu, encode_apdu};
use bacnet_encoding::segmentation::SegmentReceiver;
use bytes::{Bytes, BytesMut};

use crate::round_trip;

/// Runs every check on `input`.
pub fn check(input: &[u8]) {
    round_trip(
        "APDU",
        input,
        |data| decode_apdu(Bytes::copy_from_slice(data)).map(|apdu| (apdu, data.len())),
        |apdu, buf| encode_apdu(buf, apdu),
    );
    let owned = decode_apdu(Bytes::copy_from_slice(input));
    if let Ok(apdu) = &owned {
        let mut wire = BytesMut::new();
        if encode_apdu(&mut wire, apdu).is_ok() {
            let again = decode_apdu(wire.freeze()).expect("an encoded APDU decodes");
            assert_eq!(&again, apdu, "an APDU changed in a round trip");
        }
    }
    header_agrees(input, owned.as_ref().ok());
    reassemble(input);
}

/// The header view against the owned decoder's result for the same input.
fn header_agrees(input: &[u8], owned: Option<&Apdu>) {
    match ApduHeader::decode(input) {
        Ok((header, parameters)) => match (header, owned) {
            (ApduHeader::ConfirmedRequest(view), Some(Apdu::ConfirmedRequest(pdu))) => {
                assert!(!pdu.segmented, "the view read a segmented request");
                assert_eq!(view.invoke_id, pdu.invoke_id);
                assert_eq!(view.service_choice, pdu.service_choice);
                assert_eq!(
                    view.segmented_response_accepted,
                    pdu.segmented_response_accepted
                );
                assert_eq!(view.max_apdu.octets(), pdu.max_apdu_length);
                assert_eq!(parameters, &pdu.service_request[..]);
            }
            (ApduHeader::UnconfirmedRequest(service), Some(Apdu::UnconfirmedRequest(pdu))) => {
                assert_eq!(service, pdu.service_choice);
                assert_eq!(parameters, &pdu.service_request[..]);
            }
            (
                ApduHeader::ComplexAck {
                    invoke_id,
                    service_choice,
                },
                Some(Apdu::ComplexAck(pdu)),
            ) => {
                assert!(!pdu.segmented, "the view read a segmented ComplexACK");
                assert_eq!(
                    (invoke_id, service_choice),
                    (pdu.invoke_id, pdu.service_choice)
                );
                assert_eq!(parameters, &pdu.service_ack[..]);
            }
            (
                ApduHeader::Error {
                    invoke_id,
                    service_choice,
                },
                owned,
            ) => {
                if let Some(Apdu::Error(pdu)) = owned {
                    assert_eq!(
                        (invoke_id, service_choice),
                        (pdu.invoke_id, pdu.service_choice)
                    );
                } else {
                    assert!(owned.is_none(), "the view read an Error as {owned:?}");
                }
            }
            (ApduHeader::Reject { invoke_id, reason }, Some(Apdu::Reject(pdu))) => {
                assert_eq!((invoke_id, reason), (pdu.invoke_id, pdu.reject_reason));
            }
            (
                ApduHeader::Abort {
                    server,
                    invoke_id,
                    reason,
                },
                Some(Apdu::Abort(pdu)),
            ) => {
                assert_eq!(
                    (server, invoke_id, reason),
                    (pdu.sent_by_server, pdu.invoke_id, pdu.abort_reason)
                );
            }
            (header, owned) => {
                panic!("the view read {header:?}, the owned decoder {owned:?}")
            }
        },
        Err(error) => {
            let viewed = match owned {
                Some(Apdu::ConfirmedRequest(pdu)) => !pdu.segmented,
                Some(Apdu::ComplexAck(pdu)) => !pdu.segmented,
                Some(
                    Apdu::UnconfirmedRequest(_) | Apdu::Error(_) | Apdu::Reject(_) | Apdu::Abort(_),
                ) => true,
                Some(Apdu::SimpleAck(_) | Apdu::SegmentAck(_)) | None => false,
            };
            assert!(
                !viewed,
                "the view refused {owned:?}, which it reads: {error:?}"
            );
        }
    }
}

/// Reads `input` as a segment count less one, then APDUs, each behind a
/// length octet; feeds the reassembler the service octets of each segmented
/// Confirmed-Request or ComplexACK that decodes, by its sequence number, and
/// reassembles them.
fn reassemble(input: &[u8]) {
    let Some((&count, mut rest)) = input.split_first() else {
        return;
    };
    let mut receiver = SegmentReceiver::new();
    let mut expected: [Option<Bytes>; 256] = [const { None }; 256];
    while let [length, tail @ ..] = rest {
        let length = usize::from(*length).min(tail.len());
        let (pdu, after) = tail.split_at(length);
        rest = after;
        let (sequence, payload) = match decode_apdu(Bytes::copy_from_slice(pdu)) {
            Ok(Apdu::ConfirmedRequest(pdu)) if pdu.segmented => {
                (pdu.sequence_number, pdu.service_request)
            }
            Ok(Apdu::ComplexAck(pdu)) if pdu.segmented => (pdu.sequence_number, pdu.service_ack),
            _ => continue,
        };
        let sequence = sequence.expect("a segmented PDU has a sequence number");
        receiver
            .receive(sequence, payload.clone())
            .expect("a segment under 1,476 octets is taken");
        expected[usize::from(sequence)] = Some(payload);
    }
    let total = usize::from(count) + 1;
    let wanted: Option<Vec<u8>> =
        expected[..total]
            .iter()
            .try_fold(Vec::new(), |mut whole, segment| {
                whole.extend_from_slice(segment.as_ref()?);
                Some(whole)
            });
    match (receiver.reassemble(total), wanted) {
        (Ok(whole), Some(wanted)) => assert_eq!(whole, wanted),
        (Err(_), None) => {}
        (result, wanted) => panic!("reassembled {result:?}, expected {wanted:?}"),
    }
    assert!(
        receiver.reassemble(257).is_err(),
        "257 segments reassembled"
    );
}
