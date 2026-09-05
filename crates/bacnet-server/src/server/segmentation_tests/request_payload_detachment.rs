//! Real incoming Bytes backing owners must not survive as saved payload owners.

use super::*;
use bacnet_encoding::npdu::{encode_npdu, Npdu};
use bacnet_transport::port::ReceivedNpdu;
use request_peer_quota::{assert_positive_ack, next_routed_apdu};
use request_reassembly::{
    inject_routed_segment, present_value, split_into, start_routed_reassembly_server,
    write_property_payload,
};
use std::sync::atomic::AtomicUsize;

struct InputOwner {
    bytes: Vec<u8>,
    drops: Arc<AtomicUsize>,
}

impl AsRef<[u8]> for InputOwner {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl Drop for InputOwner {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn large_backing_view(data: &[u8], drops: &Arc<AtomicUsize>) -> Bytes {
    let mut bytes = vec![0; 1024 * 1024];
    bytes[..data.len()].copy_from_slice(data);
    Bytes::from_owner(InputOwner {
        bytes,
        drops: Arc::clone(drops),
    })
    .slice(..data.len())
}

#[test]
fn request_payload_detachment_shared_view_negative_control() {
    let drops = Arc::new(AtomicUsize::new(0));
    let source = large_backing_view(b"payload", &drops);
    let shared = source.clone();
    let detached = Bytes::copy_from_slice(&source);
    drop(source);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(shared.as_ref(), b"payload");
    drop(shared);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(detached.as_ref(), b"payload");
}

#[tokio::test]
async fn request_payload_detachment_first_and_later_real_inputs_release_while_incomplete() {
    let (mut server, incoming, sent) = start_routed_reassembly_server().await;
    let router = test_mac(30);
    let remote = routed_address(400, 40);
    let text = "detached first and later saved payloads remain byte-exact";
    let chunks = split_into(&write_property_payload(text), 3);
    let drops = Arc::new(AtomicUsize::new(0));
    let mut observed = Vec::new();
    let mut index = 0;
    for seq in 0..2 {
        let mut encoded = BytesMut::new();
        encode_apdu(
            &mut encoded,
            &Apdu::ConfirmedRequest(ConfirmedRequestPdu {
                segmented: true,
                more_follows: true,
                segmented_response_accepted: true,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: 1,
                sequence_number: Some(seq),
                proposed_window_size: Some(1),
                service_choice: ConfirmedServiceChoice::WRITE_PROPERTY,
                service_request: Bytes::copy_from_slice(&chunks[seq as usize]),
            }),
        )
        .unwrap();
        let mut frame = BytesMut::new();
        encode_npdu(
            &mut frame,
            &Npdu {
                source: Some(remote.clone()),
                payload: encoded.freeze(),
                ..Npdu::default()
            },
        )
        .unwrap();
        incoming
            .send(ReceivedNpdu {
                npdu: large_backing_view(&frame, &drops),
                source_mac: router.clone(),
                link_layer_group: false,
                data_attributes: Vec::new(),
                reply_tx: None,
            })
            .await
            .unwrap();
        assert_positive_ack(
            next_routed_apdu(&sent, &mut index, &router, &remote).await,
            1,
            seq,
        );
        // A distinct empty request's ACK is a production-loop barrier: neither
        // the old ReceivedApdu nor its decoded request is still current input.
        inject_routed_segment(&incoming, &router, &remote, 10 + seq, 0, true, &[]).await;
        assert_positive_ack(
            next_routed_apdu(&sent, &mut index, &router, &remote).await,
            10 + seq,
            0,
        );
        observed.push(drops.load(Ordering::SeqCst));
    }
    assert_eq!(present_value(&server).await, "");
    inject_routed_segment(&incoming, &router, &remote, 1, 2, false, &chunks[2]).await;
    assert_positive_ack(
        next_routed_apdu(&sent, &mut index, &router, &remote).await,
        1,
        2,
    );
    let reply = next_routed_apdu(&sent, &mut index, &router, &remote).await;
    let value = present_value(&server).await;
    server.stop().await.unwrap();
    assert!(matches!(reply, Apdu::SimpleAck(ack) if ack.invoke_id == 1));
    assert_eq!(value, text);
    assert_eq!(
        observed,
        [1, 2],
        "saved first/template and later payloads must detach"
    );
}
