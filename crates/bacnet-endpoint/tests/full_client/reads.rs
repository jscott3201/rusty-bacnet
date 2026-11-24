//! Discovery and reads through the full client, and the high-level calls a
//! standalone client has.
use super::*;
use bacnet_services::file::{FileAccessMethod, FileReadAckMethod};
use bacnet_types::constructed::{PropertyReference, ReadAccessSpecification};
use bacnet_types::enums::PropertyIdentifier;

#[tokio::test(start_paused = true)]
async fn who_is_fills_the_full_clients_table_in_each_scope() {
    let lab = lab(SessionRole::Both).await;
    let client = lab.session.client().unwrap();
    let peer = Wire::mac(PEER);
    let mut events = client.device_events();

    // To every network: the peer's I-Am is a broadcast, which the node's
    // server role hears too.
    client.who_is(Broadcast::Global, None).await.unwrap();
    eventually(WAIT, "the peer's I-Am in the table", async || {
        client.get_device(PEER_DEVICE).await.unwrap().is_some()
    })
    .await;
    let row = client.get_device(PEER_DEVICE).await.unwrap().unwrap();
    assert_eq!(row.mac_address, peer);
    assert_eq!(row.segmentation_supported, Segmentation::SEGMENTED_BOTH);
    let first = tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        first.kind,
        bacnet_client::client::DeviceEventKind::Discovered
    );

    // To the one device, once the peer no longer takes a second Who-Is as a
    // repeat of the first: another I-Am, another event for the same row.
    tokio::time::sleep(Duration::from_secs(30)).await;
    client.who_is_directed(&peer, None).await.unwrap();
    let again = tokio::time::timeout(WAIT, events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(again.kind, bacnet_client::client::DeviceEventKind::Updated);
    assert_eq!(client.discovered_devices().await.len(), 1);

    // To one remote network: the question leaves with that DNET. No router
    // is on this link, so nothing answers.
    client.who_is(Broadcast::Network(7), None).await.unwrap();
    let last = lab.node_sent.frames().pop().unwrap();
    assert!(last.broadcast);
    assert_eq!(last.decode_npdu().destination.unwrap().network, 7);

    // A Who-Is from another node reaches both roles and upsets neither.
    let seen = lab.session.client_seat_counters().apdus;
    let other = other_client(&lab.wire).await;
    other.who_is(Broadcast::Global, None).await.unwrap();
    eventually(WAIT, "the Who-Is copied to the seat", async || {
        lab.session.client_seat_counters().apdus > seen
    })
    .await;
    assert!(lab.session.is_running());
    assert_eq!(lab.session.client_seat_counters().full_drops, 0);
    drop((client, other));
    stop(lab).await;
}

#[tokio::test(start_paused = true)]
async fn the_full_client_reads_as_a_standalone_client_does() {
    let lab = lab(SessionRole::Both).await;
    let client = lab.session.client().unwrap();
    let peer = Wire::mac(PEER);

    let value = client
        .read_property(&peer, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
        .await
        .unwrap();
    assert_eq!(
        value.property_value,
        [&[0x44][..], &21.5f32.to_be_bytes()].concat()
    );

    let both = client
        .read_property_multiple(
            &peer,
            vec![ReadAccessSpecification {
                object_identifier: ai1(),
                list_of_property_references: vec![
                    PropertyReference {
                        property_identifier: PropertyIdentifier::PRESENT_VALUE,
                        property_array_index: None,
                    },
                    PropertyReference {
                        property_identifier: PropertyIdentifier::OBJECT_NAME,
                        property_array_index: None,
                    },
                ],
            }],
        )
        .await
        .unwrap();
    assert_eq!(both.list_of_read_access_results.len(), 1);
    assert_eq!(both.list_of_read_access_results[0].list_of_results.len(), 2);

    // The Object_List is longer than one APDU: the answer comes in segments,
    // each of which is the full client's, and it reassembles them.
    let list = client
        .read_property(
            &peer,
            oid(ObjectType::DEVICE, PEER_DEVICE),
            PropertyIdentifier::OBJECT_LIST,
            None,
        )
        .await
        .unwrap();
    assert_eq!(list.property_value.len(), 5 * 403);
    let segments = apdus(&lab.peer_sent)
        .into_iter()
        .filter(|(_, apdu)| matches!(apdu, Apdu::ComplexAck(ack) if ack.segmented))
        .count();
    assert!(segments >= 2, "{segments} segments");

    // A file block, short and then longer than one APDU.
    for (start, count) in [(10, 100), (0, 2_500)] {
        let block = client
            .atomic_read_file_decoded(
                &peer,
                oid(ObjectType::FILE, 1),
                FileAccessMethod::Stream {
                    file_start_position: start,
                    requested_octet_count: count,
                },
            )
            .await
            .unwrap();
        assert!(!block.end_of_file);
        let FileReadAckMethod::Stream {
            file_start_position,
            file_data,
        } = block.access
        else {
            panic!("a stream block");
        };
        assert_eq!(file_start_position, start);
        let (from, to) = (start as usize, start as usize + count as usize);
        assert_eq!(file_data, file_octets()[from..to]);
    }

    // The client's link is the node's own transport, started once.
    assert_eq!(client.transport().handle().starts(), 1);
    assert_eq!(client.local_mac(), Wire::mac(NODE).as_slice());
    assert_eq!(lab.node_link.starts(), 1);
    // Every answer found its owner: the session counted none unclaimed.
    assert_eq!(lab.session.policy_counters().unclaimed_terminal, 0);
    assert_eq!(lab.session.active_leases(), 0);

    drop(client);
    stop(lab).await;
}
