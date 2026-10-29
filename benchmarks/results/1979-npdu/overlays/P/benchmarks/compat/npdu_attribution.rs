//! Diagnostics for the retained shared-Bytes NPDU workload; timings are not additive.
use bacnet_encoding::npdu::decode_npdu;
use bytes::Bytes;
use criterion::Criterion;
use std::hint::black_box;

/// Check the independent literal and every decoded field before timing.
pub fn assert_anchor(data: &Bytes) {
    let mut expected = [0; 102];
    expected[0] = 1;
    assert_eq!(data.as_ref(), expected);
    let owned = decode_npdu(data.clone()).unwrap();
    assert!(!owned.expecting_reply);
    assert_eq!(owned.priority.to_raw(), 0);
    assert!(owned.destination.is_none());
    assert!(owned.source.is_none());
    assert_eq!(owned.hop_count, 255);
    assert!(owned.message_type.is_none());
    assert!(owned.vendor_id.is_none());
    assert_eq!(owned.payload.as_ref(), &[0; 100]);
    assert_eq!(owned.payload.as_ptr(), data[2..].as_ptr());
    let view = bacnet_npdu::Npdu::decode(data).unwrap();
    assert!(!view.expecting_reply);
    assert_eq!(view.priority.to_raw(), 0);
    assert!(view.destination.is_none());
    assert!(view.source.is_none());
    assert_eq!(view.hop_count, 255);
    assert!(!view.is_network_message());
    assert!(view.message_type.is_none());
    assert!(view.vendor_id.is_none());
    assert_eq!(view.payload_offset, 2);
    assert_eq!(view.payload, &[0; 100]);
    assert_eq!(view.payload.as_ptr(), data[2..].as_ptr());
}

/// Measure independent borrowed decoding and shared ownership bookkeeping.
pub fn diagnostics(c: &mut Criterion) {
    let mut literal = vec![0; 102];
    literal[0] = 1;
    let data = Bytes::from(literal);
    assert_anchor(&data);
    c.bench_function("npdu_diagnostic_borrowed_decode_100b_payload", |b| {
        b.iter(|| {
            black_box(bacnet_npdu::Npdu::decode(&data).unwrap());
        })
    });
    c.bench_function("npdu_diagnostic_shared_bytes_clone_drop_102b", |b| {
        b.iter(|| {
            black_box(data.clone());
        })
    });
}
