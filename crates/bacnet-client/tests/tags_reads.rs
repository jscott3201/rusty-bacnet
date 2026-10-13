//! Actual RP/RPM exchange with independently encoded Tags payloads.
use bacnet_client::client::BACnetClient;
use bacnet_client::tags::{decode_tags_read, TagsRead};
use bacnet_encoding::apdu::{decode_apdu, Apdu};
use bacnet_network::layer::NetworkLayer;
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::rpm::ReadPropertyMultipleRequest;
use bacnet_transport::bip::BipTransport;
use bacnet_types::constructed::{BACnetNameValue, PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::{
    ConfirmedServiceChoice as S, ErrorClass, ErrorCode, NetworkPriority, ObjectType,
    PropertyIdentifier as P,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};
use std::net::Ipv4Addr;
use tokio::time::{timeout, Duration};

const SEMANTIC: &[u8] = &[0x0a, 0, b'a'];
const VALUE: &[u8] = &[0x0a, 0, b'b', 0x21, 3];
const IDENTITY: &[u8] = &[0x0c, 0x0f, 0xc0, 0, 1, 0x1a, 1, 0xe6];

#[tokio::test]
async fn tags_reads_preserve_raw_acks_and_type_successes_without_hiding_rpm_errors() {
    let oid = ObjectIdentifier::new(ObjectType::COLOR, 1).unwrap();
    let mut client = BACnetClient::bip_builder()
        .interface(Ipv4Addr::LOCALHOST)
        .port(0)
        .apdu_timeout_ms(2000)
        .build()
        .await
        .unwrap();
    let mut peer = NetworkLayer::new(BipTransport::new(
        Ipv4Addr::LOCALHOST,
        0,
        Ipv4Addr::BROADCAST,
    ));
    let mut incoming = peer.start().await.unwrap();
    let address = peer.local_mac().to_vec();
    let whole = [SEMANTIC, VALUE].concat();
    let cases = vec![
        (None, whole.clone()),
        (Some(1), SEMANTIC.to_vec()),
        (Some(0), vec![0x21, 2]),
        (None, vec![]),
        (Some(0), vec![0x21, 0]),
    ];
    let replies = cases.clone();
    let responder = tokio::spawn(async move {
        for (index, value) in replies {
            let received = timeout(Duration::from_secs(2), incoming.recv())
                .await
                .unwrap()
                .unwrap();
            let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu).unwrap() else {
                panic!("expected RP");
            };
            assert_eq!(request.service_choice, S::READ_PROPERTY);
            let decoded = ReadPropertyRequest::decode(&request.service_request).unwrap();
            assert_eq!(
                (
                    decoded.object_identifier,
                    decoded.property_identifier,
                    decoded.property_array_index
                ),
                (oid, P::TAGS, index)
            );
            let mut payload = IDENTITY.to_vec();
            if let Some(index) = index {
                payload.extend([0x29, index as u8]);
            }
            assert_eq!(request.service_request.as_ref(), payload);
            payload.push(0x3e);
            payload.extend(value);
            payload.push(0x3f);
            let response = [vec![0x30, request.invoke_id, 12], payload].concat();
            peer.send_apdu(
                &response,
                &received.source_mac,
                false,
                NetworkPriority::NORMAL,
            )
            .await
            .unwrap();
        }
        let received = timeout(Duration::from_secs(2), incoming.recv())
            .await
            .unwrap()
            .unwrap();
        let Apdu::ConfirmedRequest(request) = decode_apdu(received.apdu).unwrap() else {
            panic!("expected RPM");
        };
        assert_eq!(request.service_choice, S::READ_PROPERTY_MULTIPLE);
        let decoded = ReadPropertyMultipleRequest::decode(&request.service_request).unwrap();
        assert_eq!(decoded.list_of_read_access_specs.len(), 1);
        let spec = &decoded.list_of_read_access_specs[0];
        assert_eq!(spec.object_identifier, oid);
        assert_eq!(
            spec.list_of_property_references
                .iter()
                .map(|p| (p.property_identifier, p.property_array_index))
                .collect::<Vec<_>>(),
            vec![
                (P::TAGS, None),
                (P::TAGS, Some(1)),
                (P::TAGS, Some(0)),
                (P::DESCRIPTION, None)
            ]
        );
        // Object [0], results [1], Tags [2], optional index [3], value [4].
        let payload = [
            &[0x0c, 0x0f, 0xc0, 0, 1, 0x1e, 0x2a, 1, 0xe6, 0x4e][..],
            &whole,
            &[0x4f, 0x2a, 1, 0xe6, 0x39, 1, 0x4e],
            SEMANTIC,
            &[0x4f, 0x2a, 1, 0xe6, 0x39, 0, 0x4e, 0x21, 2, 0x4f],
            // Description error: PROPERTY / UNKNOWN_PROPERTY.
            &[0x29, 28, 0x5e, 0x91, 2, 0x91, 32, 0x5f, 0x1f],
        ]
        .concat();
        let response = [vec![0x30, request.invoke_id, 14], payload].concat();
        peer.send_apdu(
            &response,
            &received.source_mac,
            false,
            NetworkPriority::NORMAL,
        )
        .await
        .unwrap();
        peer.stop().await.unwrap();
    });
    for (index, bytes) in cases {
        let ack = client
            .read_property(&address, oid, P::TAGS, index)
            .await
            .unwrap();
        assert_eq!(
            (
                ack.object_identifier,
                ack.property_identifier,
                ack.property_array_index
            ),
            (oid, P::TAGS, index)
        );
        assert_eq!(ack.property_value, bytes);
        let expected = match index {
            None if bytes.is_empty() => TagsRead::Whole(vec![]),
            None => TagsRead::Whole(vec![
                BACnetNameValue::semantic("a"),
                BACnetNameValue::valued("b", PropertyValue::Unsigned(3)),
            ]),
            Some(0) => TagsRead::Size(u32::from(bytes[1])),
            Some(_) => TagsRead::Element(BACnetNameValue::semantic("a")),
        };
        assert_eq!(
            decode_tags_read(ack.property_array_index, &ack.property_value).unwrap(),
            expected
        );
    }
    let specs = vec![ReadAccessSpecification {
        object_identifier: oid,
        list_of_property_references: [
            (P::TAGS, None),
            (P::TAGS, Some(1)),
            (P::TAGS, Some(0)),
            (P::DESCRIPTION, None),
        ]
        .into_iter()
        .map(
            |(property_identifier, property_array_index)| PropertyReference {
                property_identifier,
                property_array_index,
            },
        )
        .collect(),
    }];
    let ack = client
        .read_property_multiple(&address, specs)
        .await
        .unwrap();
    let result = &ack.list_of_read_access_results[0];
    assert_eq!(result.object_identifier, oid);
    let rows = &result.list_of_results;
    assert_eq!(rows.len(), 4);
    assert_eq!(
        decode_tags_read(
            rows[0].property_array_index,
            rows[0].property_value.as_ref().unwrap()
        )
        .unwrap(),
        TagsRead::Whole(vec![
            BACnetNameValue::semantic("a"),
            BACnetNameValue::valued("b", PropertyValue::Unsigned(3))
        ])
    );
    assert_eq!(
        decode_tags_read(
            rows[1].property_array_index,
            rows[1].property_value.as_ref().unwrap()
        )
        .unwrap(),
        TagsRead::Element(BACnetNameValue::semantic("a"))
    );
    assert_eq!(
        decode_tags_read(
            rows[2].property_array_index,
            rows[2].property_value.as_ref().unwrap()
        )
        .unwrap(),
        TagsRead::Size(2)
    );
    assert_eq!(
        rows[3].error,
        Some((ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY))
    );
    assert_eq!(rows[3].property_value, None);
    client.stop().await.unwrap();
    responder.await.unwrap();
}
