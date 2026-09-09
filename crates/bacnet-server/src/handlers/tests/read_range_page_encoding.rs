use super::*;

fn selected(
    backwards: bool,
    property: u32,
    index: Option<u32>,
    sequences: Option<&[u32]>,
) -> PreparedReadRange {
    PreparedReadRange {
        request: ReadRangeRequest {
            object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
            property_identifier: PropertyIdentifier::from_raw(property),
            property_array_index: index,
            range: Some(RangeSpec::ByPosition {
                reference_index: if backwards { 3 } else { 1 },
                count: if backwards { -3 } else { 3 },
            }),
        },
        items: vec![
            PropertyValue::Unsigned(1),
            PropertyValue::Unsigned(2),
            PropertyValue::Unsigned(3),
        ],
        selection: SignedRangeSelection::from_range(3, 0..3),
        first_sequence_number: sequences.map(|s| s[0]),
        identities: sequences.map(|s| {
            s.iter()
                .map(|seq| {
                    LogRecordIdentity::new(
                        *seq,
                        Date {
                            year: 126,
                            month: 9,
                            day: 1,
                            day_of_week: 2,
                        },
                        Time {
                            hour: 1,
                            minute: 0,
                            second: 0,
                            hundredths: 0,
                        },
                    )
                    .unwrap()
                })
                .collect()
        }),
    }
}

#[test]
fn bounded_encode_calls_defer_failures_and_preserve_output_on_current_error() {
    for backwards in [false, true] {
        let selected = selected(backwards, 131, None, None);
        for (cap, byte_cap, failure_at, expected_calls, success) in [
            (1, 100, 2, 1, true),  // next item deferred, never encoded
            (3, 16, 3, 2, true),   // one fitting item, one oversized trial, third untouched
            (3, 100, 2, 2, false), // error after accepted scratch, not a successful page
            (3, 13, 1, 0, false),  // even empty header cannot fit
            (3, 15, 3, 1, false),  // first item cannot fit; no empty MORE_ITEMS loop
        ] {
            let mut output = BytesMut::from(&b"old"[..]);
            let mut calls = 0;
            let mut order = Vec::new();
            let result = append_page_with(
                &selected,
                &mut output,
                ReadRangeBudget {
                    max_returned_items: cap,
                    max_service_ack_bytes: byte_cap,
                },
                |buf, item| {
                    calls += 1;
                    order.push(item.clone());
                    if calls == failure_at {
                        return Err(Error::Encoding("current item".into()));
                    }
                    encode_property_value(buf, item)
                },
            );
            assert_eq!(calls, expected_calls);
            assert_eq!(result.is_ok(), success);
            if success {
                let ack = ReadRangeAck::decode(&output[3..]).unwrap();
                assert_eq!(ack.item_count, 1);
                assert_eq!(ack.result_flags, (!backwards, backwards, true));
            } else {
                assert_eq!(&output[..], b"old");
                if calls == failure_at {
                    assert!(matches!(
                        result,
                        Err(ReadRangeFailure::Service(Error::Encoding(_)))
                    ));
                } else {
                    assert!(matches!(result, Err(ReadRangeFailure::Bytes)));
                }
            }
            if calls > 0 {
                assert_eq!(order[0], selected.items[if backwards { 2 } else { 0 }]);
            }
        }
    }
}

#[test]
fn exact_envelopes_include_property_index_and_changing_first_sequence_widths() {
    for property in [255, 256, 65535, 65536, u32::MAX] {
        for index in [
            None,
            Some(255),
            Some(256),
            Some(65535),
            Some(65536),
            Some(u32::MAX),
        ] {
            for sequences in [[u32::MAX, 255, 256], [1, 65536, 65535], [256, 1, u32::MAX]] {
                for backwards in [false, true] {
                    let selected = selected(backwards, property, index, Some(&sequences));
                    // Independent field-width arithmetic: OID=5, flags=3, count=2,
                    // opening/closing tags=2; unsigned/enumerated fields add tag+width.
                    let width = |x: u32| {
                        if x <= 255 {
                            1
                        } else if x <= 65535 {
                            2
                        } else if x <= 16777215 {
                            3
                        } else {
                            4
                        }
                    };
                    let fixed = 12 + 1 + width(property) + index.map_or(0, |i| 1 + width(i));
                    for n in [1, 2, 3] {
                        let first = if backwards { 3 - n } else { 0 };
                        let bytes = fixed + n * 2 + 1 + width(sequences[first]);
                        let mut output = BytesMut::new();
                        let result = append_page_with(
                            &selected,
                            &mut output,
                            ReadRangeBudget {
                                max_returned_items: n,
                                max_service_ack_bytes: bytes,
                            },
                            encode_property_value,
                        );
                        // A later narrower identity must not rescue a page whose
                        // directional first item (or earlier candidate) did not fit.
                        let first_over = (1..=n).find(|m| {
                            let start = if backwards { 3 - m } else { 0 };
                            fixed + m * 2 + 1 + width(sequences[start]) > bytes
                        });
                        if let Some(m) = first_over {
                            if m == 1 {
                                assert!(matches!(result, Err(ReadRangeFailure::Bytes)));
                                assert!(output.is_empty());
                            } else {
                                result.unwrap();
                                assert_eq!(
                                    ReadRangeAck::decode(&output).unwrap().item_count,
                                    (m - 1) as u32
                                );
                            }
                            continue;
                        }
                        result.unwrap();
                        assert_eq!(output.len(), bytes);
                        let ack = ReadRangeAck::decode(&output).unwrap();
                        assert_eq!(ack.first_sequence_number, Some(sequences[first]));
                        assert_eq!(ack.item_count, n as u32);
                        let mut expected = BytesMut::new();
                        for item in &selected.items[if backwards { 3 - n..3 } else { 0..n }] {
                            encode_property_value(&mut expected, item).unwrap();
                        }
                        assert_eq!(ack.item_data, expected);
                        let mut under = BytesMut::from(&b"old"[..]);
                        let result = append_page_with(
                            &selected,
                            &mut under,
                            ReadRangeBudget {
                                max_returned_items: n,
                                max_service_ack_bytes: bytes - 1,
                            },
                            encode_property_value,
                        );
                        if n == 1 {
                            assert!(matches!(result, Err(ReadRangeFailure::Bytes)));
                            assert_eq!(&under[..], b"old");
                        } else if result.is_ok() {
                            assert!(
                                ReadRangeAck::decode(&under[3..]).unwrap().item_count < n as u32
                            );
                        }
                    }
                }
            }
        }
    }
}
