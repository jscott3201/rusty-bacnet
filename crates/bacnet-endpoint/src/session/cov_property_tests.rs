//! SubscribeCOVProperty on an endpoint session (#2490), on paused time over
//! the in-memory link: the opt-in, `Reporter`, send path and bounds of
//! SubscribeCOV. A property subscription's replies and notifications match
//! `bacnet-server`'s octet for octet, from the first notification through a
//! change under and past its own COV increment, a local write, a renewal, a
//! cancellation and the lifetime running out. It lives beside an object
//! subscription to the same object, counts against the caps, the outbox
//! and the bound on confirmed notifications in flight, and a client's
//! write, which reaches only the Device, notifies nobody: the Device takes
//! no subscription. The B/IP builder's `cov` is the session's opt-in.

use super::confirmed::{ack, confirmed_notification};
use super::*;
use crate::session::bus_port::bus_port;
use crate::session::eventually::eventually;
use bacnet_device_core::cov::{CovPolicy, FlightTiming};
use bacnet_device_core::report::OutboxLimits;
use bacnet_device_core::write::local::LocalCommandSource;
use bacnet_encoding::{primitives, tags};
use bacnet_types::enums::{ErrorClass, ErrorCode, RejectReason};

const PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
const OOS: PropertyIdentifier = PropertyIdentifier::OUT_OF_SERVICE;

/// A SubscribeCOVProperty body, written field by field so a refused
/// pairing can be sent too: a cancellation without `confirmed` and
/// `lifetime`.
fn property_body(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
    process: u32,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
    increment: Option<f32>,
) -> Vec<u8> {
    let mut body = BytesMut::new();
    primitives::encode_ctx_unsigned(&mut body, 0, u64::from(process));
    primitives::encode_ctx_object_id(&mut body, 1, &object);
    if let Some(confirmed) = confirmed {
        primitives::encode_ctx_boolean(&mut body, 2, confirmed);
    }
    if let Some(lifetime) = lifetime {
        primitives::encode_ctx_unsigned(&mut body, 3, u64::from(lifetime));
    }
    tags::encode_opening_tag(&mut body, 4);
    primitives::encode_ctx_unsigned(&mut body, 0, u64::from(property.to_raw()));
    if let Some(index) = index {
        primitives::encode_ctx_unsigned(&mut body, 1, u64::from(index));
    }
    tags::encode_closing_tag(&mut body, 4);
    if let Some(increment) = increment {
        primitives::encode_ctx_real(&mut body, 5, increment);
    }
    body.to_vec()
}

/// An unconfirmed subscription to `property` of `object` for 300 s, with
/// its own COV increment when given.
pub(super) fn subscribe_property(
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    process: u32,
    increment: Option<f32>,
) -> Vec<u8> {
    property_body(
        object,
        property,
        None,
        process,
        Some(false),
        Some(300),
        increment,
    )
}

/// A SimpleACK of SubscribeCOVProperty under `invoke_id`.
fn property_acked(invoke_id: u8) -> Bytes {
    Bytes::from(vec![0x20, invoke_id, PROPERTY.to_raw()])
}

/// An UnconfirmedCOVNotification's process, Time_Remaining and first
/// value, with the property it is of.
fn reported(apdu: &[u8]) -> (u32, u32, PropertyIdentifier, PropertyValue) {
    let Ok(Apdu::UnconfirmedRequest(request)) = decode_apdu(Bytes::copy_from_slice(apdu)) else {
        panic!("an unconfirmed request: {apdu:02X?}");
    };
    assert_eq!(
        request.service_choice,
        bacnet_types::enums::UnconfirmedServiceChoice::UNCONFIRMED_COV_NOTIFICATION
    );
    let notification =
        bacnet_services::cov::COVNotificationRequest::decode(&request.service_request).unwrap();
    let first = &notification.list_of_values[0];
    let (value, _) =
        bacnet_encoding::primitives::decode_application_value(&first.value, 0).unwrap();
    (
        notification.subscriber_process_identifier,
        notification.time_remaining,
        first.property_identifier,
        value,
    )
}

