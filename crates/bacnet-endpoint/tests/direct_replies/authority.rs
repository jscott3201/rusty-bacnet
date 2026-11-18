use super::*;
use bacnet_transport::port::TransportProvenance;
use tokio::sync::oneshot;

async fn replacement(client: bool) {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut consumer = Consumer::start(client, port).await;
    let mut a = f.peer(ca.tls("A"), None).await;
    let request = if client { unsupported(1) } else { read_name(1) };
    let held_a = f.capture(&mut a, &request, routed()).await;
    let identity_a = held_a.provenance.direct_sc_identity().unwrap();
    // A worked before retirement, including routed response framing.
    f.feed(held_a.clone()).await;
    assert_eq!(f.response(&a).await.0.destination, routed());
    let mut b = f.peer(ca.tls("B"), None).await;
    let fresh_b = f.capture(&mut b, &unsupported(3), None).await;
    let identity_b = fresh_b.provenance.direct_sc_identity().unwrap();
    assert_ne!(identity_a.leaf_sha256(), identity_b.leaf_sha256());
    assert_ne!(identity_a.incarnation(), identity_b.incarnation());
    // The listener has committed B (its NPDU was admitted). A's already
    // admitted complete work retains its old route, never B's current one.
    f.feed(held_a).await;
    f.feed(fresh_b).await;
    assert!(matches!(f.response(&b).await.1, Apdu::Reject(r) if r.invoke_id == 3));
    f.barrier(&mut b, 4).await;
    consumer.stop().await;
    f.listener.stop().await;
}
#[tokio::test]
async fn client_queued_a_reply_never_falls_back_after_b_replaces_it() {
    replacement(true).await;
}
#[tokio::test]
async fn endpoint_queued_a_reply_never_falls_back_after_b_replaces_it() {
    replacement(false).await;
}

async fn mixed_envelopes(client: bool) {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut consumer = Consumer::start(client, port).await;
    let mut a = f.peer(ca.tls("A"), None).await;
    let from_a = f.capture(&mut a, &unsupported(10), None).await;
    let mut b = f.peer(ca.tls("B"), None).await;
    let from_b = f.capture(&mut b, &unsupported(11), None).await;
    for case in 0..3 {
        let mut mixed = from_a.clone();
        match case {
            0 => mixed.direct_response = None,
            1 => mixed.direct_response = from_b.direct_response.clone(),
            2 => {
                mixed = from_b.clone();
                mixed.provenance = TransportProvenance::unverified();
            }
            _ => unreachable!(),
        }
        let (reply_tx, reply_rx) = oneshot::channel();
        mixed.reply_tx = Some(reply_tx);
        f.feed(mixed).await;
        f.barrier(&mut b, 20 + case).await;
        assert!(
            bounded(reply_rx).await.is_err(),
            "mixed authority bypassed checks through reply_tx"
        );
    }
    // Even a valid direct envelope must not take the prompt channel shortcut.
    let mut valid = from_b;
    let (reply_tx, reply_rx) = oneshot::channel();
    valid.reply_tx = Some(reply_tx);
    f.feed(valid).await;
    assert!(matches!(f.response(&b).await.1, Apdu::Reject(r) if r.invoke_id == 11));
    assert!(bounded(reply_rx).await.is_err());
    consumer.stop().await;
    f.listener.stop().await;
}
#[tokio::test]
async fn client_mixed_prompt_channels_cannot_bypass_direct_authority() {
    mixed_envelopes(true).await;
}
#[tokio::test]
async fn endpoint_mixed_prompt_channels_cannot_bypass_direct_authority() {
    mixed_envelopes(false).await;
}

