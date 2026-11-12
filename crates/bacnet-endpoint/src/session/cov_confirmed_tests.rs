//! Confirmed COV notifications on an endpoint session (#2490): their invoke
//! IDs come from the pool the client role draws from, an Ack ends one and
//! lets a change held behind it go, a subscriber that never answers is asked
//! again and then held off, the bounds hold, and a stop with reports
//! outstanding gives every invoke ID back and sends nothing more.

use super::*;
use crate::session::eventually::eventually;
use bacnet_device_core::cov::CovPolicy;
use bacnet_device_core::report::OutboxLimits;

const NOTIFY: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;

/// A ConfirmedCOVNotification's invoke ID and service parameters.
fn confirmed_notification(apdu: &[u8]) -> (u8, Bytes) {
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(Bytes::copy_from_slice(apdu)) else {
        panic!("a confirmed request: {apdu:02X?}");
    };
    assert_eq!(request.service_choice, NOTIFY);
    (request.invoke_id, request.service_request)
}

/// A SimpleACK of ConfirmedCOVNotification under `invoke_id`.
fn ack(invoke_id: u8) -> [u8; 3] {
    [0x20, invoke_id, NOTIFY.to_raw()]
}

/// The client role's read of a node that never answers holds one invoke ID
/// while a confirmed notification takes another from the same pool; each
/// carries the parameters the server's would. The Ack gives the ID back
/// and sends the change held behind the report; the next report takes the
/// next ID in turn.
#[tokio::test(start_paused = true)]
async fn confirmed_notifications_share_the_client_roles_invoke_ids() {
    let mut node = Node::new().await;
    let mut server = server().await;
    let mut session = endpoint(SessionRole::Both, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let client = session.cloned_client_handle().unwrap();
    let silent = MacAddr::from_slice(node.mac());
    let read = tokio::spawn(async move {
        client
            .read_property(&silent, ai1(), PropertyIdentifier::PRESENT_VALUE, None)
            .await
    });
    let (from, apdu) = node.next_within(Duration::from_millis(100)).await.unwrap();
    assert_eq!(from, to);
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(apdu) else {
        panic!("the client role's read")
    };
    assert_eq!(request.service_choice, READ_PROPERTY);
    let client_id = request.invoke_id;
    assert_eq!(session.active_leases(), 1);

    let body = subscribe(7, Some(true), Some(300));
    let server_mac = MacAddr::from_slice(server.local_mac());
    node.request(&server_mac, 1, SUBSCRIBE_COV, &body).await;
    let from_server = node.quiet(&server_mac).await;
    let (server_id, server_params) = confirmed_notification(&from_server[1]);
    node.send(&server_mac, &ack(server_id)).await;
    assert!(node.quiet(&server_mac).await.is_empty());

    node.request(&to, 1, SUBSCRIBE_COV, &body).await;
    let sent = node.quiet(&to).await;
    assert_eq!(sent[0], acked(1));
    let (first, params) = confirmed_notification(&sent[1]);
    assert_eq!(params, server_params, "the server's parameters");
    assert_ne!(first, client_id, "one pool, two invoke IDs");
    assert_eq!(session.active_leases(), 2);

    // A change while the report is out waits for its answer.
    session
        .set_present_value_local(&ai1(), PropertyValue::Real(24.0))
        .await
        .unwrap();
    assert!(node.quiet(&to).await.is_empty(), "held behind the report");
    node.send(&to, &ack(first)).await;
    let sent = node.quiet(&to).await;
    let (second, _) = confirmed_notification(&sent[0]);
    assert_eq!(second, first.wrapping_add(1), "the next ID in turn");
    assert_eq!(session.active_leases(), 2, "the read's and the change's");
    node.send(&to, &ack(second)).await;
    eventually(
        Duration::from_secs(1),
        "the Ack gives its ID back",
        async || session.active_leases() == 1,
    )
    .await;
    assert!(read.await.unwrap().is_err(), "the read timed out");
    assert_eq!(session.active_leases(), 0);
    session.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Unanswered, a confirmed notification goes again under its invoke ID at
/// each timeout of the default timing (3 s, three retries), then fails and
/// gives the ID back; a change in the hold-off that follows, one retry
/// cycle, is not reported, and one after it is.
#[tokio::test(start_paused = true)]
async fn a_subscriber_that_never_answers_is_asked_again_then_held_off() {
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let start = tokio::time::Instant::now();
    node.request(&to, 1, SUBSCRIBE_COV, &subscribe(7, Some(true), Some(300)))
        .await;
    let sent = node.quiet(&to).await;
    let first = confirmed_notification(&sent[1]);
    for attempt in 1..=3 {
        let (_, apdu) = node.next_within(Duration::from_secs(4)).await.unwrap();
        assert_eq!(confirmed_notification(&apdu), first, "attempt {attempt}");
        let at = start.elapsed();
        let due = Duration::from_secs(3 * attempt);
        assert!(at >= due && at < due + Duration::from_millis(10), "{at:?}");
    }
    eventually(
        Duration::from_secs(4),
        "the failure gives the ID back",
        async || session.active_leases() == 0,
    )
    .await;
    let failed = start.elapsed();
    assert!(failed >= Duration::from_secs(12), "{failed:?}");

    session
        .set_present_value_local(&ai1(), PropertyValue::Real(30.0))
        .await
        .unwrap();
    assert!(node.next_within(Duration::from_secs(10)).await.is_none());
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(start.elapsed() > failed + Duration::from_secs(12));
    session
        .set_present_value_local(&ai1(), PropertyValue::Real(31.0))
        .await
        .unwrap();
    let (_, apdu) = node.next_within(Duration::from_millis(100)).await.unwrap();
    let (next, _) = confirmed_notification(&apdu);
    assert_ne!(next, first.0);
    session.stop().await.unwrap();
}

/// The table holds at most `max_subscriptions_per_peer` from one
/// subscriber and `max_subscriptions_global` in all, and refuses the next
/// with RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT; an outbox of one entry
/// sends a change to three subscriptions one after the other, the later
/// ones with the current value once room frees. An invalid policy, an empty
/// outbox and a client-only session fail startup.
#[tokio::test(start_paused = true)]
async fn the_bounds_hold() {
    let policy = CovPolicy {
        max_subscriptions_global: 3,
        max_subscriptions_per_peer: 2,
        ..CovPolicy::default()
    };
    let one = OutboxLimits {
        entries: 1,
        octets: 256,
    };
    let config = ReportConfig::new(policy).with_outbox(one);
    let (mut a, mut b) = (Node::new().await, Node::new().await);
    let mut session = endpoint(SessionRole::ServerOnly, Some(config)).await;
    let to = mac_of(&session);
    // Error: RESOURCES (3), NO_SPACE_TO_ADD_LIST_ELEMENT (19).
    let refused = |invoke_id| [Bytes::from(vec![0x50, invoke_id, 5, 0x91, 3, 0x91, 19])];
    for process in [1, 2] {
        let body = subscribe(process, Some(false), Some(300));
        a.request(&to, process as u8, SUBSCRIBE_COV, &body).await;
        assert_eq!(a.quiet(&to).await.len(), 2);
    }
    let third = subscribe(3, Some(false), Some(300));
    a.request(&to, 3, SUBSCRIBE_COV, &third).await;
    assert_eq!(
        a.quiet(&to).await,
        refused(3),
        "past the per-subscriber cap"
    );
    b.request(&to, 1, SUBSCRIBE_COV, &subscribe(1, Some(false), Some(300)))
        .await;
    assert_eq!(b.quiet(&to).await.len(), 2, "another subscriber's");
    b.request(&to, 2, SUBSCRIBE_COV, &subscribe(2, Some(false), Some(300)))
        .await;
    assert_eq!(b.quiet(&to).await, refused(2), "past the global cap");

    session
        .set_present_value_local(&ai1(), PropertyValue::Real(26.0))
        .await
        .unwrap();
    let value = PropertyValue::Real(26.0);
    let mut heard: Vec<_> = a
        .quiet(&to)
        .await
        .iter()
        .map(|apdu| notified(apdu))
        .collect();
    heard.sort_by_key(|(process, ..)| *process);
    assert_eq!(heard, [(1, 300, value.clone()), (2, 300, value.clone())]);
    let heard: Vec<_> = b
        .quiet(&to)
        .await
        .iter()
        .map(|apdu| notified(apdu))
        .collect();
    assert_eq!(heard, [(1, 300, value)]);
    let counters = session.cov.host().unwrap().lock().reporter.counters();
    assert_eq!(counters.owed, 2, "two reports waited for room");
    session.stop().await.unwrap();

    let invalid = CovPolicy {
        max_subscriptions_global: 0,
        ..CovPolicy::default()
    };
    let empty =
        |entries, octets| ReportConfig::default().with_outbox(OutboxLimits { entries, octets });
    for (role, config) in [
        (SessionRole::ServerOnly, ReportConfig::new(invalid)),
        (SessionRole::ServerOnly, empty(0, 2048)),
        (SessionRole::ServerOnly, empty(16, 0)),
        (SessionRole::ClientOnly, ReportConfig::default()),
    ] {
        let mut refused = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(role)
            .database(fixture())
            .build_session_over(BusPort::new())
            .unwrap()
            .with_cov(config.clone());
        assert!(refused.start().await.is_err(), "{role:?} {config:?}");
        assert!(!refused.is_running());
    }
}

/// However many subscribers stop answering, their confirmed notifications
/// hold at most `COV_CONFIRMED_IN_FLIGHT` invoke IDs: the client role still
/// gets one and its reads complete. At the bound the task sleeps: neither
/// the client role's transactions ending nor time passing wakes it to look
/// again at the reports it holds back. One Ack lets a held report go at
/// once, with no time passing.
#[tokio::test(start_paused = true)]
async fn silent_subscribers_leave_the_client_role_its_invoke_ids() {
    use crate::session::COV_CONFIRMED_IN_FLIGHT;
    use bacnet_device_core::cov::FlightTiming;
    // One subscriber with room for 256 subscriptions, all in flight at
    // once, and a minute's timeout, so nothing ends on its own.
    let policy = CovPolicy {
        max_subscriptions_per_peer: 256,
        max_confirmed_in_flight_per_peer: 256,
        ..CovPolicy::default()
    };
    let minute = FlightTiming::new(bacnet_time::Duration::from_secs(60), 0);
    let config = ReportConfig::new(policy).with_confirmed(minute);
    let mut node = Node::new().await;
    let mut server = server().await;
    let mut session = endpoint(SessionRole::Both, Some(config)).await;
    let to = mac_of(&session);
    let mut out = Vec::new();
    for process in 0..256u32 {
        let invoke_id = process as u8;
        let body = subscribe(process, Some(true), Some(300));
        node.request(&to, invoke_id, SUBSCRIBE_COV, &body).await;
        // Its reply, after any notification still on its way.
        loop {
            let (_, apdu) = node.next_within(Duration::from_millis(100)).await.unwrap();
            if apdu[0] >> 4 != 0 {
                assert_eq!(apdu, acked(invoke_id));
                break;
            }
            out.push(confirmed_notification(&apdu).0);
        }
    }
    while let Some((_, apdu)) = node.next_within(Duration::from_millis(100)).await {
        out.push(confirmed_notification(&apdu).0);
    }
    assert_eq!(out.len(), COV_CONFIRMED_IN_FLIGHT);
    assert_eq!(session.active_leases(), COV_CONFIRMED_IN_FLIGHT);

    let host = Arc::clone(session.cov.host().unwrap());
    let passes = || host.passes.load(std::sync::atomic::Ordering::Relaxed);
    let before = passes();
    let client = session.client().unwrap();
    for _ in 0..3 {
        let read = client.read_property(
            server.local_mac(),
            ai1(),
            PropertyIdentifier::PRESENT_VALUE,
            None,
        );
        let answer = read.await.expect("the client role gets an invoke ID");
        assert_eq!(answer.object_identifier, ai1());
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(passes(), before, "asleep at the bound");

    let at = tokio::time::Instant::now();
    node.send(&to, &ack(out[0])).await;
    let (_, apdu) = node.next_within(Duration::from_secs(1)).await.unwrap();
    assert_eq!(tokio::time::Instant::now(), at, "with no time passing");
    let (next, _) = confirmed_notification(&apdu);
    assert!(!out[1..].contains(&next), "an ID that came back");
    assert_eq!(session.active_leases(), COV_CONFIRMED_IN_FLIGHT);

    // A held notification's own timeout still wakes the task: the other 63
    // fail at their minute, and as many held reports take their IDs.
    tokio::time::sleep(Duration::from_millis(59_500)).await;
    let mut refilled = 0;
    while let Some((_, apdu)) = node.next_within(Duration::from_millis(100)).await {
        confirmed_notification(&apdu);
        refilled += 1;
    }
    assert_eq!(refilled, COV_CONFIRMED_IN_FLIGHT - 1);
    assert_eq!(session.active_leases(), COV_CONFIRMED_IN_FLIGHT);
    session.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Stopping with reports outstanding and changes held behind them gives
/// every invoke ID back and sends nothing more.
#[tokio::test(start_paused = true)]
async fn stop_gives_back_every_invoke_id_and_sends_nothing_more() {
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    for process in [1, 2] {
        let body = subscribe(process, Some(true), Some(300));
        node.request(&to, process as u8, SUBSCRIBE_COV, &body).await;
        assert_eq!(node.quiet(&to).await.len(), 2);
    }
    session
        .set_present_value_local(&ai1(), PropertyValue::Real(27.0))
        .await
        .unwrap();
    assert!(node.quiet(&to).await.is_empty(), "held behind the reports");
    assert_eq!(session.active_leases(), 2);
    session.stop().await.unwrap();
    assert_eq!(session.active_leases(), 0);
    assert!(node.next_within(Duration::from_secs(30)).await.is_none());
}

/// With every invoke ID leased elsewhere, a confirmed report waits, looking
/// again now and then without spinning, and goes as soon as an ID comes
/// back.
#[tokio::test(start_paused = true)]
async fn a_report_waits_for_an_invoke_id() {
    use bacnet_endpoint_core::coordinator::TerminalPolicy;
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let coordinator = session.coordinator();
    let lease = || {
        let peer = CanonicalPeer::direct(&[9]);
        LeaseMetadata::requester(peer, READ_PROPERTY, TerminalPolicy::ComplexAck)
    };
    let held: Vec<_> = (0..256)
        .map(|_| coordinator.reserve(lease()).unwrap())
        .collect();
    node.request(&to, 1, SUBSCRIBE_COV, &subscribe(7, Some(true), Some(300)))
        .await;
    assert_eq!(node.quiet(&to).await, [acked(1)], "no ID for the report");
    // Paused time moves, so the task waits rather than spins.
    assert!(
        node.next_within(Duration::from_millis(1_025))
            .await
            .is_none()
    );
    coordinator.release(held[0]).unwrap();
    let (_, apdu) = node.next_within(Duration::from_millis(10)).await.unwrap();
    let (id, _) = confirmed_notification(&apdu);
    assert_eq!(id, held[0].invoke_id(), "the ID that came back");
    node.send(&to, &ack(id)).await;
    for token in &held[1..] {
        coordinator.release(*token).unwrap();
    }
    eventually(Duration::from_secs(1), "every ID back", async || {
        session.active_leases() == 0
    })
    .await;
    session.stop().await.unwrap();
}
