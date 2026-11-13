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
pub(super) fn confirmed_notification(apdu: &[u8]) -> (u8, Bytes) {
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(Bytes::copy_from_slice(apdu)) else {
        panic!("a confirmed request: {apdu:02X?}");
    };
    assert_eq!(request.service_choice, NOTIFY);
    (request.invoke_id, request.service_request)
}

/// A SimpleACK of ConfirmedCOVNotification under `invoke_id`.
pub(super) fn ack(invoke_id: u8) -> [u8; 3] {
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
/// ones with the current value once room frees. An invalid policy, an
/// outbox that is empty or past its bounds, a zero or over-long confirmed
/// timeout and a client-only session fail startup; at each bound it starts.
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
    let timed = |millis| {
        let timeout = bacnet_time::Duration::from_millis(millis);
        ReportConfig::default().with_confirmed(FlightTiming::new(timeout, 3))
    };
    use crate::session::{
        COV_CONFIRMED_TIMEOUT_MAX_MS, COV_OUTBOX_MAX_ENTRIES, COV_OUTBOX_MAX_OCTETS,
    };
    use bacnet_device_core::cov::FlightTiming;
    let session = |role, config: &ReportConfig| {
        BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(role)
            .database(fixture())
            .build_session_over(BusPort::new())
            .unwrap()
            .with_cov(config.clone())
    };
    for (role, config, says) in [
        (
            SessionRole::ServerOnly,
            ReportConfig::new(invalid),
            "max_subscriptions_global",
        ),
        (
            SessionRole::ServerOnly,
            empty(0, 2048),
            "at least one entry",
        ),
        (SessionRole::ServerOnly, empty(16, 0), "at least one entry"),
        (
            SessionRole::ServerOnly,
            empty(COV_OUTBOX_MAX_ENTRIES + 1, 2048),
            "at most",
        ),
        (
            SessionRole::ServerOnly,
            empty(16, COV_OUTBOX_MAX_OCTETS + 1),
            "at most",
        ),
        (SessionRole::ServerOnly, timed(0), "confirmed timeout"),
        (
            SessionRole::ServerOnly,
            timed(COV_CONFIRMED_TIMEOUT_MAX_MS + 1),
            "confirmed timeout",
        ),
        (
            SessionRole::ClientOnly,
            ReportConfig::default(),
            "server role",
        ),
    ] {
        let mut refused = session(role, &config);
        let error = refused.start().await.expect_err("refused at start");
        assert!(
            error.to_string().contains(says),
            "{role:?} {config:?}: {error}"
        );
        assert!(!refused.is_running());
    }
    // At each bound the session starts.
    let widest = ReportConfig::default()
        .with_outbox(OutboxLimits {
            entries: COV_OUTBOX_MAX_ENTRIES,
            octets: COV_OUTBOX_MAX_OCTETS,
        })
        .with_confirmed(FlightTiming::new(
            bacnet_time::Duration::from_millis(COV_CONFIRMED_TIMEOUT_MAX_MS),
            3,
        ));
    let mut widest = session(SessionRole::ServerOnly, &widest);
    widest.start().await.unwrap();
    widest.stop().await.unwrap();
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

/// With every invoke ID leased elsewhere, a confirmed report waits without
/// looking again or spinning, and goes as soon as an ID comes back.
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
    // Paused time moves, so the task waits rather than spins: no pass
    // while it does.
    let host = Arc::clone(session.cov.host().unwrap());
    let passes = || host.passes.load(std::sync::atomic::Ordering::Relaxed);
    let before = passes();
    assert!(
        node.next_within(Duration::from_millis(1_025))
            .await
            .is_none()
    );
    assert_eq!(passes(), before, "asleep while the pool is full");
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

/// Two changes back to back to a confirmed subscriber, on the server and
/// on the endpoint alike: the first goes, the second waits for its Ack,
/// and the look the Ack brings sends the current value (#2508).
#[tokio::test(start_paused = true)]
async fn back_to_back_confirmed_changes_follow_the_ack_as_on_the_server() {
    let mut node = Node::new().await;
    let mut server = server().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let body = subscribe(7, Some(true), Some(300));
    let mut heard = Vec::new();
    for to in [MacAddr::from_slice(server.local_mac()), mac_of(&session)] {
        node.request(&to, 1, SUBSCRIBE_COV, &body).await;
        let sent = node.quiet(&to).await;
        node.send(&to, &ack(confirmed_notification(&sent[1]).0))
            .await;
        assert!(node.quiet(&to).await.is_empty());
        for value in [25.0, 20.0] {
            let value = PropertyValue::Real(value);
            if to.as_slice() == server.local_mac() {
                server.set_present_value_local(&ai1(), value).await.unwrap();
            } else {
                session
                    .set_present_value_local(&ai1(), value)
                    .await
                    .unwrap();
            }
        }
        let mut params = Vec::new();
        for _ in 0..2 {
            let sent = node.quiet(&to).await;
            let [notification] = &sent[..] else {
                panic!("one notification at a time: {sent:?}");
            };
            let (id, notification) = confirmed_notification(notification);
            params.push(notification);
            node.send(&to, &ack(id)).await;
        }
        assert!(node.quiet(&to).await.is_empty());
        heard.push(params);
    }
    assert_eq!(heard[1], heard[0], "endpoint, then server");
    session.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// At the bound, with an outbox of one entry, an unconfirmed report waits
/// for room alone (#2512): LSP-1's two unconfirmed subscribers both hear
/// its change with no time passing, while 65 silent confirmed
/// subscriptions hold every ID the bound allows and one more waits.
#[tokio::test(start_paused = true)]
async fn at_the_bound_an_unconfirmed_report_goes_at_once() {
    use crate::session::COV_CONFIRMED_IN_FLIGHT;
    use bacnet_device_core::cov::FlightTiming;
    use bacnet_device_core::write::local::LocalCommandSource;
    let policy = CovPolicy {
        max_subscriptions_per_peer: 256,
        max_confirmed_in_flight_per_peer: 256,
        ..CovPolicy::default()
    };
    let minute = FlightTiming::new(bacnet_time::Duration::from_secs(60), 0);
    let one = OutboxLimits {
        entries: 1,
        octets: 256,
    };
    let config = ReportConfig::new(policy)
        .with_confirmed(minute)
        .with_outbox(one);
    let (mut silent, mut node) = (Node::new().await, Node::new().await);
    let mut session = endpoint(SessionRole::ServerOnly, Some(config)).await;
    let to = mac_of(&session);
    for process in 0..=COV_CONFIRMED_IN_FLIGHT as u32 {
        let invoke_id = process as u8;
        let body = subscribe(process, Some(true), Some(300));
        silent.request(&to, invoke_id, SUBSCRIBE_COV, &body).await;
        while let Some((_, apdu)) = silent.next_within(Duration::from_millis(100)).await {
            if apdu[0] >> 4 != 0 {
                assert_eq!(apdu, acked(invoke_id));
                break;
            }
        }
    }
    silent.quiet(&to).await;
    assert_eq!(session.active_leases(), COV_CONFIRMED_IN_FLIGHT);
    for process in [1, 2] {
        let body = subscribe_to(lsp1(), process, Some(false), Some(300));
        node.request(&to, process as u8, SUBSCRIBE_COV, &body).await;
        assert_eq!(node.quiet(&to).await.len(), 2);
    }
    let at = tokio::time::Instant::now();
    let source = LocalCommandSource::ServerDevice;
    let oos = PropertyIdentifier::OUT_OF_SERVICE;
    let value = PropertyValue::Boolean(true);
    session
        .write_local(&lsp1(), oos, None, value, None, source)
        .await
        .unwrap();
    for subscriber in 1..=2 {
        let heard = node.next_within(Duration::from_secs(1)).await;
        assert!(heard.is_some(), "subscriber {subscriber} hears it");
    }
    assert_eq!(tokio::time::Instant::now(), at, "with no time passing");
    session.stop().await.unwrap();
}

/// An ID that comes back while the task is still sending, after a report
/// found the pool empty and before the task waits again, isn't lost: the
/// coordinator's wake-up reaches only a waiter already registered, so the
/// task registers, then looks at the pool. Here every ID is leased, AI-1's
/// confirmed report waits for one, and the subscriber to LSP-1 gives one
/// back on hearing the first of five notifications the same pass sends.
#[tokio::test(start_paused = true)]
async fn an_id_back_while_the_task_sends_is_not_lost() {
    use bacnet_device_core::write::local::LocalCommandSource;
    use bacnet_endpoint_core::coordinator::TerminalPolicy;
    let (mut watcher, mut releaser) = (Node::new().await, Node::new().await);
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let coordinator = session.coordinator();
    let lease = || {
        let peer = CanonicalPeer::direct(&[9]);
        LeaseMetadata::requester(peer, READ_PROPERTY, TerminalPolicy::ComplexAck)
    };
    let mut held: Vec<_> = (0..256)
        .map(|_| coordinator.reserve(lease()).unwrap())
        .collect();
    let body = subscribe(1, Some(true), Some(300));
    watcher.request(&to, 1, SUBSCRIBE_COV, &body).await;
    assert_eq!(watcher.quiet(&to).await, [acked(1)], "no ID for its report");
    for process in 1..=5u8 {
        let body = subscribe_to(lsp1(), process.into(), Some(false), Some(300));
        releaser.request(&to, process, SUBSCRIBE_COV, &body).await;
        assert_eq!(releaser.quiet(&to).await.len(), 2);
    }
    let given = held.pop().unwrap();
    let back = Arc::clone(&coordinator);
    let subscriber = tokio::spawn(async move {
        let first = releaser.next_within(Duration::from_secs(1)).await;
        back.release(given).unwrap();
        first.is_some()
    });
    // One change to each: AI-1's report is refused an ID again, then
    // LSP-1's five go, and the ID comes back while they do.
    let real = PropertyValue::Real(30.0);
    session.set_present_value_local(&ai1(), real).await.unwrap();
    let (oos, source) = (
        PropertyIdentifier::OUT_OF_SERVICE,
        LocalCommandSource::ServerDevice,
    );
    let value = PropertyValue::Boolean(true);
    session
        .write_local(&lsp1(), oos, None, value, None, source)
        .await
        .unwrap();
    assert!(subscriber.await.unwrap(), "LSP-1's subscriber heard it");
    let (_, apdu) = watcher
        .next_within(Duration::from_secs(250))
        .await
        .expect("AI-1's report took the ID that came back");
    confirmed_notification(&apdu);
    for token in held {
        coordinator.release(token).unwrap();
    }
    session.stop().await.unwrap();
}

/// Three confirmed reports owed an ID at a full pool, and IDs coming back
/// one at a time, at 0, 0.5 and 1 s: each report goes as its ID comes back,
/// not once the subscriber before it, which never answers, has used up its
/// retries. One report getting an ID doesn't end the wait for the others.
#[tokio::test(start_paused = true)]
async fn each_owed_report_goes_as_an_id_comes_back() {
    use bacnet_endpoint_core::coordinator::TerminalPolicy;
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let coordinator = session.coordinator();
    let lease = || {
        let peer = CanonicalPeer::direct(&[9]);
        LeaseMetadata::requester(peer, READ_PROPERTY, TerminalPolicy::ComplexAck)
    };
    let mut held: Vec<_> = (0..256)
        .map(|_| coordinator.reserve(lease()).unwrap())
        .collect();
    for process in 1..=3u8 {
        let body = subscribe(process.into(), Some(true), Some(300));
        node.request(&to, process, SUBSCRIBE_COV, &body).await;
        assert_eq!(node.quiet(&to).await, [acked(process)], "owed an ID");
    }
    let mut heard = Vec::new();
    let start = tokio::time::Instant::now();
    for at in [0, 500, 1_000] {
        tokio::time::sleep_until(start + Duration::from_millis(at)).await;
        let back = tokio::time::Instant::now();
        coordinator.release(held.pop().unwrap()).unwrap();
        let (_, apdu) = node
            .next_within(Duration::from_millis(10))
            .await
            .unwrap_or_else(|| panic!("a report as the ID at {at} ms came back"));
        assert_eq!(tokio::time::Instant::now(), back, "with no time passing");
        let (_, body) = confirmed_notification(&apdu);
        let request = bacnet_services::cov::COVNotificationRequest::decode(&body).unwrap();
        heard.push(request.subscriber_process_identifier);
    }
    assert_eq!(heard, [1, 2, 3]);
    for token in held {
        coordinator.release(token).unwrap();
    }
    session.stop().await.unwrap();
}

/// Only an empty pool is waited for: it fills again as IDs come back. A
/// coordinator whose generations ran out, or whose state is poisoned,
/// never leases again, so the task stops watching after either.
#[test]
fn only_an_empty_pool_is_waited_for() {
    use bacnet_endpoint_core::coordinator::ReserveError;
    let leases = || Leases {
        coordinator: Arc::new(OutboundTransactionCoordinator::new()),
        out: Vec::new(),
        spent: false,
    };
    let mut empty = leases();
    empty.refused(ReserveError::Exhausted);
    assert!(empty.could_take(), "an empty pool fills again");
    for error in [
        ReserveError::GenerationExhausted,
        ReserveError::StatePoisoned,
    ] {
        let mut spent = leases();
        spent.refused(error);
        assert!(!spent.could_take(), "{error:?} lasts");
    }
}

/// A report owed an ID by a coordinator that can no longer lease waits
/// without watching the pool: IDs coming back wake no pass and send
/// nothing.
#[tokio::test(start_paused = true)]
async fn a_spent_coordinator_is_not_watched() {
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
    let host = Arc::clone(session.cov.host().unwrap());
    // As a refusal that lasts would leave it.
    host.lock()
        .leases
        .refused(bacnet_endpoint_core::coordinator::ReserveError::GenerationExhausted);
    let passes = || host.passes.load(std::sync::atomic::Ordering::Relaxed);
    let before = passes();
    for token in held {
        coordinator.release(token).unwrap();
    }
    assert!(node.next_within(Duration::from_secs(1)).await.is_none());
    assert_eq!(passes(), before, "no pass for IDs coming back");
    session.stop().await.unwrap();
}

/// IDs that come back count for the `Reporter` only up to the session's
/// bound: with 63 notifications out and the rest of the pool leased
/// elsewhere, three reports wait for an ID, and ten coming back at once let
/// one go, into the bound's last place. The other two aren't looked at only
/// to be refused at the bound and owed again.
#[tokio::test(start_paused = true)]
async fn ids_back_past_the_bound_count_only_to_it() {
    use crate::session::COV_CONFIRMED_IN_FLIGHT;
    use bacnet_device_core::cov::FlightTiming;
    use bacnet_endpoint_core::coordinator::TerminalPolicy;
    // One subscriber with room for all of them, and a minute's timeout, so
    // nothing ends on its own.
    let policy = CovPolicy {
        max_subscriptions_per_peer: 256,
        max_confirmed_in_flight_per_peer: 256,
        ..CovPolicy::default()
    };
    let minute = FlightTiming::new(bacnet_time::Duration::from_secs(60), 0);
    let config = ReportConfig::new(policy).with_confirmed(minute);
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(config)).await;
    let to = mac_of(&session);
    let out = COV_CONFIRMED_IN_FLIGHT as u8 - 1;
    for process in 0..out {
        let body = subscribe(process.into(), Some(true), Some(300));
        node.request(&to, process, SUBSCRIBE_COV, &body).await;
        assert_eq!(node.quiet(&to).await.len(), 2, "its Ack and report");
    }
    let coordinator = session.coordinator();
    let lease = || {
        let peer = CanonicalPeer::direct(&[9]);
        LeaseMetadata::requester(peer, READ_PROPERTY, TerminalPolicy::ComplexAck)
    };
    let mut held = Vec::new();
    while let Ok(token) = coordinator.reserve(lease()) {
        held.push(token);
    }
    assert_eq!(held.len(), 256 - usize::from(out));
    for process in out..out + 3 {
        let body = subscribe(process.into(), Some(true), Some(300));
        node.request(&to, process, SUBSCRIBE_COV, &body).await;
        assert_eq!(node.quiet(&to).await, [acked(process)], "owed an ID");
    }
    let host = Arc::clone(session.cov.host().unwrap());
    let owed = || host.lock().reporter.counters().owed;
    let before = owed();
    for token in held.drain(..10) {
        coordinator.release(token).unwrap();
    }
    let sent = node.quiet(&to).await;
    assert_eq!(sent.len(), 1, "the bound's last place");
    confirmed_notification(&sent[0]);
    assert_eq!(owed(), before, "none looked at only to be owed again");
    assert!(host.lock().reporter.owes_invoke_id(), "two still wait");
    for token in held {
        coordinator.release(token).unwrap();
    }
    session.stop().await.unwrap();
}
