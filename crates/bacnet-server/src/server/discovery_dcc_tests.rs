//! Solicited discovery responses remain available during DISABLE_INITIATION
//! (#1590), with ordinary matching, routing and limiter admission. Independent
//! I-Am announcements remain restricted (#1388).
//!
//! Each test enters the state through an acknowledged one-minute DCC request.
//! Transition cases restore initiation through timer expiry or DCC ENABLE.
//! Requests come from the wire harness peer, and the server answers it
//! directly or through a router.
use super::cov_wire_test_support::*;
use super::test_transport::SendLog;
use super::*;
use bacnet_encoding::apdu::encode_apdu;
use bacnet_encoding::npdu::{encode_npdu, Npdu, NpduAddress};
use bacnet_services::who_has::{IHaveRequest, WhoHasObject, WhoHasRequest};
use bacnet_services::who_is::DeviceInstanceRange;
use bacnet_transport::port::{ReceivedNpdu, TransportProvenance};
use bacnet_types::enums::EnableDisable;

/// The harness server, with initiation restricted for one minute by a DCC
/// request it has acknowledged.
async fn under_disable_initiation() -> Harness {
    under_disable_initiation_with(DiscoveryPolicy::default()).await
}

/// [`under_disable_initiation`] with the discovery limiter on `policy`.
async fn under_disable_initiation_with(policy: DiscoveryPolicy) -> Harness {
    let mut h = Harness::start(ServerConfig {
        dcc_policy: DccPolicy::LegacyPermissive,
        discovery_policy: policy,
        ..Default::default()
    })
    .await;
    h.dcc(EnableDisable::DISABLE_INITIATION, Some(1)).await;
    assert_eq!(response(&h).await, Ok(()));
    assert_eq!(h.server.comm_state(), DccState::DisableInitiation);
    h
}

/// Let the DCC request's minute run out.
async fn timer_expires(h: &Harness) {
    tokio::time::advance(Duration::from_secs(60)).await;
    h.settle().await;
    assert_eq!(
        h.server.comm_state(),
        DccState::Enable,
        "the timer enabled initiation"
    );
}

/// Enable initiation again with an acknowledged DCC ENABLE.
async fn enable(h: &mut Harness) {
    h.dcc(EnableDisable::ENABLE, None).await;
    assert_eq!(response(h).await, Ok(()));
    assert_eq!(h.server.comm_state(), DccState::Enable);
}

/// Whether `error` is the refusal of an I-Am announcement under DCC.
fn communication_disabled(error: &Error) -> bool {
    matches!(error, Error::Protocol { class, code }
        if *class == ErrorClass::SERVICES.to_raw() as u32
            && *code == ErrorCode::COMMUNICATION_DISABLED.to_raw() as u32)
}

/// Deliver an unconfirmed request from the harness peer and let the server
/// run everything it makes ready.
async fn unconfirmed(h: &Harness, service_choice: UnconfirmedServiceChoice, body: BytesMut) {
    h.respond(Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
        service_choice,
        service_request: body.freeze(),
    }))
    .await;
    h.settle().await;
}

/// A Who-Has for AV-1 by name, from any device.
async fn who_has_av1(h: &Harness) {
    who_has(h, WhoHasObject::Name("AV-1".into()), None).await;
}

async fn who_has(h: &Harness, object: WhoHasObject, range: Option<DeviceInstanceRange>) {
    let mut body = BytesMut::new();
    WhoHasRequest { range, object }.encode(&mut body).unwrap();
    unconfirmed(h, UnconfirmedServiceChoice::WHO_HAS, body).await;
}

/// A Who-Is for every device.
async fn who_is(h: &Harness) {
    let mut body = BytesMut::new();
    WhoIsRequest { range: None }.encode(&mut body);
    unconfirmed(h, UnconfirmedServiceChoice::WHO_IS, body).await;
}

/// Take every unconfirmed request the server has sent the peer, in order.
fn sent_to_peer(h: &Harness) -> Vec<UnconfirmedRequestPdu> {
    let mut frames = h.frames.lock().unwrap();
    let mut taken = Vec::new();
    frames.retain(|apdu| match apdu {
        Apdu::UnconfirmedRequest(request) => {
            taken.push(request.clone());
            false
        }
        _ => true,
    });
    taken
}

fn assert_i_have(h: &Harness) {
    let sent = sent_to_peer(h);
    assert_eq!(sent.len(), 1, "expected one matching I-Have: {sent:?}");
    assert_eq!(sent[0].service_choice, UnconfirmedServiceChoice::I_HAVE);
    let answer = IHaveRequest::decode(&sent[0].service_request).unwrap();
    assert_eq!(answer.device_identifier.instance_number(), 856);
    assert_eq!(answer.object_identifier, av1());
    assert_eq!(answer.object_name, "AV-1");
}

