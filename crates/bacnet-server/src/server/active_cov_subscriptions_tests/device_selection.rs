//! Multi-Device fixtures are complete before startup; runtime membership changes
//! and discovery-limiter rebinding are deliberately outside this contract.
use super::*;
use bacnet_encoding::apdu::decode_apdu;
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_encoding::primitives::encode_app_object_id;
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::cov_multiple::COVNotificationMultipleRequest;
use bacnet_services::who_has::{IHaveRequest, WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::{IAmRequest, WhoIsRequest};
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};

#[tokio::test(start_paused = true)]
async fn lowest_device_owns_wildcard_reads_and_both_live_cov_lists_in_both_orders() {
    for instances in [[900, 813], [813, 900]] {
        let mut wire = Wire::start_with_devices(ServerConfig::default(), &instances).await;
        let other = ObjectIdentifier::new(ObjectType::DEVICE, 900).unwrap();
        simple_ack(
            wire.send(&direct(), subscribe_cov(61, av(1), Some(false), None))
                .await,
        );
        simple_ack(
            wire.send(
                &direct(),
                subscribe_cov_property(av(1), (62, PV, None, None, Some(false), Some(300))),
            )
            .await,
        );
        simple_ack(
            wire.send(
                &direct(),
                subscribe_cov_property_multiple(
                    63,
                    false,
                    Some((300, 1)),
                    vec![(av(1), vec![plain(PV)])],
                ),
            )
            .await,
        );

        for _ in 0..3 {
            for property in [ACTIVE, MULTIPLE] {
                let selected = wire.read(device(), property, None).await.unwrap();
                assert!(!selected.is_empty(), "accepted subscriptions are live");
                assert_same_list(
                    property,
                    &wire.read(wildcard(), property, None).await.unwrap(),
                    &selected,
                );
                assert!(wire.read(other, property, None).await.unwrap().is_empty());
                for oid in [device(), wildcard()] {
                    let PropertyValue::ApplicationData(local) =
                        wire.server.read_local(&oid, property, None).await.unwrap()
                    else {
                        panic!("encoded COV list");
                    };
                    assert_same_list(property, &local, &selected);
                }
                assert_eq!(
                    wire.server
                        .read_local(&other, property, None)
                        .await
                        .unwrap(),
                    PropertyValue::ApplicationData(Vec::new())
                );

                let ack = wire
                    .rpm(vec![
                        (device(), vec![(property, None)]),
                        (
                            wildcard(),
                            vec![
                                (property, None),
                                (PropertyIdentifier::ALL, None),
                                (PropertyIdentifier::OPTIONAL, None),
                            ],
                        ),
                        (
                            other,
                            vec![
                                (property, None),
                                (PropertyIdentifier::ALL, None),
                                (PropertyIdentifier::OPTIONAL, None),
                            ],
                        ),
                    ])
                    .await;
                assert_eq!(
                    ack.list_of_read_access_results[1].object_identifier,
                    device()
                );
                let lists = rows(&ack, property);
                assert_eq!(lists.len(), 7);
                assert!(lists[..4].iter().all(|list| !list.is_empty()));
                assert!(lists[4..].iter().all(Vec::is_empty));
                // Finite Multiple lifetimes can tick between requests. Within
                // one RPM response all selected rows share a single snapshot.
                assert!(lists[..4].iter().all(|list| list == &lists[0]));
            }
            let property = PropertyIdentifier::OBJECT_IDENTIFIER;
            let mut expected = BytesMut::new();
            encode_app_object_id(&mut expected, &device());
            assert_eq!(
                wire.read(wildcard(), property, None).await.unwrap(),
                expected
            );
            let ack = wire.rpm(vec![(wildcard(), vec![(property, None)])]).await;
            assert_eq!(
                ack.list_of_read_access_results[0].object_identifier,
                device()
            );
            assert_eq!(rows(&ack, property), vec![expected.to_vec()]);
            assert_eq!(
                wire.server
                    .read_local(&wildcard(), property, None)
                    .await
                    .unwrap(),
                PropertyValue::ObjectIdentifier(device())
            );
        }

        // Inspect the actual outbound I-Am and initial ordinary/Single/Multiple
        // notifications, so the read selector cannot diverge from send identity.
        wire.server.broadcast_i_am().await.unwrap();
        let mut who_is = BytesMut::new();
        WhoIsRequest {
            range: Some(DeviceInstanceRange::single(813).unwrap()),
        }
        .encode(&mut who_is);
        unconfirmed(&wire, UnconfirmedServiceChoice::WHO_IS, who_is).await;
        let mut who_has = BytesMut::new();
        WhoHasRequest {
            range: Some(DeviceInstanceRange::single(813).unwrap()),
            object: WhoHasObject::Identifier(av(1)),
        }
        .encode(&mut who_has)
        .unwrap();
        unconfirmed(&wire, UnconfirmedServiceChoice::WHO_HAS, who_has).await;
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if wire.sent.len() >= 6 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all initial notifications and I-Am sent");
        let packets = wire.sent.frames();
        let mut processes = Vec::new();
        let mut i_am = 0;
        let mut i_have = 0;
        for packet in packets {
            let npdu = decode_npdu(packet.npdu).unwrap();
            let Apdu::UnconfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected unconfirmed discovery or COV");
            };
            let initiating_device = match request.service_choice {
                UnconfirmedServiceChoice::I_AM => {
                    i_am += 1;
                    IAmRequest::decode(&request.service_request)
                        .unwrap()
                        .object_identifier
                }
                UnconfirmedServiceChoice::I_HAVE => {
                    i_have += 1;
                    let response = IHaveRequest::decode(&request.service_request).unwrap();
                    assert_eq!(response.object_identifier, av(1));
                    response.device_identifier
                }
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION => {
                    let cov = COVNotificationRequest::decode(&request.service_request).unwrap();
                    processes.push(cov.subscriber_process_identifier);
                    cov.initiating_device_identifier
                }
                UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION_MULTIPLE => {
                    let cov =
                        COVNotificationMultipleRequest::decode(&request.service_request).unwrap();
                    processes.push(cov.subscriber_process_identifier);
                    cov.initiating_device_identifier
                }
                service => panic!("unexpected service {service:?}"),
            };
            assert_eq!(initiating_device, device());
        }
        processes.sort_unstable();
        assert_eq!(processes, vec![61, 62, 63]);
        assert_eq!(i_am, 2);
        assert_eq!(i_have, 1);
        wire.server.stop().await.unwrap();
    }
}