async fn ordinary_prompt(client: bool) {
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut consumer = Consumer::start(client, port).await;
    let mut peer = f.peer(ca.tls("A"), None).await;
    let mut ordinary = f.capture(&mut peer, &unsupported(30), routed()).await;
    ordinary.provenance = TransportProvenance::unverified();
    ordinary.direct_response = None;
    let (reply_tx, reply_rx) = oneshot::channel();
    let mut prompt = ordinary.clone();
    prompt.reply_tx = Some(reply_tx);
    f.feed(prompt).await;
    let npdu = bacnet_encoding::npdu::decode_npdu(bounded(reply_rx).await.unwrap()).unwrap();
    assert_eq!(npdu.destination, routed());
    assert!(
        matches!(bacnet_encoding::apdu::decode_apdu(npdu.payload).unwrap(), Apdu::Reject(r) if r.invoke_id == 30)
    );
    let (reply_tx, reply_rx) = oneshot::channel();
    drop(reply_rx);
    ordinary.reply_tx = Some(reply_tx);
    f.feed(ordinary).await;
    if client {
        let fallback = bounded(f.generic.recv()).await.unwrap();
        assert_eq!(fallback.destination, routed());
        assert!(
            matches!(bacnet_encoding::apdu::decode_apdu(fallback.payload).unwrap(), Apdu::Reject(r) if r.invoke_id == 30)
        );
    }
    // Endpoint ordinary failed prompt completion deliberately has NO fallback.
    f.barrier(&mut peer, 31).await;
    consumer.stop().await;
    f.listener.stop().await;
}
#[tokio::test]
async fn ordinary_client_failed_prompt_falls_back_with_route() {
    ordinary_prompt(true).await;
}
#[tokio::test]
async fn ordinary_endpoint_failed_prompt_does_not_fall_back() {
    ordinary_prompt(false).await;
}

#[tokio::test]
async fn both_consumers_drop_group_direct_replies() {
    for client in [true, false] {
        let ca = TestCa::new();
        let (mut f, port) = Fixture::new(&ca).await;
        let mut consumer = Consumer::start(client, port).await;
        let mut peer = f.peer(ca.tls("group"), None).await;
        let mut group = f.capture(&mut peer, &unsupported(40), None).await;
        group.link_layer_group = true;
        let (tx, rx) = oneshot::channel();
        group.reply_tx = Some(tx);
        f.feed(group).await;
        f.barrier(&mut peer, 41).await;
        assert!(bounded(rx).await.is_err());
        consumer.stop().await;
        f.listener.stop().await;
    }
}

/// The session's server sends its direct answer with no data attributes, as
/// the narrow responder did, and its ordinary answer with none either: the
/// narrow responder copied the request's onto that one, the full server
/// does not (#2617).
#[tokio::test]
async fn the_servers_answers_carry_no_data_attributes() {
    use bacnet_transport::port::DataAttribute;
    let ca = TestCa::new();
    let (mut f, port) = Fixture::new(&ca).await;
    let mut session = EndpointSession::new(port, SessionRole::Both, SessionConfig::default())
        .unwrap()
        .device(node())
        .with_database(database("attributes"));
    session.start().await.unwrap();
    let mut peer = f.peer(ca.tls("A"), None).await;
    let attributes = vec![DataAttribute {
        option_type: 3,
        must_understand: false,
        data: vec![4, 5],
    }];
    let mut direct = f.capture(&mut peer, &read_name(60), None).await;
    direct.data_attributes = attributes.clone();
    f.feed(direct).await;
    // `response` checks the frame's data options are empty.
    assert!(matches!(f.response(&peer).await.1, Apdu::ComplexAck(a) if a.invoke_id == 60));
    let mut ordinary = f.capture(&mut peer, &read_name(61), routed()).await;
    ordinary.provenance = TransportProvenance::unverified();
    ordinary.direct_response = None;
    ordinary.data_attributes = attributes;
    f.feed(ordinary).await;
    let response = bounded(f.generic.recv()).await.unwrap();
    assert_eq!(response.destination, routed());
    assert!(
        matches!(bacnet_encoding::apdu::decode_apdu(response.payload).unwrap(), Apdu::ComplexAck(a) if a.invoke_id == 61)
    );
    assert!(f.attributes.lock().unwrap().is_empty());
    f.barrier(&mut peer, 62).await;
    session.stop().await.unwrap();
    f.listener.stop().await;
}
