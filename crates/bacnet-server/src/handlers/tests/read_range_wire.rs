use super::*;
use crate::server::{BACnetServer, ReadRangeBudget, ServerConfig};
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu, NpduAddress};
use bacnet_transport::bip::BipTransport;
use bacnet_types::enums::{AbortReason, ConfirmedServiceChoice, Segmentation};
use bytes::Bytes;
use std::net::Ipv4Addr;
use std::time::Duration;

#[tokio::test]
async fn read_range_unsegmented_direct_and_routed_fit_actual_peer_local_envelope() {
    for segmentation in [
        Segmentation::NONE,
        Segmentation::RECEIVE,
        Segmentation::TRANSMIT,
        Segmentation::BOTH,
    ] {
        for accepts in [false, true] {
            if accepts && matches!(segmentation, Segmentation::TRANSMIT | Segmentation::BOTH) {
                continue;
            }
            for (peer, local) in [(50, 1476), (480, 50)] {
                for (cap, expected_count) in
                    [(16384, Some(16)), (18, Some(2)), (15, None), (1, None)]
                {
                    let items = unsigned_items(&(1..=30).collect::<Vec<_>>());
                    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), None);
                    let mut server = BACnetServer::start(
                        ServerConfig {
                            segmentation_supported: segmentation,
                            max_apdu_length: local,
                            read_range_budget: ReadRangeBudget {
                                max_service_ack_bytes: cap,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        db,
                        BipTransport::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::LOCALHOST),
                    )
                    .await
                    .unwrap();
                    let mac = server.local_mac();
                    let dest = (
                        Ipv4Addr::new(mac[0], mac[1], mac[2], mac[3]),
                        u16::from_be_bytes([mac[4], mac[5]]),
                    );
                    let socket = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                        .await
                        .unwrap();
                    for routed in [false, true] {
                        let route = routed.then(|| NpduAddress {
                            network: 7,
                            mac_address: bacnet_types::MacAddr::from_slice(&[9]),
                        });
                        let mut service = BytesMut::new();
                        ReadRangeRequest {
                            object_identifier: oid,
                            property_identifier: PropertyIdentifier::LOG_BUFFER,
                            property_array_index: None,
                            range: None,
                        }
                        .encode(&mut service);
                        let request = Apdu::ConfirmedRequest(ConfirmedRequest {
                            segmented: false,
                            more_follows: false,
                            segmented_response_accepted: accepts,
                            max_segments: None,
                            max_apdu_length: peer,
                            invoke_id: 17,
                            sequence_number: None,
                            proposed_window_size: None,
                            service_choice: ConfirmedServiceChoice::READ_RANGE,
                            service_request: service.freeze(),
                        });
                        let mut apdu = BytesMut::new();
                        encode_apdu(&mut apdu, &request).unwrap();
                        let mut npdu = BytesMut::new();
                        encode_npdu(
                            &mut npdu,
                            &Npdu {
                                source: route.clone(),
                                payload: apdu.freeze(),
                                ..Default::default()
                            },
                        )
                        .unwrap();
                        let mut wire = vec![0x81, 0x0a];
                        wire.extend_from_slice(&((npdu.len() + 4) as u16).to_be_bytes());
                        wire.extend_from_slice(&npdu);
                        socket.send_to(&wire, dest).await.unwrap();
                        let mut reply = vec![0; 4096];
                        let n =
                            tokio::time::timeout(Duration::from_secs(2), socket.recv(&mut reply))
                                .await
                                .unwrap()
                                .unwrap();
                        assert_eq!(u16::from_be_bytes([reply[2], reply[3]]) as usize, n);
                        let npdu = decode_npdu(Bytes::copy_from_slice(&reply[4..n])).unwrap();
                        assert_eq!(npdu.destination, route);
                        assert!(npdu.payload.len() <= 50);
                        match (decode_apdu(npdu.payload).unwrap(), expected_count) {
                            (Apdu::ComplexAck(a), Some(count)) => {
                                assert!(!a.segmented);
                                assert!(a.service_ack.len() <= cap);
                                assert_ack(
                                    &ReadRangeAck::decode(&a.service_ack).unwrap(),
                                    &items[..count],
                                    (true, false, true),
                                    None,
                                );
                            }
                            (Apdu::Abort(a), None) => {
                                assert!(a.sent_by_server);
                                assert_eq!(a.abort_reason, AbortReason::BUFFER_OVERFLOW);
                            }
                            other => panic!("unexpected response: {other:?}"),
                        }
                    }
                    server.stop().await.unwrap();
                }
            }
        }
    }
}

#[tokio::test]
async fn read_range_segmented_complete_page_keeps_config_cap_and_actual_identity() {
    use bacnet_client::client::BACnetClient;
    for segmentation in [Segmentation::TRANSMIT, Segmentation::BOTH] {
        let items = unsigned_items(&(1..=100).collect::<Vec<_>>());
        let ids = (0..100).map(|i| identity(1000 + i * 2, 1)).collect();
        let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), Some(ids));
        let mut server = BACnetServer::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .database(db)
            .segmentation_supported(segmentation)
            .read_range_budget(ReadRangeBudget {
                max_returned_items: 60,
                ..Default::default()
            })
            .build()
            .await
            .unwrap();
        let mut client = BACnetClient::bip_builder()
            .interface(Ipv4Addr::LOCALHOST)
            .port(0)
            .max_apdu_length(50)
            .build()
            .await
            .unwrap();
        let ack = client
            .read_range(
                server.local_mac(),
                oid,
                PropertyIdentifier::LOG_BUFFER,
                None,
                Some(RangeSpec::BySequenceNumber {
                    reference_seq: 1198,
                    count: -100,
                }),
            )
            .await
            .unwrap();
        assert_ack(&ack, &items[40..], (false, true, true), Some(1080));
        let mut encoded = BytesMut::new();
        ack.encode(&mut encoded);
        assert!(encoded.len() > 50);
        for backwards in [false, true] {
            let first = client
                .read_range(
                    server.local_mac(),
                    oid,
                    PropertyIdentifier::LOG_BUFFER,
                    None,
                    Some(RangeSpec::ByTime {
                        reference_time: (DATE, time(if backwards { 2 } else { 0 })),
                        count: if backwards { -100 } else { 100 },
                    }),
                )
                .await
                .unwrap();
            assert_eq!(first.item_count, 60);
            assert_eq!(
                first.first_sequence_number,
                Some(if backwards { 1080 } else { 1000 })
            );
            // These next resident identities are known from this stable fixture,
            // not guessed by adding an ordinal to First_Sequence_Number.
            let next = client
                .read_range(
                    server.local_mac(),
                    oid,
                    PropertyIdentifier::LOG_BUFFER,
                    None,
                    Some(RangeSpec::BySequenceNumber {
                        reference_seq: if backwards { 1078 } else { 1120 },
                        count: if backwards { -40 } else { 40 },
                    }),
                )
                .await
                .unwrap();
            assert_eq!(next.item_count, 40);
            assert!(!next.result_flags.2);
            let combined = if backwards {
                [next.item_data, first.item_data].concat()
            } else {
                [first.item_data, next.item_data].concat()
            };
            assert_eq!(combined, encoded_items(&items));
        }
        client.stop().await.unwrap();
        server.stop().await.unwrap();
    }
}
