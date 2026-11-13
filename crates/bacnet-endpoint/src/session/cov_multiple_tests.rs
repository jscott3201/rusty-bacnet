//! SubscribeCOVPropertyMultiple on an endpoint session (#2490), on paused
//! time over the in-memory link: the opt-in, `Reporter`, send path and
//! bounds of SubscribeCOV. Both hosts read one fixed Device clock, so every
//! Time_Of_Change agrees, and a context's replies, reports and the Device's
//! Active_COV_Multiple_Subscriptions match `bacnet-server`'s octet for
//! octet: a timestamped and an untimestamped reference, two contexts to one
//! object, a report split to a small subscriber, a cancellation, a renewal,
//! the lifetime running out, the refusals and the caps. Confirmed reports
//! carry the server's parameters, hold IDs of the session's pool and count
//! against its bound. Without a clock the endpoint refuses a timestamped
//! reference.

use super::*;
use bacnet_device_core::cov::{CovPolicy, FlightTiming};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_services::cov_multiple::{
    COVReference, COVSubscriptionSpecification, SubscribeCOVPropertyMultipleRequest,
};
use bacnet_types::constructed::PropertyReference;
use bacnet_types::enums::{ErrorClass, ErrorCode};
use bacnet_types::primitives::{Date, Time};

const MULTIPLE: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY_MULTIPLE;
const NOTIFY: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION_MULTIPLE;
const PV: PropertyIdentifier = PropertyIdentifier::PRESENT_VALUE;
/// Max_Notification_Delay of every context here, in seconds.
const DELAY: u32 = 10;

#[path = "cov_multiple_timing_tests.rs"]
mod timing;

/// A SubscribeCOVPropertyMultiple body for `process`: Present_Value of each
/// of `references`, timestamped or not, for `timing`'s lifetime and
/// Max_Notification_Delay; a cancellation without `timing`.
pub(super) fn multiple_body(
    process: u32,
    confirmed: bool,
    references: &[(ObjectIdentifier, bool)],
    timing: Option<(u32, u32)>,
) -> Vec<u8> {
    let references: Vec<_> = references
        .iter()
        .map(|&(object, timestamped)| (object, PV, timestamped))
        .collect();
    multiple_of(process, confirmed, &references, timing)
}

/// [`multiple_body`] naming each reference's property, an object's
/// references one specification.
fn multiple_of(
    process: u32,
    confirmed: bool,
    references: &[(ObjectIdentifier, PropertyIdentifier, bool)],
    timing: Option<(u32, u32)>,
) -> Vec<u8> {
    let mut specifications: Vec<COVSubscriptionSpecification> = Vec::new();
    for &(object, property, timestamped) in references {
        let reference = COVReference {
            monitored_property: PropertyReference {
                property_identifier: property,
                property_array_index: None,
            },
            cov_increment: None,
            timestamped,
        };
        match specifications.last_mut() {
            Some(last) if last.monitored_object_identifier == object => {
                last.list_of_cov_references.push(reference);
            }
            _ => specifications.push(COVSubscriptionSpecification {
                monitored_object_identifier: object,
                list_of_cov_references: vec![reference],
            }),
        }
    }
    let mut body = BytesMut::new();
    SubscribeCOVPropertyMultipleRequest {
        subscriber_process_identifier: process,
        issue_confirmed_notifications: confirmed,
        lifetime: timing.map(|(lifetime, _)| lifetime),
        max_notification_delay: timing.map(|(_, delay)| delay),
        list_of_cov_subscription_specifications: specifications,
    }
    .encode(&mut body)
    .unwrap();
    body.to_vec()
}

/// A SimpleACK of SubscribeCOVPropertyMultiple under `invoke_id`.
fn multiple_acked(invoke_id: u8) -> Bytes {
    Bytes::from(vec![0x20, invoke_id, MULTIPLE.to_raw()])
}

