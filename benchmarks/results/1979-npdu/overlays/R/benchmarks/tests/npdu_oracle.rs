//! Literal correctness guards for the historical parser controls.
#[path = "../compat/npdu_attribution.rs"]
mod attribution;
use bacnet_encoding::npdu::decode_npdu;
use bytes::Bytes;

#[test]
fn literal_anchor_and_payload_sharing() {
    let mut literal = vec![0; 102];
    literal[0] = 1;
    let data = Bytes::from(literal);
    attribution::assert_anchor(&data);
    let _diagnostics: fn(&mut criterion::Criterion) = attribution::diagnostics;
    assert!(decode_npdu(data).unwrap().message_type.is_none());
}
