//! A confirmed COV report that finds no invoke ID free is owed a look once
//! one comes back (#2661): on a standalone server with every ID leased, and
//! on a server seated on a node's link at its cap of 64. The look measures
//! from the kept baseline, waits for an ID rather than spinning, keeps the
//! order the references were owed in, and goes with its subscription.
//!
//! Time is paused; each test leases the IDs itself, so nothing but the
//! server's own notifications ends a lease unless the test drops one.
use super::cov_wire_test_support::*;
use super::notification_transactions::{NotificationReservation, SEAT_LEASES};
use super::warn_spy::WarnSpy;
use super::*;
use crate::server::test_transport::{SendLog, TestTransport};
use bacnet_device_core::cov::{CovNotificationKind, CovSubscription};
use bacnet_endpoint_core::endpoint_ingress::{EndpointIngress, IngressReceivers};
use bacnet_objects::analog::AnalogValueObject;
use bacnet_services::cov::{COVNotificationRequest, SubscribeCOVRequest};
use bacnet_types::enums::EngineeringUnits;
use std::sync::atomic::Ordering::Relaxed;

const COV: ConfirmedServiceChoice = ConfirmedServiceChoice::CONFIRMED_COV_NOTIFICATION;

/// Present_Value a COVNotification carries.
pub(super) fn pv_of(report: &COVNotificationRequest) -> Vec<u8> {
    report
        .list_of_values
        .iter()
        .find(|value| value.property_identifier == PV)
        .map(|value| value.value.clone())
        .expect("Present_Value reported")
}

/// SubscribeCOV with confirmed notifications for `process` on `object`,
/// for `lifetime` seconds, or a cancellation when it is `None`.
async fn subscribe_on(
    h: &mut Harness,
    process: u32,
    object: ObjectIdentifier,
    lifetime: Option<u32>,
) {
    let mut body = BytesMut::new();
    SubscribeCOVRequest {
        subscriber_process_identifier: process,
        monitored_object_identifier: object,
        issue_confirmed_notifications: lifetime.map(|_| true),
        lifetime,
    }
    .encode(&mut body)
    .unwrap();
    h.request(ConfirmedServiceChoice::SUBSCRIBE_COV, body).await;
    response(h).await.expect("answered");
}

/// Subscribe `process` to AV-1 for 300 s.
async fn subscribe(h: &mut Harness, process: u32) {
    subscribe_on(h, process, av1(), Some(300)).await;
}

/// Subscribe `process` to `object` and acknowledge its initial report.
pub(super) async fn subscribed_on(h: &mut Harness, process: u32, object: ObjectIdentifier) {
    subscribe_on(h, process, object, Some(300)).await;
    h.cov_notification().await;
    h.ack().await;
    h.settle().await;
}

/// A standalone server with a confirmed SubscribeCOV to AV-1 whose initial
/// report is acknowledged, so its baseline is 0.0.
async fn standalone() -> Harness {
    let mut h = Harness::start(ServerConfig::default()).await;
    subscribed_on(&mut h, 2661, av1()).await;
    h
}

/// Lease every invoke ID of `h`'s server to another peer.
pub(super) fn lease_all(h: &Harness) -> Vec<NotificationReservation> {
    h.server.notification_transactions.lease_every_invoke_id()
}

/// Refusals for want of an invoke ID logged so far: one per reservation
/// refused, since none is retried until an ID comes back.
pub(super) fn refusals(spy: &WarnSpy) -> usize {
    spy.warnings()
        .iter()
        .filter(|warning| warning.message.starts_with("No free invoke ID"))
        .count()
}

/// `(queued, delivered, dropped)` owed-ID revisits.
pub(super) fn owed_counts(h: &Harness) -> (u64, u64, u64) {
    let counters = &h.server.cov_counters;
    (
        counters.owed_id_revisits_queued.load(Relaxed),
        counters.owed_id_revisits_delivered.load(Relaxed),
        counters.owed_id_revisits_dropped.load(Relaxed),
    )
}

/// The references waiting for an invoke ID.
pub(super) async fn owed(h: &Harness) -> Vec<bacnet_device_core::cov::CovSubscriptionKey> {
    h.server.cov_table.read().await.revisits().owed()
}