/// A ConfirmedCOVNotificationMultiple's invoke ID and service parameters.
fn confirmed_report(apdu: &[u8]) -> (u8, Bytes) {
    let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(Bytes::copy_from_slice(apdu)) else {
        panic!("a confirmed request: {apdu:02X?}");
    };
    assert_eq!(request.service_choice, NOTIFY);
    (request.invoke_id, request.service_request)
}

/// A SimpleACK of ConfirmedCOVNotificationMultiple under `invoke_id`.
fn report_acked(invoke_id: u8) -> [u8; 3] {
    [0x20, invoke_id, NOTIFY.to_raw()]
}

/// The class and code of a SubscribeCOVPropertyMultiple-Error.
fn refusal(apdu: &Bytes) -> (ErrorClass, ErrorCode) {
    let Ok(Apdu::Error(error)) = decode_apdu(apdu.clone()) else {
        panic!("an Error: {apdu:02X?}");
    };
    assert_eq!(error.service_choice, MULTIPLE);
    (error.error_class, error.error_code)
}

/// The Device clock both hosts read.
struct Fixed;

impl ClockReader for Fixed {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: Date {
                year: 126,
                month: 11,
                day: 2,
                day_of_week: 1,
            },
            local_time: Time {
                hour: 10,
                minute: 30,
                second: 15,
                hundredths: 31,
            },
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

impl Pair {
    /// [`Pair::start`] with both databases reading the fixed clock.
    async fn clocked() -> Self {
        Self::clocked_under(CovPolicy::default()).await
    }

    /// [`Pair::clocked`] with both hosts under `policy`.
    async fn clocked_under(policy: CovPolicy) -> Self {
        let mut db = fixture();
        db.set_clock_reader(Some(Arc::new(Fixed)));
        let mut endpoint = BipEndpointBuilder::new(Ipv4Addr::LOCALHOST, 0, Ipv4Addr::BROADCAST)
            .role(SessionRole::ServerOnly)
            .database(db)
            .cov(ReportConfig::new(policy.clone()))
            .build_session_over(BusPort::new())
            .unwrap();
        endpoint.start().await.unwrap();
        let server = BACnetServer::generic_builder()
            .device(DeviceIdentity::new(DEVICE, 0x105).unwrap())
            .transport(BusPort::new())
            .database(fixture())
            .cov_policy(policy)
            .build()
            .await
            .unwrap();
        // Start installs the system clock; serve the fixed one.
        server
            .database()
            .write()
            .await
            .set_clock_reader(Some(Arc::new(Fixed)));
        Self {
            node: Node::new().await,
            server,
            endpoint,
            invoke_id: 0,
        }
    }

    /// [`Pair::both`] for a SubscribeCOVPropertyMultiple whose header
    /// states a Max-APDU-Length-Accepted of 50 octets.
    async fn small(&mut self, what: &str, body: &[u8]) -> Vec<Bytes> {
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let mut apdu = BytesMut::new();
        let request = Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 50,
            invoke_id: self.invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: MULTIPLE,
            service_request: Bytes::copy_from_slice(body),
        });
        encode_apdu(&mut apdu, &request).unwrap();
        let server = MacAddr::from_slice(self.server.local_mac());
        let endpoint = mac_of(&self.endpoint);
        self.node.send(&server, &apdu).await;
        let from_server = self.node.quiet(&server).await;
        self.node.send(&endpoint, &apdu).await;
        let from_endpoint = self.node.quiet(&endpoint).await;
        assert_eq!(from_endpoint, from_server, "{what}: endpoint, then server");
        from_endpoint
    }
}

