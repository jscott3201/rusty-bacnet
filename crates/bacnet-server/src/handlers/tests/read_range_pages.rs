use super::*;
use crate::server::ReadRangeBudget;

fn page(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    range: Option<RangeSpec>,
    cap: usize,
    bytes: usize,
) -> Result<ReadRangeAck, ReadRangeFailure> {
    let request = ReadRangeRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range,
    };
    let mut input = BytesMut::new();
    request.encode(&mut input);
    let mut output = BytesMut::from(&b"sentinel"[..]);
    let result = handle_read_range_budgeted(
        db,
        &input,
        &mut output,
        ReadRangeBudget {
            max_returned_items: cap,
            max_service_ack_bytes: bytes,
        },
    );
    if result.is_err() {
        assert_eq!(&output[..], b"sentinel");
    }
    result?;
    assert!(output.len() - 8 <= bytes);
    Ok(ReadRangeAck::decode(&output[8..]).unwrap())
}

#[test]
fn directional_pages_preserve_sparse_wrapped_resident_identity() {
    let items = unsigned_items(&[10, 20, 30, 40, 50]);
    let ids = vec![
        identity(u32::MAX, 1),
        identity(1, 2),
        identity(255, 3),
        identity(65536, 4),
        identity(9, 5),
    ];
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), Some(ids));
    for cap in [1, 2, 5, 256] {
        for kind in 0..3 {
            for backwards in [false, true] {
                let count = if backwards { -4 } else { 4 };
                let range = match kind {
                    0 => RangeSpec::ByPosition {
                        reference_index: if backwards { 4 } else { 2 },
                        count,
                    },
                    1 => RangeSpec::BySequenceNumber {
                        reference_seq: if backwards { 65536 } else { 1 },
                        count,
                    },
                    _ => RangeSpec::ByTime {
                        reference_time: (DATE, time(if backwards { 5 } else { 1 })),
                        count,
                    },
                };
                let n = cap.min(4);
                let expected = if backwards { 4 - n..4 } else { 1..1 + n };
                let first = (kind != 0).then(|| [u32::MAX, 1, 255, 65536, 9][expected.start]);
                let ack = page(&db, oid, Some(range), cap, 16384).unwrap();
                assert_ack(
                    &ack,
                    &items[expected.clone()],
                    (expected.start == 0, expected.end == 5, n < 4),
                    first,
                );
            }
        }
        let n = cap.min(5);
        assert_ack(
            &page(&db, oid, None, cap, 16384).unwrap(),
            &items[..n],
            (true, n == 5, n < 5),
            None,
        );
    }
}

#[test]
fn count_width_255_256_and_default_exact_limit() {
    for total in [256, 257] {
        let items = vec![PropertyValue::Unsigned(1); total];
        let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), None);
        let unlimited = call(&db, oid, PropertyIdentifier::LOG_BUFFER, None).unwrap();
        for n in [1, 255, 256] {
            let ack = page(&db, oid, None, n, 16384).unwrap();
            let mut encoded = BytesMut::new();
            ack.encode(&mut encoded);
            assert_eq!(encoded.len(), 12 + if n == 256 { 3 } else { 2 } + n * 2);
            assert_ack(
                &page(&db, oid, None, usize::MAX, encoded.len()).unwrap(),
                &items[..n],
                (true, n == total, n < total),
                None,
            );
            if n > 1 {
                assert_eq!(
                    page(&db, oid, None, usize::MAX, encoded.len() - 1)
                        .unwrap()
                        .item_count,
                    (n - 1) as u32
                );
            }
        }
        let parity = page(&db, oid, None, usize::MAX, usize::MAX).unwrap();
        let mut actual = BytesMut::new();
        let mut expected = BytesMut::new();
        parity.encode(&mut actual);
        unlimited.encode(&mut expected);
        assert_eq!(actual, expected);
    }
}

#[test]
fn missing_references_empty_lists_and_validation_precede_tiny_budget() {
    let (db, oid) = list_db(
        PropertyIdentifier::LOG_BUFFER,
        unsigned_items(&[1]),
        Some(vec![identity(1, 1)]),
    );
    for range in [
        RangeSpec::ByPosition {
            reference_index: 2,
            count: -1,
        },
        RangeSpec::BySequenceNumber {
            reference_seq: 2,
            count: 1,
        },
        RangeSpec::ByTime {
            reference_time: (DATE, time(0)),
            count: -1,
        },
    ] {
        assert_ack(
            &page(&db, oid, Some(range.clone()), 1, 14).unwrap(),
            &[],
            (false, false, false),
            None,
        );
        assert!(matches!(
            page(&db, oid, Some(range), 1, 13),
            Err(ReadRangeFailure::Bytes)
        ));
    }
    let (empty, empty_oid) = list_db(PropertyIdentifier::LOG_BUFFER, vec![], None);
    assert_ack(
        &page(&empty, empty_oid, None, 1, 14).unwrap(),
        &[],
        (false, false, false),
        None,
    );
    for ids in [None, Some(vec![])] {
        let (bad, bad_oid) = list_db(PropertyIdentifier::LOG_BUFFER, unsigned_items(&[1]), ids);
        for range in [
            RangeSpec::BySequenceNumber {
                reference_seq: 1,
                count: 1,
            },
            RangeSpec::ByTime {
                reference_time: (DATE, time(0)),
                count: 1,
            },
        ] {
            assert!(matches!(
                page(&bad, bad_oid, Some(range), 1, 1),
                Err(ReadRangeFailure::Service(Error::Protocol { .. }))
            ));
        }
    }
    for count in [0, 32768, -32769] {
        assert!(matches!(
            page(
                &db,
                oid,
                Some(RangeSpec::ByPosition {
                    reference_index: 1,
                    count
                }),
                1,
                1
            ),
            Err(ReadRangeFailure::Service(Error::Decoding { .. }))
        ));
    }
    assert!(matches!(
        page(&empty, oid, None, 1, 1),
        Err(ReadRangeFailure::Bytes)
    ));
    let missing_oid = ObjectIdentifier::new(ObjectType::TREND_LOG, 92).unwrap();
    assert!(
        matches!(page(&db, missing_oid, None, 1, 1), Err(ReadRangeFailure::Service(Error::Protocol { code, .. }))
        if code == ErrorCode::UNKNOWN_OBJECT.to_raw() as u32)
    );
}