/// A value that changes once while every invoke ID is leased is reported
/// once one comes back, with no further change to prompt it.
#[tokio::test(start_paused = true)]
async fn a_change_refused_an_invoke_id_goes_once_one_comes_back() {
    let h = standalone().await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    // Long past any retry or backstop: nothing goes while no ID is free.
    tokio::time::sleep(Duration::from_secs(60)).await;
    h.no_notification().await;
    assert_eq!(owed(&h).await.len(), 1);

    drop(held.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
    assert!(owed(&h).await.is_empty());
}

/// The initial report of a subscription made while every ID is leased
/// goes once one comes back.
#[tokio::test(start_paused = true)]
async fn an_initial_report_refused_an_invoke_id_goes_once_one_comes_back() {
    let mut h = Harness::start(ServerConfig::default()).await;
    let mut held = lease_all(&h);
    subscribe(&mut h, 2661).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    h.no_notification().await;

    drop(held.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(0.0));
}

/// A value back at its baseline by the time an ID comes back sends nothing:
/// the look measures from the baseline the subscriber acknowledged.
#[tokio::test(start_paused = true)]
async fn a_value_back_at_its_baseline_before_the_look_sends_nothing() {
    let h = standalone().await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    h.write_local(0.0).await;
    drop(held.pop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
    assert!(owed(&h).await.is_empty());
}

/// Many changes while refused make one wait and one report, of the latest
/// value: the subscription is owed one look however often it is refused.
#[tokio::test(start_paused = true)]
async fn changes_while_refused_coalesce_into_one_report_of_the_latest() {
    let (spy, _guard) = WarnSpy::install();
    let h = standalone().await;
    let mut held = lease_all(&h);
    for value in [1.0, 2.0, 3.0, 4.0] {
        h.write_local(value).await;
    }
    assert_eq!(refusals(&spy), 4);
    assert_eq!(owed(&h).await.len(), 1);

    drop(held.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(4.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
    assert_eq!(owed_counts(&h), (1, 1, 0));
}

/// A cancelled subscription's wait goes with it.
#[tokio::test(start_paused = true)]
async fn a_cancelled_subscription_drops_its_wait() {
    let mut h = standalone().await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    subscribe_on(&mut h, 2661, av1(), None).await;
    assert!(owed(&h).await.is_empty());
    assert_eq!(owed_counts(&h), (1, 0, 1));

    drop(held.pop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
}

/// An expired subscription's wait goes with it when the purge removes it.
#[tokio::test(start_paused = true)]
async fn an_expired_subscription_drops_its_wait() {
    let mut h = Harness::start(ServerConfig::default()).await;
    subscribe_on(&mut h, 2661, av1(), Some(5)).await;
    h.cov_notification().await;
    h.ack().await;
    h.settle().await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    assert_eq!(owed(&h).await.len(), 1);
    // The purge runs every 30 s.
    tokio::time::sleep(Duration::from_secs(35)).await;
    assert!(owed(&h).await.is_empty());
    assert_eq!(owed_counts(&h), (1, 0, 1));

    drop(held.pop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
}

/// A re-subscription replaces the old wait: its own initial report, refused
/// too, waits in its place, and one report of the current value goes.
#[tokio::test(start_paused = true)]
async fn a_resubscription_replaces_the_wait_with_its_own() {
    let mut h = standalone().await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    subscribe(&mut h, 2661).await;
    // The old wait dropped, the initial report's queued.
    assert_eq!(owed_counts(&h), (2, 0, 1));
    assert_eq!(owed(&h).await.len(), 1);

    drop(held.pop());
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
    h.ack().await;
    h.settle().await;
    h.no_notification().await;
}

/// A server stopped with references waiting stops at once, and sends
/// nothing when IDs come back afterwards.
#[tokio::test(start_paused = true)]
async fn a_stop_with_references_waiting_sends_nothing_after() {
    let mut h = standalone().await;
    let held = lease_all(&h);
    h.write_local(42.0).await;
    tokio::time::timeout(Duration::from_secs(5), h.server.stop())
        .await
        .expect("stop does not wait for an ID")
        .unwrap();
    drop(held);
    tokio::time::sleep(Duration::from_secs(1)).await;
    h.no_notification().await;
}

/// References refused at once are served as IDs come back, one per ID, in
/// the order they were refused, which here is not the order of their keys
/// (by monitored object first): AV-2's subscribers were refused first. No
/// reservation is tried again while none is free, so the refusals stay at
/// one each however long the wait.
#[tokio::test(start_paused = true)]
async fn references_refused_together_go_in_the_order_they_were_refused() {
    let (spy, _guard) = WarnSpy::install();
    let av2 = ObjectIdentifier::new(ObjectType::ANALOG_VALUE, 2).unwrap();
    let mut h = Harness::start_with(ServerConfig::default(), |db| {
        db.add(Box::new(
            AnalogValueObject::new(2, "AV-2", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
        ))
        .unwrap();
    })
    .await;
    for (process, object) in [(1, av2), (2, av2), (3, av1()), (4, av1())] {
        subscribed_on(&mut h, process, object).await;
    }
    let mut held = lease_all(&h);
    h.write_local_to(av2, 5.0).await;
    h.write_local(6.0).await;
    assert_eq!(refusals(&spy), 4);
    // Let the server run with every ID leased: it tries nothing more.
    for _ in 0..200 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(refusals(&spy), 4, "nothing is tried while no ID is free");
    h.no_notification().await;

    // One ID, passed on by each Ack.
    drop(held.pop());
    let mut served = Vec::new();
    for _ in 0..4 {
        let report = h.cov_notification().await;
        h.no_notification().await;
        served.push(report.subscriber_process_identifier);
        h.ack().await;
        h.settle().await;
    }
    assert_eq!(served, [1, 2, 3, 4]);
    assert_eq!(refusals(&spy), 4);
    assert_eq!(owed_counts(&h), (4, 4, 0));
}

/// A look that comes due while a writer holds the database waits for it,
/// holding nothing the writer needs, and reports once it is let go: the
/// wait for an ID has given the reference up to the look, and the look
/// keeps it.
#[tokio::test(start_paused = true)]
async fn a_look_waits_for_the_database_and_loses_nothing() {
    let h = standalone().await;
    let mut held = lease_all(&h);
    h.write_local(42.0).await;
    let db = h.server.db.write().await;
    drop(held.pop());
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(owed(&h).await.is_empty(), "taken for the look");
    // The table is free for others meanwhile.
    drop(h.server.cov_table.write().await);
    h.no_notification().await;
    drop(db);
    assert_eq!(pv_of(&h.cov_notification().await), real(42.0));
}

/// The subscriber of [`seated`]'s subscription.
const SUBSCRIBER: [u8; 1] = [0x11];

/// A server on a node's seat, with AV-1, and the node's send log.
struct Seated {
    _ingress: EndpointIngress<TestTransport>,
    _receivers: IngressReceivers,
    /// The link stops when its last sender goes.
    _inbound: tokio::sync::mpsc::Sender<bacnet_transport::port::ReceivedNpdu>,
    sent: SendLog,
    server: BACnetServer<TestTransport>,
}

async fn seated() -> Seated {
    let (transport, inbound) = TestTransport::inbound(8);
    let sent = transport.sent();
    let mut ingress = EndpointIngress::new(transport, 8);
    let receivers = ingress.start().await.unwrap();
    let coordinator =
        Arc::new(bacnet_endpoint_core::coordinator::OutboundTransactionCoordinator::new());
    let seat = ingress.server_seat(coordinator).unwrap();
    let identity = bacnet_objects::DeviceIdentity::new(2661, 0).unwrap();
    let mut db = identity.clone().build_database().unwrap();
    db.add(Box::new(
        AnalogValueObject::new(1, "AV-1", EngineeringUnits::DEGREES_CELSIUS).unwrap(),
    ))
    .unwrap();
    let server = BACnetServer::generic_builder()
        .device(identity)
        .database(db)
        .seat(seat)
        .build()
        .await
        .unwrap();
    Seated {
        _ingress: ingress,
        _receivers: receivers,
        _inbound: inbound,
        sent,
        server,
    }
}

impl Seated {
    /// Admit a confirmed whole-object subscription of [`SUBSCRIBER`] to
    /// AV-1, with no baseline yet.
    async fn subscribe(&self) {
        let sub = CovSubscription {
            subscriber_mac: MacAddr::from_slice(&SUBSCRIBER),
            subscriber_network: None,
            subscriber_process_identifier: 2661,
            monitored_object_identifier: av1(),
            issue_confirmed_notifications: true,
            expires_at: Some(
                crate::runtime_clock::core_now() + bacnet_time::Duration::from_secs(3600),
            ),
            last_notified_observation: None,
            monitored_property: None,
            monitored_property_array_index: None,
            cov_increment: None,
            notification_kind: CovNotificationKind::Single,
            timestamped: false,
        };
        self.server
            .cov_table
            .write()
            .await
            .admit_for_test(sub, 0)
            .unwrap();
    }

    /// Every ConfirmedCOVNotification sent to [`SUBSCRIBER`] so far.
    fn reports(&self) -> Vec<COVNotificationRequest> {
        self.sent
            .frames()
            .into_iter()
            .filter(|frame| frame.mac.as_slice() == SUBSCRIBER)
            .filter_map(|frame| {
                let npdu = bacnet_encoding::npdu::decode_npdu(frame.npdu).ok()?;
                match apdu::decode_apdu(npdu.payload).ok()? {
                    Apdu::ConfirmedRequest(request) if request.service_choice == COV => {
                        COVNotificationRequest::decode(&request.service_request).ok()
                    }
                    _ => None,
                }
            })
            .collect()
    }

    async fn write(&self, value: f32) {
        self.server
            .write_local(
                &av1(),
                PV,
                None,
                PropertyValue::Real(value),
                Some(8),
                crate::LocalCommandSource::ServerDevice,
            )
            .await
            .unwrap();
    }
}

/// On a node's seat the server holds at most 64 invoke IDs: a change refused
/// one there is reported once one of the server's own leases ends.
#[tokio::test(start_paused = true)]
async fn a_seated_change_refused_at_the_cap_goes_once_a_lease_ends() {
    let mut s = seated().await;
    let pool = Arc::clone(&s.server.notification_transactions);
    let mut held: Vec<_> = (0..SEAT_LEASES)
        .map(|_| {
            pool.reserve(canonical_direct_peer(&[0x22]), COV)
                .expect("an ID within the cap")
        })
        .collect();
    s.subscribe().await;
    s.write(42.0).await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert!(s.reports().is_empty(), "{:?}", s.reports());

    drop(held.pop());
    for _ in 0..5_000 {
        if !s.reports().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let reports = s.reports();
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(pv_of(&reports[0]), real(42.0));
    drop(held);
    s.server.stop().await.unwrap();
}