#[tokio::test]
async fn device_selection_preserves_no_device_and_wildcard_only_reads() {
    for instances in [vec![], vec![ObjectIdentifier::MAX_INSTANCE]] {
        let mut wire = Wire::start_with_devices(ServerConfig::default(), &instances).await;
        let property = PropertyIdentifier::OBJECT_IDENTIFIER;
        if instances.is_empty() {
            assert_eq!(
                wire.read(wildcard(), property, None).await,
                Err((ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))
            );
            assert!(
                matches!(wire.server.read_local(&wildcard(), property, None).await,
                Err(Error::Protocol { code, .. }) if code == ErrorCode::UNKNOWN_OBJECT.to_raw() as u32)
            );
            assert!(wire.server.broadcast_i_am().await.is_err());
        } else {
            let mut expected = BytesMut::new();
            encode_app_object_id(&mut expected, &wildcard());
            assert_eq!(
                wire.read(wildcard(), property, None).await.unwrap(),
                expected
            );
            assert_eq!(
                wire.server
                    .read_local(&wildcard(), property, None)
                    .await
                    .unwrap(),
                PropertyValue::ObjectIdentifier(wildcard())
            );
        }
        wire.server.stop().await.unwrap();
    }
}

fn assert_same_list(property: PropertyIdentifier, actual: &[u8], expected: &[u8]) {
    if property == MULTIPLE {
        assert_eq!(
            untimed(&decode_contexts(actual), &[300]),
            untimed(&decode_contexts(expected), &[300])
        );
    } else {
        let normalize = |bytes: &[u8]| {
            let mut entries = decode_subscriptions(bytes);
            for entry in &mut entries {
                if entry.recipient.process_identifier == 62 {
                    assert_eq!(entry.time_remaining, 300);
                    entry.time_remaining = 0;
                }
            }
            entries
        };
        assert_eq!(normalize(actual), normalize(expected));
    }
}

async fn unconfirmed(
    wire: &Wire,
    service_choice: UnconfirmedServiceChoice,
    service_request: BytesMut,
) {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice,
            service_request: service_request.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    wire.tx
        .send(ReceivedNpdu {
            direct_response: None,
            npdu: npdu.freeze(),
            source_mac: MacAddr::from_slice(&direct().mac),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
}
