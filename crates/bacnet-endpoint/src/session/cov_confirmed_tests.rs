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

/// The table holds at most `max_subscriptions_global` and refuses the next
/// with RESOURCES / NO_SPACE_TO_ADD_LIST_ELEMENT; an outbox of one entry
/// sends a change to two subscribers one after the other, the second with
/// the current value once room frees. An invalid policy fails startup.
#[tokio::test(start_paused = true)]
async fn the_bounds_hold() {
    let policy = CovPolicy {
        max_subscriptions_global: 2,
        ..CovPolicy::default()
    };
    let one = OutboxLimits {
        entries: 1,
        octets: 256,
    };
    let config = ReportConfig::new(policy).with_outbox(one);
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(config)).await;
    let to = mac_of(&session);
    for process in [1, 2] {
        let body = subscribe(process, Some(false), Some(300));
        node.request(&to, process as u8, SUBSCRIBE_COV, &body).await;
        assert_eq!(node.quiet(&to).await.len(), 2);
    }
    node.request(&to, 3, SUBSCRIBE_COV, &subscribe(3, Some(false), Some(300)))
        .await;
    // Error: RESOURCES (3), NO_SPACE_TO_ADD_LIST_ELEMENT (19).
    assert_eq!(
        node.quiet(&to).await,
        [Bytes::from_static(&[0x50, 3, 5, 0x91, 3, 0x91, 19])]
    );
    session
        .set_present_value_local(&ai1(), PropertyValue::Real(26.0))
        .await
        .unwrap();
    let mut heard: Vec<_> = node
        .quiet(&to)
        .await
        .iter()
        .map(|apdu| notified(apdu))
        .collect();
    heard.sort_by_key(|(process, ..)| *process);
    let value = PropertyValue::Real(26.0);
    assert_eq!(heard, [(1, 300, value.clone()), (2, 300, value)]);
    let counters = session.cov.host().unwrap().lock().reporter.counters();
    assert_eq!(counters.owed, 1, "one report waited for room");
    session.stop().await.unwrap();

    for (role, cov) in [
        (
            SessionRole::ServerOnly,
            CovPolicy {
                max_subscriptions_global: 0,
                ..CovPolicy::default()
            },
        ),
        (SessionRole::ClientOnly, CovPolicy::default()),
    ] {
        let mut refused = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(role)
            .database(fixture())
            .build_session_over(BusPort::new())
            .unwrap()
            .with_cov(ReportConfig::new(cov));
        assert!(refused.start().await.is_err(), "{role:?}");
        assert!(!refused.is_running());
    }
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