/// Unconfirmed reports match the server's, from a context's reply and
/// initial report through changes, a second context to the same object,
/// a report split for a small subscriber and a cancellation, as does the
/// Device's list at each step.
#[tokio::test(start_paused = true)]
async fn unconfirmed_multiple_reports_match_the_server() {
    let mut pair = Pair::clocked().await;
    let list = read_device(PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS);
    let timing = Some((300, DELAY));
    let body = multiple_body(1, false, &[(ai1(), true), (lsp1(), false)], timing);
    let sent = pair.both("subscribe", MULTIPLE, &body).await;
    assert_eq!(sent.len(), 2, "the reply, then the initial report");
    assert_eq!(sent[0], multiple_acked(pair.invoke_id));
    pair.both("listed", READ_PROPERTY, &list).await;
    assert_eq!(pair.change("a change", 25.0).await.len(), 1);
    assert!(pair.change("no change", 25.0).await.is_empty());
    let oos = PropertyIdentifier::OUT_OF_SERVICE;
    // Out of service and back: Status_Flags reports each way, and AI-1
    // takes the application's values again.
    for on in [true, false] {
        let sent = pair
            .write("Status_Flags", ai1(), oos, PropertyValue::Boolean(on))
            .await;
        assert_eq!(sent.len(), 1, "out of service {on}");
    }

    let untimed = multiple_body(2, false, &[(ai1(), false)], timing);
    assert_eq!(
        pair.both("a second context", MULTIPLE, &untimed)
            .await
            .len(),
        2
    );
    assert_eq!(pair.change("both contexts", 26.0).await.len(), 2);
    let split = multiple_body(3, false, &[(ai1(), false), (lsp1(), false)], timing);
    let sent = pair.small("split", &split).await;
    assert_eq!(sent.len(), 3, "the reply, then one report per object");
    pair.both("three contexts", READ_PROPERTY, &list).await;

    let cancel = multiple_body(1, false, &[], None);
    let sent = pair.both("cancel", MULTIPLE, &cancel).await;
    assert_eq!(sent, [multiple_acked(pair.invoke_id)]);
    pair.both("two contexts", READ_PROPERTY, &list).await;
    assert_eq!(pair.change("the other two", 27.0).await.len(), 2);
    pair.stop().await;
}