/// How many I-Am broadcasts the transport has carried.
fn i_am_broadcasts(log: &SendLog) -> usize {
    log.broadcasts()
        .iter()
        .filter(|frame| {
            matches!(frame.apdu(), Apdu::UnconfirmedRequest(request)
                if request.service_choice == UnconfirmedServiceChoice::I_AM)
        })
        .count()
}

#[tokio::test(start_paused = true)]
async fn disable_initiation_answers_matching_who_has_and_who_is() {
    let h = under_disable_initiation().await;
    who_has_av1(&h).await;
    assert_i_have(&h);
    who_is(&h).await;
    let sent: Vec<_> = sent_to_peer(&h)
        .into_iter()
        .map(|request| request.service_choice)
        .collect();
    assert_eq!(sent, [UnconfirmedServiceChoice::I_AM]);
    let counters = h.server.discovery_counters();
    assert_eq!((counters.i_am_sent, counters.i_have_sent), (1, 1));
}

#[tokio::test(start_paused = true)]
async fn dcc_expiry_preserves_who_has_responses_and_duplicate_coalescing() {
    // A DCC transition does not erase the earlier response's coalescing entry.
    let h = under_disable_initiation_with(DiscoveryPolicy {
        coalesce_window: Duration::from_secs(3600),
        ..DiscoveryPolicy::default()
    })
    .await;
    who_has_av1(&h).await;
    assert_i_have(&h);

    // The same request again, from the same peer.
    timer_expires(&h).await;
    who_has_av1(&h).await;
    assert!(sent_to_peer(&h).is_empty());
    assert_eq!(h.server.discovery_counters().requests_coalesced, 1);
    // A different lookup is eligible even though the name remains coalesced.
    who_has(&h, WhoHasObject::Identifier(av1()), None).await;
    assert_i_have(&h);
    assert_eq!(h.server.discovery_counters().i_have_sent, 2);
}

#[tokio::test(start_paused = true)]
async fn broadcast_i_am_fails_under_disable_initiation_and_sends_once_enabled() {
    let h = under_disable_initiation().await;
    let log = h.server.test_network().transport().sent();
    let refused = h.server.broadcast_i_am().await.unwrap_err();
    assert!(communication_disabled(&refused), "{refused:?}");
    assert_eq!(i_am_broadcasts(&log), 0);
    assert_eq!(h.server.discovery_counters().i_am_sent, 0);

    timer_expires(&h).await;
    h.server.broadcast_i_am().await.unwrap();
    assert_eq!(i_am_broadcasts(&log), 1);
    assert_eq!(h.server.discovery_counters().i_am_sent, 1);
}

#[tokio::test(start_paused = true)]
async fn dcc_enable_preserves_who_has_responses_and_duplicate_coalescing() {
    let mut h = under_disable_initiation_with(DiscoveryPolicy {
        coalesce_window: Duration::from_secs(3600),
        ..DiscoveryPolicy::default()
    })
    .await;
    who_has_av1(&h).await;
    assert_i_have(&h);
    assert_eq!(h.server.discovery_counters().requests_coalesced, 0);

    enable(&mut h).await;
    who_has_av1(&h).await;
    assert!(sent_to_peer(&h).is_empty());
    who_has(&h, WhoHasObject::Identifier(av1()), None).await;
    assert_i_have(&h);
    let counters = h.server.discovery_counters();
    assert_eq!((counters.i_have_sent, counters.requests_coalesced), (2, 1));
}

#[tokio::test(start_paused = true)]
async fn an_i_am_broadcaster_handle_is_refused_under_disable_initiation() {
    let mut h = under_disable_initiation().await;
    let announcer = h.server.i_am_broadcaster();
    let log = h.server.test_network().transport().sent();
    let refused = announcer.broadcast_i_am().await.unwrap_err();
    assert!(communication_disabled(&refused), "{refused:?}");
    assert_eq!(i_am_broadcasts(&log), 0);

    enable(&mut h).await;
    announcer.broadcast_i_am().await.unwrap();
    assert_eq!(i_am_broadcasts(&log), 1);
}

