//! A station that claims a Device instance in an I-Am the server did not ask
//! for is not taken for that device (#2825): a write it makes names its own
//! address as the command's source.
use super::*;
use bacnet_services::who_is::IAmRequest;
use bacnet_types::enums::Segmentation;

/// Deliver Device `instance`'s I-Am as `peer` sent it, unasked.
async fn i_am_from(wire: &Wire, peer: &Peer, instance: u32) {
    let mut service = BytesMut::new();
    IAmRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::DEVICE, instance).unwrap(),
        max_apdu_length: 1476,
        segmentation_supported: Segmentation::NO_SEGMENTATION,
        vendor_id: 260,
    }
    .encode(&mut service);
    let mut payload = BytesMut::new();
    encode_apdu(
        &mut payload,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::I_AM,
            service_request: service.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    bacnet_encoding::npdu::encode_npdu(
        &mut npdu,
        &bacnet_encoding::npdu::Npdu {
            source: peer.network.clone(),
            payload: payload.freeze(),
            ..bacnet_encoding::npdu::Npdu::default()
        },
    )
    .unwrap();
    wire.tx
        .send(bacnet_transport::port::ReceivedNpdu {
            direct_response: None,
            npdu: npdu.freeze(),
            source_mac: MacAddr::from_slice(&peer.mac),
            link_layer_group: false,
            data_attributes: Vec::new(),
            provenance: bacnet_transport::port::TransportProvenance::unverified(),
            reply_tx: None,
        })
        .await
        .unwrap();
    // The I-Am is handled on a task of its own.
    tokio::time::sleep(Duration::from_millis(10)).await;
}

#[tokio::test(start_paused = true)]
async fn an_unsolicited_i_am_does_not_name_a_writers_source() {
    for stranger in [direct(), routed()] {
        let mut wire = Wire::start(ServerConfig::default()).await;
        i_am_from(&wire, &stranger, 9).await;
        simple_ack(
            wire.send(&stranger, wp(av(1), PV, PropertyValue::Real(31.0), 8))
                .await,
        );
        assert_eq!(
            source(&mut wire, av(1), None).await,
            address_source(&stranger)
        );
        wire.server.stop().await.unwrap();
    }
}