/// The class and code of a SubscribeCOVProperty's Error.
fn refusal(apdu: &Bytes) -> (ErrorClass, ErrorCode) {
    let Ok(Apdu::Error(error)) = decode_apdu(apdu.clone()) else {
        panic!("an Error: {apdu:02X?}");
    };
    assert_eq!(error.service_choice, PROPERTY);
    (error.error_class, error.error_code)
}

/// The one APDU in `sent`.
fn alone(what: &str, sent: &[Bytes]) -> Bytes {
    let [apdu] = sent else {
        panic!("{what}: one APDU, got {sent:02X?}");
    };
    apdu.clone()
}

impl Pair {
    /// The application writes `value` to `property` of `object` on both
    /// hosts: the notifications that follow, which must match.
    pub(super) async fn write(
        &mut self,
        what: &str,
        object: ObjectIdentifier,
        property: PropertyIdentifier,
        value: PropertyValue,
    ) -> Vec<Bytes> {
        let source = LocalCommandSource::ServerDevice;
        let server = MacAddr::from_slice(self.server.local_mac());
        let endpoint = mac_of(&self.endpoint);
        self.server
            .write_local(&object, property, None, value.clone(), None, source)
            .await
            .unwrap();
        let from_server = self.node.quiet(&server).await;
        self.endpoint
            .write_local(&object, property, None, value, None, source)
            .await
            .unwrap();
        let from_endpoint = self.node.quiet(&endpoint).await;
        assert_eq!(from_endpoint, from_server, "{what}: endpoint, then server");
        from_endpoint
    }
}

/// A property subscription's reply, first notification and the Device's
/// list match the server's; a change under the subscription's own COV
/// increment, not AI-1's 0, reports nothing on either, and one past it the
/// same notification.
#[tokio::test(start_paused = true)]
async fn a_property_subscription_notifies_as_the_server_does() {
    let mut pair = Pair::start().await;
    let body = subscribe_property(ai1(), PV, 7, Some(1.0));
    let sent = pair.both("subscribe", PROPERTY, &body).await;
    assert_eq!(sent.len(), 2, "the reply, then the first notification");
    assert_eq!(sent[0], property_acked(pair.invoke_id));
    assert_eq!(reported(&sent[1]), (7, 300, PV, PropertyValue::Real(20.0)));
    let list = read_device(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS);
    pair.both("the list", READ_PROPERTY, &list).await;

    assert!(pair.change("under the increment", 20.5).await.is_empty());
    let sent = pair.change("past the increment", 21.5).await;
    let heard = reported(&alone("past the increment", &sent));
    assert_eq!(heard, (7, 300, PV, PropertyValue::Real(21.5)));
    pair.stop().await;
}

/// A local write to the monitored property reports it as on the server.
#[tokio::test(start_paused = true)]
async fn a_local_write_to_the_monitored_property_is_reported() {
    let mut pair = Pair::start().await;
    let body = subscribe_property(ai1(), OOS, 9, None);
    let sent = pair.both("subscribe Out_Of_Service", PROPERTY, &body).await;
    assert_eq!(
        reported(&sent[1]),
        (9, 300, OOS, PropertyValue::Boolean(false))
    );
    let value = PropertyValue::Boolean(true);
    let sent = pair.write("out of service", ai1(), OOS, value).await;
    let heard = reported(&alone("out of service", &sent));
    assert_eq!(heard, (9, 300, OOS, PropertyValue::Boolean(true)));
    pair.stop().await;
}

