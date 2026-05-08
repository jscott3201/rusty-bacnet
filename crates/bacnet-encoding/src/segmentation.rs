//! Segmentation: split and reassemble large APDU payloads.
//!
//! Per ASHRAE 135-2020 Clause 9, segmented messages use 8-bit sequence
//! numbers with windowed flow control. This module provides the basic
//! payload splitting and reassembly primitives.

use bacnet_types::error::Error;
use bytes::Bytes;
use std::collections::HashMap;

/// PDU types that affect segmentation overhead calculation.
///
/// Named `SegmentedPduType` to avoid collision with `bacnet_types::enums::PduType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentedPduType {
    /// ConfirmedRequest: 6-byte overhead (type + max-seg/apdu + invoke + seq + window + service).
    ConfirmedRequest,
    /// ComplexACK: 5-byte overhead (type + invoke + seq + window + service).
    ComplexAck,
}

/// Compute the maximum service data payload per segment.
///
/// This is the max APDU length minus the PDU header overhead for segmented messages.
pub fn max_segment_payload(max_apdu_length: u16, pdu_type: SegmentedPduType) -> usize {
    let overhead = match pdu_type {
        SegmentedPduType::ConfirmedRequest => 6,
        SegmentedPduType::ComplexAck => 5,
    };
    (max_apdu_length as usize).saturating_sub(overhead)
}

/// Split a payload into segments of at most `max_segment_size` bytes.
///
/// Always returns at least one segment (possibly empty).
pub fn split_payload(payload: &[u8], max_segment_size: usize) -> Result<Vec<Bytes>, Error> {
    if payload.is_empty() {
        return Ok(vec![Bytes::new()]);
    }
    if max_segment_size == 0 {
        return Err(Error::Segmentation(
            "non-empty payload cannot be segmented with max segment size 0".into(),
        ));
    }
    let segments: Vec<Bytes> = payload
        .chunks(max_segment_size)
        .map(Bytes::copy_from_slice)
        .collect();
    if segments.len() > 256 {
        return Err(Error::Segmentation(format!(
            "payload requires {} segments, maximum is 256",
            segments.len()
        )));
    }
    Ok(segments)
}

/// Collects received segments for reassembly.
///
/// Segments can arrive out of order. Call [`reassemble`](SegmentReceiver::reassemble)
/// once all segments have been received.
pub struct SegmentReceiver {
    segments: HashMap<u8, Bytes>,
}

impl Default for SegmentReceiver {
    fn default() -> Self {
        Self::new()
    }
}

impl SegmentReceiver {
    /// Create a new empty receiver.
    pub fn new() -> Self {
        Self {
            segments: HashMap::new(),
        }
    }

    /// Maximum BACnet APDU segment size (BACnet/IP over UDP).
    const MAX_SEGMENT_SIZE: usize = 1476;

    /// Store a received segment.
    ///
    /// Returns an error if the segment exceeds [`MAX_SEGMENT_SIZE`](Self::MAX_SEGMENT_SIZE).
    pub fn receive(&mut self, sequence_number: u8, data: Bytes) -> Result<(), Error> {
        if data.len() > Self::MAX_SEGMENT_SIZE {
            return Err(Error::Segmentation(format!(
                "segment size {} exceeds maximum {}",
                data.len(),
                Self::MAX_SEGMENT_SIZE
            )));
        }
        self.segments.insert(sequence_number, data);
        Ok(())
    }

    /// Check whether a specific segment has been received.
    pub fn has_segment(&self, sequence_number: u8) -> bool {
        self.segments.contains_key(&sequence_number)
    }

    /// Number of segments received so far.
    pub fn received_count(&self) -> usize {
        self.segments.len()
    }

    /// Reassemble all segments in order. `total_segments` is the expected count.
    ///
    /// Returns an error if any segment is missing or if `total_segments` exceeds
    /// the BACnet 8-bit sequence number limit (256).
    pub fn reassemble(&self, total_segments: usize) -> Result<Vec<u8>, Error> {
        if total_segments > 256 {
            return Err(Error::Segmentation(format!(
                "total_segments {total_segments} exceeds maximum BACnet value (256)"
            )));
        }
        let mut result = Vec::with_capacity(total_segments * 480);
        for i in 0..total_segments {
            let seq = i as u8;
            match self.segments.get(&seq) {
                Some(data) => result.extend_from_slice(data),
                None => {
                    return Err(Error::Segmentation(format!(
                        "missing segment {} of {}",
                        i, total_segments
                    )));
                }
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_segment_payload_confirmed_request() {
        assert_eq!(
            max_segment_payload(480, SegmentedPduType::ConfirmedRequest),
            474
        );
        assert_eq!(
            max_segment_payload(1476, SegmentedPduType::ConfirmedRequest),
            1470
        );
    }

    #[test]
    fn max_segment_payload_complex_ack() {
        assert_eq!(max_segment_payload(480, SegmentedPduType::ComplexAck), 475);
        assert_eq!(
            max_segment_payload(1476, SegmentedPduType::ComplexAck),
            1471
        );
    }

    #[test]
    fn split_payload_fits_single_segment() {
        let payload = vec![0u8; 100];
        let segments = split_payload(&payload, 200).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0], payload);
    }

    #[test]
    fn split_payload_exact_fit() {
        let payload = vec![0u8; 200];
        let segments = split_payload(&payload, 100).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].len(), 100);
        assert_eq!(segments[1].len(), 100);
    }

    #[test]
    fn split_payload_remainder() {
        let payload = vec![0u8; 250];
        let segments = split_payload(&payload, 100).unwrap();
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].len(), 100);
        assert_eq!(segments[1].len(), 100);
        assert_eq!(segments[2].len(), 50);
    }

    #[test]
    fn split_empty_payload() {
        let segments = split_payload(&[], 100).unwrap();
        assert_eq!(segments.len(), 1);
        assert!(segments[0].is_empty());
    }

    #[test]
    fn reassemble_ordered_segments() {
        let original = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let segments = split_payload(&original, 3).unwrap();
        assert_eq!(segments.len(), 4); // 3+3+3+1

        let mut receiver = SegmentReceiver::new();
        for (i, seg) in segments.iter().enumerate() {
            receiver.receive(i as u8, seg.clone()).unwrap();
        }
        let reassembled = receiver.reassemble(segments.len()).unwrap();
        assert_eq!(reassembled, original);
    }

    #[test]
    fn reassemble_out_of_order() {
        let mut receiver = SegmentReceiver::new();
        receiver.receive(2, Bytes::from_static(&[5, 6])).unwrap();
        receiver.receive(0, Bytes::from_static(&[1, 2])).unwrap();
        receiver.receive(1, Bytes::from_static(&[3, 4])).unwrap();
        let reassembled = receiver.reassemble(3).unwrap();
        assert_eq!(reassembled, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn reassemble_missing_segment_fails() {
        let mut receiver = SegmentReceiver::new();
        receiver.receive(0, Bytes::from_static(&[1, 2])).unwrap();
        // Missing segment 1
        receiver.receive(2, Bytes::from_static(&[5, 6])).unwrap();
        assert!(receiver.reassemble(3).is_err());
    }

    #[test]
    fn split_payload_zero_segment_size() {
        assert!(split_payload(&[1, 2, 3], 0).is_err());
    }

    #[test]
    fn split_payload_over_256_segments_errors() {
        let payload = vec![0u8; 257];
        assert!(split_payload(&payload, 1).is_err());
    }
}