#[tokio::test(start_paused = true)]
async fn disable_initiation_preserves_who_has_id_name_and_device_range_matching() {
    let h = under_disable_initiation().await;
    let missing =
        ObjectIdentifier::new(bacnet_types::enums::ObjectType::ANALOG_VALUE, 999).unwrap();
    for (object, limits, matches) in [
        (WhoHasObject::Identifier(missing), None, false),
        (WhoHasObject::Name("missing-object".into()), None, false),
        (WhoHasObject::Identifier(av1()), Some((900, 1000)), false),
        (WhoHasObject::Name("AV-1".into()), Some((1, 855)), false),
        (WhoHasObject::Identifier(av1()), Some((856, 856)), true),
        (WhoHasObject::Name("AV-1".into()), Some((800, 900)), true),
    ] {
        let range = limits.map(|(low, high)| {
            DeviceInstanceRange::from_limits(Some(low), Some(high))
                .unwrap()
                .unwrap()
        });
        who_has(&h, object, range).await;
        if matches {
            assert_i_have(&h);
        } else {
            assert!(
                sent_to_peer(&h).is_empty(),
                "an ineligible lookup must not respond"
            );
        }
    }
    let counters = h.server.discovery_counters();
    assert_eq!((counters.who_has_received, counters.i_have_sent), (6, 2));
    assert_eq!(counters.requests_coalesced, 0);
}

#[tokio::test(start_paused = true)]
async fn disable_initiation_i_have_consumes_budget_and_coalesces_duplicates() {
    let h = under_disable_initiation_with(DiscoveryPolicy {
        max_responses_per_sec_global: 1,
        max_responses_per_sec_per_source: 1,
        global_burst_capacity: 1,
        source_burst_capacity: 1,
        reserved_capacity: 0,
        coalesce_window: Duration::from_secs(5),
        ..Default::default()
    })
    .await;
    who_has_av1(&h).await;
    assert_i_have(&h);
    who_has_av1(&h).await;
    assert!(sent_to_peer(&h).is_empty());
    who_has(&h, WhoHasObject::Identifier(av1()), None).await;
    assert!(sent_to_peer(&h).is_empty());
    let limited = h.server.discovery_counters();
    assert_eq!(limited.i_have_sent, 1);
    assert_eq!(limited.requests_coalesced, 1);
    assert_eq!(limited.responses_throttled_global, 1);

    tokio::time::advance(Duration::from_secs(1)).await;
    who_has(&h, WhoHasObject::Identifier(av1()), None).await;
    assert_i_have(&h);
    assert_eq!(h.server.discovery_counters().i_have_sent, 2);
    assert_eq!(h.server.comm_state(), DccState::DisableInitiation);
}

/// Inject a group-delivered Who-Has with optional routed source information.
async fn who_has_from(h: &Harness, source_mac: &[u8], routed: Option<NpduAddress>) {
    let mut body = BytesMut::new();
    WhoHasRequest {
        range: None,
        object: WhoHasObject::Name("AV-1".into()),
    }
    .encode(&mut body)
    .unwrap();
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::UnconfirmedRequest(UnconfirmedRequestPdu {
            service_choice: UnconfirmedServiceChoice::WHO_HAS,
            service_request: body.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            source: routed,
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    h.tx.send(ReceivedNpdu {
        direct_response: None,
        npdu: npdu.freeze(),
        source_mac: MacAddr::from_slice(source_mac),
        link_layer_group: true,
        data_attributes: Vec::new(),
        provenance: TransportProvenance::unverified(),
        reply_tx: None,
    })
    .await
    .unwrap();
    h.settle().await;
}

#[tokio::test(start_paused = true)]
async fn disable_initiation_i_have_uses_directed_and_routed_reply_addresses() {
    let h = under_disable_initiation().await;
    let log = h.server.test_network().transport().sent();
    log.clear();
    who_has_from(&h, &PEER, None).await;
    assert_i_have(&h);
    let local = log.take();
    assert_eq!(local.len(), 1);
    assert!(!local[0].broadcast);
    assert_eq!(local[0].mac.as_slice(), &PEER);
    assert!(local[0].decode_npdu().destination.is_none());

    let router = [10, 0, 0, 254, 0xBA, 0xC0];
    let remote = NpduAddress {
        network: 200,
        mac_address: MacAddr::from_slice(&[0x11, 0x22]),
    };
    who_has_from(&h, &router, Some(remote.clone())).await;
    assert_i_have(&h);
    let routed = log.take();
    assert_eq!(routed.len(), 1);
    assert!(!routed[0].broadcast);
    assert_eq!(routed[0].mac.as_slice(), &router);
    assert_eq!(routed[0].decode_npdu().destination, Some(remote));
    let counters = h.server.discovery_counters();
    assert_eq!(
        (counters.i_have_sent, counters.directed_responses_sent),
        (2, 2)
    );
    assert_eq!(h.server.comm_state(), DccState::DisableInitiation);
}