/// A renewal reports again and restarts the lifetime, which then runs out
/// on the session's clock; each step as the server takes it.
#[tokio::test(start_paused = true)]
async fn a_context_renews_and_expires_as_on_the_server() {
    let mut pair = Pair::clocked().await;
    let list = read_device(PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS);
    let lasting = |lifetime| multiple_body(1, false, &[(ai1(), false)], Some((lifetime, DELAY)));
    pair.both("subscribe", MULTIPLE, &lasting(30)).await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    pair.both("the list after 10 s", READ_PROPERTY, &list).await;
    let sent = pair.both("renew", MULTIPLE, &lasting(60)).await;
    assert_eq!(sent.len(), 2, "the reply, then a report");
    tokio::time::sleep(Duration::from_secs(59)).await;
    assert_eq!(
        pair.change("a second before it lapses", 21.0).await.len(),
        1
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let host = pair.endpoint.cov.host().unwrap();
    assert!(
        host.lock().reporter.table().is_empty(),
        "purged on the clock"
    );
    pair.both("the list once lapsed", READ_PROPERTY, &list)
        .await;
    assert!(pair.change("lapsed", 22.0).await.is_empty());
    pair.stop().await;
}

/// Each refusal matches the server's octet for octet, the Device's own
/// Present_Value among them, and so does what a refusal after an accepted
/// reference keeps: the reference before it, and its report.
#[tokio::test(start_paused = true)]
async fn multiple_refusals_match_the_server() {
    let mut pair = Pair::clocked().await;
    let timing = Some((60, DELAY));
    let gone = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 99).unwrap();
    // Max_Notification_Delay as long as the lifetime: the encoder refuses
    // it, so the delay's octet is set by hand.
    let mut too_late = multiple_body(1, false, &[(ai1(), false)], timing);
    let at = too_late
        .windows(2)
        .position(|pair| pair == [0x39, DELAY as u8])
        .unwrap();
    too_late[at + 1] = 60;
    let cases = [
        (
            "the Device, which reports no COV",
            multiple_body(1, false, &[(device(), false)], timing),
            (
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ),
        ),
        (
            "no object",
            multiple_body(1, false, &[(gone, false)], timing),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            "a property AI-1 lacks",
            multiple_of(
                1,
                false,
                &[(ai1(), PropertyIdentifier::PRIORITY_ARRAY, false)],
                timing,
            ),
            (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
        ),
        (
            "a property its object doesn't report",
            multiple_of(
                1,
                false,
                &[(lsp1(), PropertyIdentifier::MODE, false)],
                timing,
            ),
            (ErrorClass::PROPERTY, ErrorCode::NOT_COV_PROPERTY),
        ),
        (
            "a delay as long as the lifetime",
            too_late,
            (ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE),
        ),
    ];
    for (what, body, expected) in cases {
        let sent = pair.both(what, MULTIPLE, &body).await;
        let [reply] = &sent[..] else {
            panic!("{what}: the reply alone: {sent:02X?}");
        };
        assert_eq!(refusal(reply), expected, "{what}");
    }
    let host = Arc::clone(pair.endpoint.cov.host().unwrap());
    assert!(host.lock().reporter.table().is_empty());

    let what = "a refusal after an accepted reference";
    let body = multiple_of(
        2,
        false,
        &[
            (ai1(), PV, false),
            (ai1(), PropertyIdentifier::PRIORITY_ARRAY, false),
        ],
        timing,
    );
    let sent = pair.both(what, MULTIPLE, &body).await;
    assert_eq!(
        sent.len(),
        2,
        "{what}: the Error, then the kept one's report"
    );
    assert_eq!(
        refusal(&sent[0]),
        (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY)
    );
    let list = read_device(PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS);
    pair.both("the kept reference", READ_PROPERTY, &list).await;
    pair.stop().await;
}

/// The caps count a context's references with the subscriber's other
/// subscriptions: past `max_subscriptions_per_peer` a context is refused
/// as on the server.
#[tokio::test(start_paused = true)]
async fn contexts_count_against_the_caps_as_on_the_server() {
    let policy = CovPolicy {
        max_subscriptions_per_peer: 2,
        ..CovPolicy::default()
    };
    let mut pair = Pair::clocked_under(policy).await;
    let timing = Some((300, DELAY));
    let object = subscribe(1, Some(false), Some(300));
    assert_eq!(
        pair.both("an object", SUBSCRIBE_COV, &object).await.len(),
        2
    );
    let one = multiple_body(1, false, &[(ai1(), false)], timing);
    assert_eq!(pair.both("one reference", MULTIPLE, &one).await.len(), 2);
    let two = multiple_body(2, false, &[(lsp1(), false)], timing);
    let sent = pair.both("past the cap", MULTIPLE, &two).await;
    let [reply] = &sent[..] else {
        panic!("the reply alone: {sent:02X?}");
    };
    assert_eq!(
        refusal(reply),
        (
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT
        )
    );
    // The refused context left nothing behind on either host.
    let list = read_device(PropertyIdentifier::ACTIVE_COV_MULTIPLE_SUBSCRIPTIONS);
    pair.both("the first context alone", READ_PROPERTY, &list)
        .await;
    pair.stop().await;
}

/// Without a clock the database reads, the endpoint refuses a timestamped
/// reference with SERVICES / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED and
/// takes an untimestamped one.
#[tokio::test(start_paused = true)]
async fn a_clockless_endpoint_refuses_a_timestamped_reference() {
    let mut node = Node::new().await;
    let mut session = endpoint(SessionRole::ServerOnly, Some(ReportConfig::default())).await;
    let to = mac_of(&session);
    let timing = Some((300, DELAY));
    node.request(
        &to,
        1,
        MULTIPLE,
        &multiple_body(1, false, &[(ai1(), true)], timing),
    )
    .await;
    let sent = node.quiet(&to).await;
    assert_eq!(sent.len(), 1, "the Error alone");
    assert_eq!(
        refusal(&sent[0]),
        (
            ErrorClass::SERVICES,
            ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED
        )
    );
    node.request(
        &to,
        2,
        MULTIPLE,
        &multiple_body(1, false, &[(ai1(), false)], timing),
    )
    .await;
    let sent = node.quiet(&to).await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0], multiple_acked(2));
    session.stop().await.unwrap();
}