/// An object subscription and a property subscription to AI-1, under one
/// subscriber and process: each reports on its own increment, both are
/// listed, and cancelling the property one leaves the object one. Each
/// step as on the server.
#[tokio::test(start_paused = true)]
async fn an_object_and_a_property_subscription_live_side_by_side() {
    let mut pair = Pair::start().await;
    let list = read_device(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS);
    let object = subscribe(7, Some(false), Some(300));
    assert_eq!(
        pair.both("the object", SUBSCRIBE_COV, &object).await.len(),
        2
    );
    let property = subscribe_property(ai1(), PV, 7, Some(1.0));
    assert_eq!(pair.both("its value", PROPERTY, &property).await.len(), 2);
    pair.both("both listed", READ_PROPERTY, &list).await;

    // AI-1's own increment of 0 for the object, 1.0 for the property.
    assert_eq!(pair.change("a small change", 20.5).await.len(), 1);
    let sent = pair.change("a large change", 21.5).await;
    assert_eq!(sent.len(), 2, "one for each");
    let cancel = property_body(ai1(), PV, None, 7, None, None, None);
    let sent = pair.both("cancel the property", PROPERTY, &cancel).await;
    assert_eq!(sent, [property_acked(pair.invoke_id)]);
    pair.both("the object's alone", READ_PROPERTY, &list).await;
    assert_eq!(pair.change("after the cancel", 25.0).await.len(), 1);
    pair.stop().await;
}

/// A renewal restarts the lifetime and reports again; a cancellation ends
/// the subscription, and one that matches nothing succeeds; a lifetime
/// runs out on the session's clock. Each step as the server takes it.
#[tokio::test(start_paused = true)]
async fn a_property_subscription_renews_cancels_and_expires_as_on_the_server() {
    let mut pair = Pair::start().await;
    let list = read_device(PropertyIdentifier::ACTIVE_COV_SUBSCRIPTIONS);
    let lasting = |lifetime| property_body(ai1(), PV, None, 7, Some(false), Some(lifetime), None);
    pair.both("subscribe", PROPERTY, &lasting(30)).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    pair.both("the list after 10 s", READ_PROPERTY, &list).await;
    let sent = pair.both("renew", PROPERTY, &lasting(60)).await;
    assert_eq!(reported(&sent[1]), (7, 60, PV, PropertyValue::Real(20.0)));

    let cancel = |process| property_body(ai1(), PV, None, process, None, None, None);
    let sent = pair.both("cancel", PROPERTY, &cancel(7)).await;
    assert_eq!(sent, [property_acked(pair.invoke_id)]);
    pair.both("the list once cancelled", READ_PROPERTY, &list)
        .await;
    assert!(pair.change("cancelled", 21.0).await.is_empty());
    let sent = pair.both("cancel nothing", PROPERTY, &cancel(8)).await;
    assert_eq!(sent, [property_acked(pair.invoke_id)]);

    pair.both("subscribe again", PROPERTY, &lasting(30)).await;
    tokio::time::sleep(Duration::from_secs(29)).await;
    let sent = pair.change("a second before it lapses", 22.0).await;
    let heard = reported(&alone("a second before it lapses", &sent));
    assert_eq!(heard, (7, 1, PV, PropertyValue::Real(22.0)));
    tokio::time::sleep(Duration::from_secs(2)).await;
    let host = pair.endpoint.cov.host().unwrap();
    assert!(
        host.lock().reporter.table().is_empty(),
        "purged on the clock"
    );
    let lapsed = pair
        .both("the list once lapsed", READ_PROPERTY, &list)
        .await;
    assert_eq!(lapsed, empty_list(pair.invoke_id));
    assert!(pair.change("lapsed", 23.0).await.is_empty());
    pair.stop().await;
}

