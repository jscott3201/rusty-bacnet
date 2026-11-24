//! Each lane's share of the source Audit slots, on the capture link
//! (#2652): a burst to one station, or to the stations of one network,
//! leaves the rest of the slots to the others, and the minimum request
//! interval holds between requests admitted together.
use super::*;
use crate::source_audit::slots::{LANE_SHARE, SLOTS};
use tokio::time::Instant;

/// A station that never answers.
const SILENT: u8 = 20;

/// The confirmed requests among the next `frames` the session sends, each
/// with when it went. The others must be Audit records for `SINK`.
async fn requests_among(
    endpoint: &mut Endpoint,
    frames: usize,
) -> Vec<(Instant, ConfirmedRequest)> {
    let mut requests = Vec::new();
    for _ in 0..frames {
        let sent = endpoint.next().await;
        let npdu = decode_npdu(sent.npdu.clone()).unwrap();
        match decode_apdu(npdu.payload).unwrap() {
            Apdu::ConfirmedRequest(request) => requests.push((Instant::now(), request)),
            _ => assert_eq!(record(sent, SINK).operation, AuditOperation::READ),
        }
    }
    requests
}

/// 64 audited reads at once to a station that never answers hold that
/// station's share of the slots, and the rest wait behind it alone: an
/// audited read of another station goes at once and is recorded. A stop
/// ends them all.
#[tokio::test(start_paused = true)]
async fn a_burst_to_a_silent_station_leaves_the_slots_to_others() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let burst: Vec<_> = (0..SLOTS)
        .map(|_| endpoint.read(Station::local(&mac(SILENT))))
        .collect();
    for _ in 0..LANE_SHARE {
        assert_eq!(endpoint.request().await.0, local(SILENT));
    }
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(endpoint.sent.try_recv().is_err(), "the rest wait");
    assert_eq!(source.available_operations(), SLOTS - LANE_SHARE);
    let asked = Instant::now();
    audited_read(&mut endpoint, direct_read(), SINK).await;
    assert_eq!(asked.elapsed(), Duration::ZERO, "admitted at once");
    drop(source);
    endpoint.stop().await;
    for read in burst {
        assert!(bounded(read).await.unwrap().is_err());
    }
}

/// The stations of another network share its lane, since the router's
/// path takes one request at a time: reads of two of them hold no more
/// than a share between them, and a read of a station on this link still
/// goes at once.
#[tokio::test(start_paused = true)]
async fn the_stations_of_another_network_share_its_lane() {
    let mut endpoint = source(SessionRole::ClientOnly, false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let reads: Vec<_> = (0..LANE_SHARE + 2)
        .map(|n| {
            let station = 30 + u8::try_from(n % 2).unwrap();
            endpoint.read(Station::routed(&mac(ROUTER), REMOTE_NETWORK, &mac(station)))
        })
        .collect();
    let (route, _) = endpoint.request().await;
    assert_eq!(route.link, mac(ROUTER));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(endpoint.sent.try_recv().is_err(), "one at a time");
    assert_eq!(source.available_operations(), SLOTS - LANE_SHARE);
    let asked = Instant::now();
    audited_read(&mut endpoint, direct_read(), SINK).await;
    assert_eq!(asked.elapsed(), Duration::ZERO, "admitted at once");
    drop(source);
    endpoint.stop().await;
    for read in reads {
        assert!(bounded(read).await.unwrap().is_err());
    }
}

/// Two reads of one station that both waited for its lane, long past the
/// minimum request interval, go that interval apart once admitted, not
/// together: each takes its turn with its slot, and the one that finds the
/// turn taken gives its slot back while it waits for the next.
#[tokio::test(start_paused = true)]
async fn requests_admitted_together_go_the_interval_apart() {
    let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
    let mut endpoint = unstarted_source(SessionRole::ClientOnly, recipient, None);
    endpoint.session.client_config.min_request_interval_ms = 1_000;
    endpoint.session.client_config.apdu_timeout_ms = 60_000;
    endpoint.start(false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let reads: Vec<_> = (0..LANE_SHARE + 2)
        .map(|_| endpoint.read(direct_read()))
        .collect();
    let mut open = Vec::new();
    for _ in 0..LANE_SHARE {
        open.push(endpoint.request().await.1);
    }
    // The other two wait for the lane, past the turns they would have had.
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(endpoint.sent.try_recv().is_err(), "they wait for the lane");
    for request in &open {
        endpoint
            .deliver_apdu(ack(request, 5), mac(PEER), None)
            .await;
    }
    let mut late = requests_among(&mut endpoint, LANE_SHARE + 1).await;
    crate::session::eventually::eventually(
        Duration::from_millis(100),
        "only the one sent holds a slot",
        async || source.available_operations() == SLOTS - 1,
    )
    .await;
    late.extend(requests_among(&mut endpoint, 1).await);
    let [(first, one), (second, two)]: [_; 2] = late.try_into().expect("two requests");
    assert!(
        second - first >= Duration::from_secs(1),
        "admitted together, they went {:?} apart",
        second - first
    );
    for request in [&one, &two] {
        endpoint
            .deliver_apdu(ack(request, 5), mac(PEER), None)
            .await;
    }
    assert!(requests_among(&mut endpoint, 2).await.is_empty());
    for read in reads {
        assert!(bounded(read).await.unwrap().is_ok());
    }
    drop(source);
    endpoint.stop().await;
}

/// A request waiting for its turn at its destination holds no slot: of
/// reads queued for one station of another network, only the one sent
/// holds one of the network's, and a read of another station there goes
/// as soon as the router's path is free, while the others wait out the
/// interval.
#[tokio::test(start_paused = true)]
async fn a_request_waiting_for_its_turn_keeps_no_one_out_of_its_lane() {
    let recipient = BACnetRecipient::Device(oid(ObjectType::DEVICE, 999));
    let mut endpoint = unstarted_source(SessionRole::ClientOnly, recipient, None);
    endpoint.session.client_config.min_request_interval_ms = 1_000;
    endpoint.session.client_config.apdu_timeout_ms = 60_000;
    endpoint.start(false).await;
    let source = Arc::clone(endpoint.session.source_audit.as_ref().unwrap());
    let at = |last| Station::routed(&mac(ROUTER), REMOTE_NETWORK, &mac(last));
    let queued: Vec<_> = (0..LANE_SHARE).map(|_| endpoint.read(at(30))).collect();
    let (_, first) = endpoint.request().await;
    // Its caller waits for the router's path, which the first holds.
    let other = endpoint.read(at(31));
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert_eq!(source.available_operations(), SLOTS - 1, "the first only");
    let from = NpduAddress {
        network: REMOTE_NETWORK,
        mac_address: mac(30),
    };
    let answered = Instant::now();
    endpoint
        .deliver_apdu(ack(&first, 5), mac(ROUTER), Some(from))
        .await;
    let [(went, _)]: [_; 1] = (requests_among(&mut endpoint, 2).await)
        .try_into()
        .expect("the other station's read");
    assert_eq!(went, answered, "it went once the path was free");
    drop((source, queued, other));
    endpoint.stop().await;
}