/// A confirmed context's reports carry the server's parameters: the
/// initial one, then, held behind it, a change that its Ack lets go with
/// the timestamped history. Each holds an ID of the session's pool, and
/// the Acks give them back.
#[tokio::test(start_paused = true)]
async fn confirmed_multiple_reports_follow_their_acks_as_on_the_server() {
    let mut pair = Pair::clocked().await;
    let body = multiple_body(
        1,
        true,
        &[(ai1(), true), (lsp1(), false)],
        Some((600, DELAY)),
    );
    let server = MacAddr::from_slice(pair.server.local_mac());
    let mut heard = Vec::new();
    for to in [server.clone(), mac_of(&pair.endpoint)] {
        let on_server = to == server;
        pair.node.request(&to, 1, MULTIPLE, &body).await;
        let sent = pair.node.quiet(&to).await;
        assert_eq!(sent[0], multiple_acked(1));
        let (id, first) = confirmed_report(&sent[1]);
        if !on_server {
            assert_eq!(pair.endpoint.active_leases(), 1, "one ID of the pool");
        }
        let value = PropertyValue::Real(25.0);
        if on_server {
            pair.server.set_present_value_local(&ai1(), value).await
        } else {
            pair.endpoint.set_present_value_local(&ai1(), value).await
        }
        .unwrap();
        assert!(pair.node.quiet(&to).await.is_empty(), "held behind it");
        pair.node.send(&to, &report_acked(id)).await;
        let sent = pair.node.quiet(&to).await;
        let [follow_up] = &sent[..] else {
            panic!("the follow-up alone: {sent:02X?}");
        };
        let (id, follow_up) = confirmed_report(follow_up);
        pair.node.send(&to, &report_acked(id)).await;
        assert!(pair.node.quiet(&to).await.is_empty());
        heard.push((first, follow_up));
    }
    assert_eq!(heard[1], heard[0], "endpoint, then server");
    assert_eq!(pair.endpoint.active_leases(), 0);
    pair.stop().await;
}

/// Silent confirmed contexts hold at most `COV_CONFIRMED_IN_FLIGHT` invoke
/// IDs, with the single subscriptions' bound: one context past it waits for
/// an ID, and one Ack lets its report go with no time passing.
#[tokio::test(start_paused = true)]
async fn silent_confirmed_contexts_hold_at_most_the_bound() {
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
        let body = multiple_body(process, true, &[(ai1(), false)], Some((300, DELAY)));
        node.request(&to, invoke_id, MULTIPLE, &body).await;
        // Its reply, after any report still on its way.
        loop {
            let (_, apdu) = node.next_within(Duration::from_millis(100)).await.unwrap();
            if apdu[0] >> 4 != 0 {
                assert_eq!(apdu, multiple_acked(invoke_id));
                break;
            }
            out.push(confirmed_report(&apdu).0);
        }
    }
    while let Some((_, apdu)) = node.next_within(Duration::from_millis(100)).await {
        out.push(confirmed_report(&apdu).0);
    }
    assert_eq!(out.len(), COV_CONFIRMED_IN_FLIGHT);
    assert_eq!(session.active_leases(), COV_CONFIRMED_IN_FLIGHT);
    let host = Arc::clone(session.cov.host().unwrap());
    assert!(host.lock().reporter.owes_invoke_id(), "the last one waits");

    let at = tokio::time::Instant::now();
    node.send(&to, &report_acked(out[0])).await;
    let (_, apdu) = node.next_within(Duration::from_secs(1)).await.unwrap();
    assert_eq!(tokio::time::Instant::now(), at, "with no time passing");
    confirmed_report(&apdu);
    assert_eq!(session.active_leases(), COV_CONFIRMED_IN_FLIGHT);
    assert!(!host.lock().reporter.owes_invoke_id());
    session.stop().await.unwrap();
    assert_eq!(session.active_leases(), 0);
}