/// Each refusal, the Device's own Description among them, matches the
/// server's octet for octet and subscribes nothing.
#[tokio::test(start_paused = true)]
async fn property_refusals_match_the_server() {
    let mut pair = Pair::start().await;
    let refused = |object, property, index, lifetime| {
        property_body(
            object,
            property,
            index,
            1,
            Some(false),
            Some(lifetime),
            None,
        )
    };
    let gone = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap();
    let cases = [
        (
            "the Device, which reports no COV",
            refused(device(), PropertyIdentifier::DESCRIPTION, None, 60),
            (
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ),
        ),
        (
            "no object",
            refused(gone, PV, None, 60),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            "a property AI-1 lacks",
            refused(ai1(), PropertyIdentifier::PRIORITY_ARRAY, None, 60),
            (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
        ),
        (
            "a property its object doesn't report",
            refused(lsp1(), PropertyIdentifier::MODE, None, 60),
            (ErrorClass::PROPERTY, ErrorCode::NOT_COV_PROPERTY),
        ),
        (
            "an index on a scalar",
            refused(ai1(), PV, Some(1), 60),
            (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            "a zero lifetime",
            refused(ai1(), PV, None, 0),
            (ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE),
        ),
    ];
    for (what, body, expected) in cases {
        let sent = pair.both(what, PROPERTY, &body).await;
        assert_eq!(refusal(&alone(what, &sent)), expected, "{what}");
    }
    let what = "a lifetime without the confirmed flag";
    let body = property_body(ai1(), PV, None, 1, None, Some(60), None);
    let sent = pair.both(what, PROPERTY, &body).await;
    let Ok(Apdu::Reject(reject)) = decode_apdu(alone(what, &sent)) else {
        panic!("{what}: a Reject");
    };
    assert_eq!(reject.reject_reason, RejectReason::INCONSISTENT_PARAMETERS);
    let host = pair.endpoint.cov.host().unwrap();
    assert!(host.lock().reporter.table().is_empty());
    pair.stop().await;
}

/// A client's write reaches only the Device, which takes no subscription,
/// so a written Description sends nothing beyond its reply, a property
/// subscription to AI-1 notwithstanding.
#[tokio::test(start_paused = true)]
async fn a_client_write_notifies_nobody() {
    let mut node = Node::new().await;
    let mut session = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
        .role(SessionRole::ServerOnly)
        .database(fixture())
        .device_writes(Arc::new(|_| true))
        .cov(ReportConfig::default())
        .build_session_over(bus_port())
        .unwrap();
    session.start().await.unwrap();
    let to = mac_of(&session);
    node.request(&to, 1, PROPERTY, &subscribe_property(ai1(), PV, 7, None))
        .await;
    assert_eq!(node.quiet(&to).await.len(), 2);
    let description = PropertyIdentifier::DESCRIPTION;
    let body = subscribe_property(device(), description, 8, None);
    node.request(&to, 2, PROPERTY, &body).await;
    let reply = alone("the Device's Description", &node.quiet(&to).await);
    assert_eq!(
        refusal(&reply),
        (
            ErrorClass::OBJECT,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
        )
    );

    let mut body = BytesMut::new();
    let mut value = BytesMut::new();
    primitives::encode_app_character_string(&mut value, "written").unwrap();
    bacnet_services::write_property::WritePropertyRequest {
        object_identifier: device(),
        property_identifier: description,
        property_array_index: None,
        property_value: value.to_vec(),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    let write = ConfirmedServiceChoice::WRITE_PROPERTY;
    node.request(&to, 3, write, &body).await;
    assert_eq!(
        node.quiet(&to).await,
        [Bytes::from(vec![0x20, 3, write.to_raw()])]
    );
    session.stop().await.unwrap();
}

/// The B/IP builder's `cov` is `with_cov`: the session executes both
/// services, refuses COV without a server role at start, and a bare
/// transport, which can't keep it, refuses it at the build.
#[tokio::test(start_paused = true)]
async fn the_builder_option_is_the_session_opt_in() {
    let builder = || {
        BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .database(fixture())
            .cov(ReportConfig::default())
    };
    let mut node = Node::new().await;
    let mut session = builder()
        .role(SessionRole::ServerOnly)
        .build_session_over(bus_port())
        .unwrap();
    let services = session.executed_services();
    assert!(services.contains(&ServiceSupported::SUBSCRIBE_COV));
    assert!(services.contains(&ServiceSupported::SUBSCRIBE_COV_PROPERTY));
    session.start().await.unwrap();
    let to = mac_of(&session);
    node.request(&to, 1, SUBSCRIBE_COV, &subscribe(7, Some(false), Some(60)))
        .await;
    assert_eq!(node.quiet(&to).await[0], acked(1));
    session.stop().await.unwrap();

    let mut client = builder()
        .role(SessionRole::ClientOnly)
        .build_session_over(bus_port())
        .unwrap();
    let error = client.start().await.expect_err("a server role");
    assert!(
        error.to_string().contains("COV requires a server role")
            && !error.to_string().contains("SubscribeCOV"),
        "{error}"
    );
    let Err(error) = builder().build_transport() else {
        panic!("a bare transport can't keep COV");
    };
    assert!(
        error.to_string().contains("COV requires build_session()"),
        "{error}"
    );
}

/// The MS/TP and SC builders take `cov` as the B/IP one does: their
/// sessions execute the three services, and the MS/TP builder's bare
/// transport refuses it.
#[tokio::test]
async fn the_mstp_and_sc_builders_take_cov_too() {
    use bacnet_transport::mstp::LoopbackSerial;
    let (line, _peer) = LoopbackSerial::pair();
    let mstp = crate::mstp::MstpEndpointBuilder::new(line, 3)
        .cov(ReportConfig::default())
        .build_session()
        .unwrap();
    let (ws, _hub) = bacnet_transport::sc::LoopbackWebSocket::pair();
    let sc = crate::sc::ScEndpointBuilder::new([2; 6], [0x11; 16])
        .cov(ReportConfig::default())
        .build_loopback_session(ws)
        .unwrap();
    for (link, services) in [
        ("MS/TP", mstp.executed_services()),
        ("SC", sc.executed_services()),
    ] {
        for service in [
            ServiceSupported::SUBSCRIBE_COV,
            ServiceSupported::SUBSCRIBE_COV_PROPERTY,
            ServiceSupported::SUBSCRIBE_COV_PROPERTY_MULTIPLE,
        ] {
            assert!(services.contains(&service), "{link} {service:?}");
        }
    }
    let (line, _peer) = LoopbackSerial::pair();
    let Err(error) = crate::mstp::MstpEndpointBuilder::new(line, 3)
        .cov(ReportConfig::default())
        .build_transport()
    else {
        panic!("a bare MS/TP transport can't keep COV");
    };
    assert!(
        error.to_string().contains("COV requires build_session()"),
        "{error}"
    );
}

/// A confirmed property subscription's notifications carry the server's
/// parameters, each holds an invoke ID of the session's pool, and its Ack
/// gives the ID back.
#[tokio::test(start_paused = true)]
async fn a_confirmed_property_subscription_is_answered_as_on_the_server() {
    let mut node = Node::new().await;
    let mut server = server().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let body = property_body(ai1(), PV, None, 7, Some(true), Some(300), None);
    let server_mac = MacAddr::from_slice(server.local_mac());
    let mut heard = Vec::new();
    for to in [server_mac.clone(), mac_of(&session)] {
        let on_server = to == server_mac;
        node.request(&to, 1, PROPERTY, &body).await;
        let sent = node.quiet(&to).await;
        assert_eq!(sent[0], property_acked(1));
        let (id, first) = confirmed_notification(&sent[1]);
        if !on_server {
            assert_eq!(session.active_leases(), 1, "one ID of the pool");
        }
        node.send(&to, &ack(id)).await;
        assert!(node.quiet(&to).await.is_empty());
        let value = PropertyValue::Real(25.0);
        if on_server {
            server.set_present_value_local(&ai1(), value).await.unwrap();
        } else {
            session
                .set_present_value_local(&ai1(), value)
                .await
                .unwrap();
        }
        let (id, change) = confirmed_notification(&alone("a change", &node.quiet(&to).await));
        node.send(&to, &ack(id)).await;
        assert!(node.quiet(&to).await.is_empty());
        heard.push((first, change));
    }
    assert_eq!(heard[1], heard[0], "endpoint, then server");
    eventually(
        Duration::from_secs(1),
        "the Ack gives its ID back",
        async || session.active_leases() == 0,
    )
    .await;
    session.stop().await.unwrap();
    server.stop().await.unwrap();
}

/// Property subscriptions count against the per-subscriber and global caps
/// with object ones, and their reports against the outbox: with one entry,
/// a change to three subscriptions goes one after the other, the later
/// ones as room frees.
#[tokio::test(start_paused = true)]
async fn property_subscriptions_count_against_the_caps_and_the_outbox() {
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
    let full = (
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
    );
    a.request(&to, 1, SUBSCRIBE_COV, &subscribe(1, Some(false), Some(300)))
        .await;
    assert_eq!(a.quiet(&to).await.len(), 2);
    a.request(&to, 2, PROPERTY, &subscribe_property(ai1(), PV, 1, None))
        .await;
    assert_eq!(a.quiet(&to).await.len(), 2);
    a.request(&to, 3, PROPERTY, &subscribe_property(ai1(), OOS, 1, None))
        .await;
    let reply = alone("past the per-subscriber cap", &a.quiet(&to).await);
    assert_eq!(refusal(&reply), full);
    b.request(&to, 1, PROPERTY, &subscribe_property(ai1(), PV, 1, None))
        .await;
    assert_eq!(b.quiet(&to).await.len(), 2, "another subscriber's");
    b.request(&to, 2, PROPERTY, &subscribe_property(ai1(), OOS, 1, None))
        .await;
    let reply = alone("past the global cap", &b.quiet(&to).await);
    assert_eq!(refusal(&reply), full);

    session
        .set_present_value_local(&ai1(), PropertyValue::Real(26.0))
        .await
        .unwrap();
    let value = |apdu: &Bytes| reported(apdu).3;
    let heard: Vec<_> = a.quiet(&to).await.iter().map(value).collect();
    assert_eq!(
        heard,
        [PropertyValue::Real(26.0), PropertyValue::Real(26.0)]
    );
    let heard: Vec<_> = b.quiet(&to).await.iter().map(value).collect();
    assert_eq!(heard, [PropertyValue::Real(26.0)]);
    let counters = session.cov.host().unwrap().lock().reporter.counters();
    assert_eq!(counters.owed, 2, "two reports waited for room");
    session.stop().await.unwrap();
}

/// Silent confirmed property subscribers hold at most
/// `COV_CONFIRMED_IN_FLIGHT` invoke IDs: one subscription past the bound
/// waits for an ID, and one Ack lets it go with no time passing.
#[tokio::test(start_paused = true)]
async fn silent_property_subscribers_hold_at_most_the_bound() {
    use crate::session::COV_CONFIRMED_IN_FLIGHT;
    let policy = CovPolicy {
        max_subscriptions_per_peer: 256,
        max_confirmed_in_flight_per_peer: 256,
        ..CovPolicy::default()
    };
    // A minute's timeout and no retry, so nothing ends on its own.
    let minute = FlightTiming::new(bacnet_time::Duration::from_secs(60), 0);
    let config = ReportConfig::new(policy).with_confirmed(minute);
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(config)).await;
    let to = mac_of(&session);
    let mut out = Vec::new();
    for process in 0..=COV_CONFIRMED_IN_FLIGHT as u32 {
        let invoke_id = process as u8;
        let body = property_body(ai1(), PV, None, process, Some(true), Some(300), None);
        node.request(&to, invoke_id, PROPERTY, &body).await;
        // Its reply, after any notification still on its way.
        loop {
            let (_, apdu) = node.next_within(Duration::from_millis(100)).await.unwrap();
            if apdu[0] >> 4 != 0 {
                assert_eq!(apdu, property_acked(invoke_id));
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
    assert!(host.lock().reporter.owes_invoke_id(), "the last one waits");

    let at = tokio::time::Instant::now();
    node.send(&to, &ack(out[0])).await;
    let (_, apdu) = node.next_within(Duration::from_secs(1)).await.unwrap();
    assert_eq!(tokio::time::Instant::now(), at, "with no time passing");
    confirmed_notification(&apdu);
    assert_eq!(session.active_leases(), COV_CONFIRMED_IN_FLIGHT);
    assert!(!host.lock().reporter.owes_invoke_id());
    session.stop().await.unwrap();
    assert_eq!(session.active_leases(), 0);
}