#[test]
fn read_range_error_precedence_matches_legacy_before_pagination() {
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, unsigned_items(&[1]), None);
    for (property, index, range) in [
        (PropertyIdentifier::LOG_BUFFER, Some(0), None),
        (PropertyIdentifier::LOG_BUFFER, Some(1), None),
        (PropertyIdentifier::ALL, None, None),
        (PropertyIdentifier::REQUIRED, None, None),
        (PropertyIdentifier::OPTIONAL, None, None),
        (PropertyIdentifier::PRESENT_VALUE, None, None),
        (
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::ByTime {
                reference_time: (DATE, time(24)),
                count: 1,
            }),
        ),
    ] {
        let mut request = BytesMut::new();
        ReadRangeRequest {
            object_identifier: oid,
            property_identifier: property,
            property_array_index: index,
            range,
        }
        .encode(&mut request);
        let legacy = handle_read_range(&db, &request, &mut BytesMut::new()).unwrap_err();
        let mut output = BytesMut::from(&b"old"[..]);
        let ReadRangeFailure::Service(error) = handle_read_range_budgeted(
            &db,
            &request,
            &mut output,
            ReadRangeBudget {
                max_returned_items: 1,
                max_service_ack_bytes: 1,
            },
        )
        .unwrap_err() else {
            panic!("budget hid existing error")
        };
        assert_eq!(error.to_string(), legacy.to_string());
        assert_eq!(&output[..], b"old");
    }
    let mut scalar_db = ObjectDatabase::new();
    let scalar = bacnet_objects::analog::AnalogValueObject::new(1, "scalar", 62).unwrap();
    let scalar_oid = scalar.object_identifier();
    scalar_db.add(Box::new(scalar)).unwrap();
    let mut request = BytesMut::new();
    ReadRangeRequest {
        object_identifier: scalar_oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
        range: None,
    }
    .encode(&mut request);
    assert!(
        matches!(handle_read_range_budgeted(&scalar_db, &request, &mut BytesMut::new(),
        ReadRangeBudget { max_returned_items:1, max_service_ack_bytes:1 }),
        Err(ReadRangeFailure::Service(Error::Protocol { code, .. })) if code == ErrorCode::PROPERTY_IS_NOT_A_LIST.to_raw() as u32)
    );
}

#[test]
fn byte_overflow_never_skips_or_loses_direction_anchor() {
    let items = vec![
        PropertyValue::Unsigned(1),
        PropertyValue::OctetString(vec![0; 100]),
        PropertyValue::Unsigned(3),
    ];
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), None);
    for backwards in [false, true] {
        let range = Some(RangeSpec::ByPosition {
            reference_index: if backwards { 3 } else { 1 },
            count: if backwards { -3 } else { 3 },
        });
        let ack = page(&db, oid, range, 10, 20).unwrap();
        assert_ack(
            &ack,
            &items[if backwards { 2..3 } else { 0..1 }],
            (!backwards, backwards, true),
            None,
        );
        let oversized = Some(RangeSpec::ByPosition {
            reference_index: 2,
            count: if backwards { -2 } else { 2 },
        });
        assert!(matches!(
            page(&db, oid, oversized, 10, 20),
            Err(ReadRangeFailure::Bytes)
        ));
    }
}

#[tokio::test]
async fn read_range_client_pages_all_matches_forward_and_backward() {
    use crate::server::BACnetServer;
    use bacnet_client::client::BACnetClient;
    use std::net::Ipv4Addr;
    let items = unsigned_items(&[10, 20, 30, 40, 50]);
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), None);
    let mut server = BACnetServer::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .database(db)
        .read_range_budget(ReadRangeBudget {
            max_returned_items: 2,
            ..Default::default()
        })
        .build()
        .await
        .unwrap();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .build()
        .await
        .unwrap();
    for backwards in [false, true] {
        let mut remaining = 5;
        let mut pieces = Vec::new();
        while remaining > 0 {
            let reference_index = if backwards { remaining } else { 6 - remaining };
            let ack = client
                .read_range(
                    server.local_mac(),
                    oid,
                    PropertyIdentifier::LOG_BUFFER,
                    None,
                    Some(RangeSpec::ByPosition {
                        reference_index,
                        count: if backwards {
                            -(remaining as i32)
                        } else {
                            remaining as i32
                        },
                    }),
                )
                .await
                .unwrap();
            assert!(ack.item_count > 0 && ack.item_count <= 2);
            remaining -= ack.item_count;
            assert_eq!(ack.result_flags.2, remaining > 0);
            pieces.push(ack.item_data);
        }
        if backwards {
            pieces.reverse();
        }
        assert_eq!(pieces.concat(), encoded_items(&items));
    }
    client.stop().await.unwrap();
    server.stop().await.unwrap();
}
