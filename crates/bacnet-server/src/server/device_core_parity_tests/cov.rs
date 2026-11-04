//! SubscribeCOV and SubscribeCOVProperty (#2303): the server and
//! bacnet-device-core's `Reporter`, given the same requests from the same
//! subscribers over the same fixture, send the same reply octets and hold
//! the same subscriptions. The notifications both send are compared in
//! `notify`.
//!
//! The server serves one database on the shared `TestTransport`; the
//! `Reporter` serves the other, at the instant the request reached the
//! server. After each request the two tables must hold the same entries
//! (identity, route, form, lifetime and increment) and have counted the
//! same; the server's first notification follows every accepted
//! subscription.
use super::*;
use crate::runtime_clock;
use bacnet_device_core::cov::{CovCounters, CovPolicy};
use bacnet_encoding::npdu::NpduAddress;
use bacnet_objects::command_source::{CommandDeviceBinding, CommandOrigin};

mod notify;

/// Both sides of one fixture under one COV policy.
struct CovParity {
    parity: Parity,
    invoke_id: u8,
    /// The core side's invoke IDs.
    ids: notify::Ids,
    /// Confirmed notifications both sides sent and nobody answered yet.
    outstanding: Vec<notify::Outstanding>,
}

impl CovParity {
    async fn start(policy: CovPolicy) -> Self {
        let identity = identity();
        let (transport, tx) = TestTransport::inbound(16);
        let sent = transport.sent();
        let config = ServerConfig {
            device: identity.clone(),
            cov_policy: policy.clone(),
            ..ServerConfig::default()
        };
        let server = BACnetServer::start(config, fixture(&identity), transport)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        sent.clear();
        let responder = Responder::new(
            identity.clone(),
            Limits::new(ReadPropertyMultipleBudget::default().max_result_elements),
        )
        .unwrap();
        let reporter = reporter(responder, &server.config);
        Self {
            parity: Parity {
                server,
                tx,
                sent,
                reporter,
                db: fixture(&identity),
                requests: 0,
            },
            invoke_id: 0,
            ids: notify::Ids::default(),
            outstanding: Vec::new(),
        }
    }

    /// Send `service` with `body` from `peer` to both, and return the
    /// server's reply, which must equal the core's.
    async fn request(
        &mut self,
        what: &str,
        peer: &SubscriberEndpoint,
        service: ConfirmedServiceChoice,
        body: &[u8],
    ) -> Apdu {
        let (reply, _) = self.exchange(what, peer, service, body).await;
        self.same_tables(what).await;
        reply
    }

    /// Send `service` with `body` from `peer` to both: the reply, which
    /// must match, and how many notifications followed it, which must
    /// match too.
    async fn exchange(
        &mut self,
        what: &str,
        peer: &SubscriberEndpoint,
        service: ConfirmedServiceChoice,
        body: &[u8],
    ) -> (Apdu, usize) {
        let now = runtime_clock::core_now();
        let (frames, core) = self.both(peer, service, body).await;
        let server = frames
            .first()
            .unwrap_or_else(|| panic!("{what}: the server sent nothing"))
            .decode_npdu()
            .payload;
        assert_eq!(server, core, "{what}: server, then core");
        let reply = bacnet_encoding::apdu::decode_apdu(core).unwrap();
        let notifications = self.same_notifications(what, &frames[1..], now);
        (reply, notifications)
    }

    /// Send `service` with `body` from `peer` to both: what the server
    /// sent, and the core's reply.
    async fn both(
        &mut self,
        peer: &SubscriberEndpoint,
        service: ConfirmedServiceChoice,
        body: &[u8],
    ) -> (Vec<test_transport::SentFrame>, Bytes) {
        self.invoke_id = self.invoke_id.wrapping_add(1);
        let apdu = confirmed(self.invoke_id, service, body, 1476);
        let now = runtime_clock::core_now();
        let frames = self.deliver(&apdu, peer).await;
        let origin = CommandOrigin::Remote {
            actual_address: bacnet_device_core::report::peer_address(peer),
            binding: CommandDeviceBinding::Unknown,
        };
        let request = Inbound::new(apdu, 1476).with_source(origin);
        let mut out = BytesMut::new();
        let parity = &mut self.parity;
        parity
            .reporter
            .handle(&mut parity.db, peer, request, now, &mut out);
        (frames, out.freeze())
    }

    /// Deliver `apdu` to the server from `peer`, routed when it has a
    /// network, and return what the server sent.
    async fn deliver(
        &mut self,
        apdu: &Bytes,
        peer: &SubscriberEndpoint,
    ) -> Vec<test_transport::SentFrame> {
        let mut npdu = BytesMut::new();
        encode_npdu(
            &mut npdu,
            &Npdu {
                expecting_reply: apdu[0] >> 4 == 0,
                source: peer.network.clone(),
                payload: apdu.clone(),
                ..Npdu::default()
            },
        )
        .unwrap();
        self.parity
            .tx
            .send(ReceivedNpdu {
                direct_response: None,
                npdu: npdu.freeze(),
                source_mac: peer.mac.clone(),
                link_layer_group: false,
                data_attributes: Vec::new(),
                provenance: TransportProvenance::unverified(),
                reply_tx: None,
            })
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        self.parity.sent.take()
    }

    /// Both tables hold the same subscriptions and counted the same.
    async fn same_tables(&mut self, what: &str) {
        let shape = |entry: &bacnet_device_core::cov::CovSubscriptionSnapshot| {
            (
                entry.key().clone(),
                entry.endpoint(),
                entry.issue_confirmed_notifications,
                entry.expires_at,
                entry.cov_increment.map(f32::to_bits),
            )
        };
        let table = self.parity.server.cov_table.read().await;
        let server: Vec<_> = table.entries().map(shape).collect();
        let core: Vec<_> = self.parity.reporter.table().entries().map(shape).collect();
        assert_eq!(server, core, "{what}: the subscriptions, server then core");
        let total = &self.parity.reporter.counters().cov;
        let server = table.counters().snapshot();
        let subscription_counts = |c: &CovCounters| {
            [
                c.subscriptions_active,
                c.subscriptions_created,
                c.subscriptions_cancelled,
                c.subscriptions_purged,
                c.subscriptions_rejected_quota,
                c.subscriptions_rejected_capacity,
                c.subscriptions_rejected_indefinite,
            ]
        };
        assert_eq!(
            subscription_counts(&server),
            subscription_counts(total),
            "{what}: the counts, server then core"
        );
    }

    async fn stop(mut self) {
        self.parity.server.stop().await.unwrap();
    }
}

/// The parity fixture, a Life Safety Point, whose Mode is readable but not
/// reported, an Averaging object, which takes property subscriptions but
/// refuses SubscribeCOV, and an object with a Table 13-1 row of its own
/// ([`notify::Staged`]).
fn fixture(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let point = bacnet_objects::life_safety::LifeSafetyPointObject::new(1, "LSP-1").unwrap();
    let averaging = bacnet_objects::averaging::AveragingObject::new(1, "AVG-1").unwrap();
    database_with(
        identity,
        vec![
            Box::new(point),
            Box::new(averaging),
            Box::new(notify::Staged::new()),
        ],
    )
}

fn averaging() -> ObjectIdentifier {
    oid(ObjectType::AVERAGING, 1)
}

fn direct(mac: u8) -> SubscriberEndpoint {
    SubscriberEndpoint::new(&[10, 0, 0, mac, 0xBA, 0xC0], None)
}

/// A client on network 7 behind the router at `router`.
fn routed(router: u8) -> SubscriberEndpoint {
    let source = NpduAddress {
        network: 7,
        mac_address: MacAddr::from_slice(&[0x42]),
    };
    SubscriberEndpoint::new(&[10, 0, 0, router, 0xBA, 0xC0], Some(&source))
}

/// A SubscribeCOV body, written field by field so a malformed pairing
/// (a lifetime without the confirmed flag) can be sent too.
fn subscribe(
    process: u32,
    object: ObjectIdentifier,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) -> Vec<u8> {
    let mut body = BytesMut::new();
    head(&mut body, process, object, confirmed, lifetime);
    body.to_vec()
}

/// A SubscribeCOVProperty body with an increment of 0.5, written as
/// [`subscribe`] is.
fn subscribe_property(
    process: u32,
    object: ObjectIdentifier,
    property: P,
    index: Option<u32>,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) -> Vec<u8> {
    use bacnet_encoding::{primitives, tags};
    let mut body = BytesMut::new();
    head(&mut body, process, object, confirmed, lifetime);
    tags::encode_opening_tag(&mut body, 4);
    primitives::encode_ctx_unsigned(&mut body, 0, u64::from(property.to_raw()));
    if let Some(index) = index {
        primitives::encode_ctx_unsigned(&mut body, 1, u64::from(index));
    }
    tags::encode_closing_tag(&mut body, 4);
    primitives::encode_ctx_real(&mut body, 5, 0.5);
    body.to_vec()
}

/// The four fields both services open with.
fn head(
    body: &mut BytesMut,
    process: u32,
    object: ObjectIdentifier,
    confirmed: Option<bool>,
    lifetime: Option<u32>,
) {
    use bacnet_encoding::primitives;
    primitives::encode_ctx_unsigned(body, 0, u64::from(process));
    primitives::encode_ctx_object_id(body, 1, &object);
    if let Some(confirmed) = confirmed {
        primitives::encode_ctx_boolean(body, 2, confirmed);
    }
    if let Some(lifetime) = lifetime {
        primitives::encode_ctx_unsigned(body, 3, u64::from(lifetime));
    }
}

const SUBSCRIBE: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV;
const PROPERTY: ConfirmedServiceChoice = ConfirmedServiceChoice::SUBSCRIBE_COV_PROPERTY;

fn ai() -> ObjectIdentifier {
    oid(ObjectType::ANALOG_INPUT, 1)
}

fn refused(reply: &Apdu) -> (ErrorClass, ErrorCode) {
    error_of(Some(reply.clone()))
}

#[tokio::test(start_paused = true)]
async fn subscribe_renew_and_cancel_match() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let (a, b) = (direct(1), direct(2));
    let body = subscribe(1, ai(), Some(false), Some(600));
    cov.request("subscribe", &a, SUBSCRIBE, &body).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    let renewal = subscribe(1, ai(), Some(true), Some(300));
    cov.request("renew, now confirmed", &a, SUBSCRIBE, &renewal)
        .await;
    let indefinite = subscribe(2, ai(), Some(false), Some(0));
    cov.request("indefinite", &a, SUBSCRIBE, &indefinite).await;
    let bv = oid(ObjectType::BINARY_VALUE, 1);
    let body = subscribe_property(1, bv, P::PRESENT_VALUE, None, Some(true), Some(60));
    cov.request("property", &b, PROPERTY, &body).await;
    let body = subscribe_property(
        3,
        averaging(),
        P::AVERAGE_VALUE,
        None,
        Some(false),
        Some(60),
    );
    let reply = cov
        .request(
            "property, an object refusing SubscribeCOV",
            &b,
            PROPERTY,
            &body,
        )
        .await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    let cancel = subscribe(1, ai(), None, None);
    cov.request("cancel", &a, SUBSCRIBE, &cancel).await;
    cov.request("cancel of nothing", &a, SUBSCRIBE, &cancel)
        .await;
    let cancel = subscribe_property(1, bv, P::PRESENT_VALUE, None, None, None);
    cov.request("cancel a property", &b, PROPERTY, &cancel)
        .await;
    // A cancellation is checked before the object: one for an object that
    // isn't there still succeeds (Clause 13.14.2).
    let gone = oid(ObjectType::ANALOG_VALUE, 99);
    let cancel = subscribe(1, gone, None, None);
    cov.request("cancel, no object", &a, SUBSCRIBE, &cancel)
        .await;
    // Past its lifetime, a subscription is purged by the next admission on
    // both sides.
    let short = subscribe(3, ai(), Some(false), Some(1));
    cov.request("one second", &b, SUBSCRIBE, &short).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let body = subscribe(4, ai(), Some(false), Some(60));
    cov.request("after an expiry", &b, SUBSCRIBE, &body).await;
    cov.stop().await;
}

/// Every refusal of design §5.8 the two services share.
#[tokio::test(start_paused = true)]
async fn refusals_match() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let a = direct(1);
    let gone = oid(ObjectType::ANALOG_VALUE, 99);
    let group = oid(ObjectType::GROUP, 1);
    for (what, service, body, expected) in [
        (
            "SubscribeCOV, no object",
            SUBSCRIBE,
            subscribe(1, gone, Some(false), Some(60)),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            "SubscribeCOVProperty, no object",
            PROPERTY,
            subscribe_property(1, gone, P::PRESENT_VALUE, None, Some(false), Some(60)),
            (ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT),
        ),
        (
            "SubscribeCOV, no COV reporting",
            SUBSCRIBE,
            subscribe(1, group, Some(false), Some(60)),
            (
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ),
        ),
        (
            "SubscribeCOV, property subscriptions only",
            SUBSCRIBE,
            subscribe(1, averaging(), Some(false), Some(60)),
            (
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ),
        ),
        (
            "SubscribeCOVProperty, no COV reporting",
            PROPERTY,
            subscribe_property(1, group, P::PRESENT_VALUE, None, Some(false), Some(60)),
            (
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ),
        ),
        (
            "property absent",
            PROPERTY,
            subscribe_property(1, ai(), P::PRIORITY_ARRAY, None, Some(false), Some(60)),
            (ErrorClass::PROPERTY, ErrorCode::UNKNOWN_PROPERTY),
        ),
        (
            "property not reportable",
            PROPERTY,
            subscribe_property(
                1,
                oid(ObjectType::LIFE_SAFETY_POINT, 1),
                P::MODE,
                None,
                Some(false),
                Some(60),
            ),
            (ErrorClass::PROPERTY, ErrorCode::NOT_COV_PROPERTY),
        ),
        (
            "an index on a scalar",
            PROPERTY,
            subscribe_property(1, ai(), P::PRESENT_VALUE, Some(1), Some(false), Some(60)),
            (ErrorClass::PROPERTY, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY),
        ),
        (
            "SubscribeCOVProperty, zero lifetime",
            PROPERTY,
            subscribe_property(1, ai(), P::PRESENT_VALUE, None, Some(false), Some(0)),
            (ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE),
        ),
    ] {
        let reply = cov.request(what, &a, service, &body).await;
        assert_eq!(refused(&reply), expected, "{what}");
    }
    for (what, service, body, reason) in [
        (
            "SubscribeCOV, a lifetime without the confirmed flag",
            SUBSCRIBE,
            subscribe(1, ai(), None, Some(60)),
            Some(RejectReason::INCONSISTENT_PARAMETERS),
        ),
        (
            "SubscribeCOVProperty, a lifetime without the confirmed flag",
            PROPERTY,
            subscribe_property(1, ai(), P::PRESENT_VALUE, None, None, Some(60)),
            Some(RejectReason::INCONSISTENT_PARAMETERS),
        ),
        // The decoder's reason, whatever it is, on both sides.
        (
            "SubscribeCOV, parameters that don't decode",
            SUBSCRIBE,
            vec![0x09],
            None,
        ),
        (
            "SubscribeCOVProperty, parameters that don't decode",
            PROPERTY,
            vec![0x09, 0x01, 0x1C],
            None,
        ),
    ] {
        let reply = cov.request(what, &a, service, &body).await;
        let Apdu::Reject(reject) = reply else {
            panic!("{what}: expected a Reject, got {reply:?}");
        };
        if let Some(reason) = reason {
            assert_eq!(reject.reject_reason, reason, "{what}");
        }
    }
    cov.stop().await;
}

/// The caps, and indefinite subscriptions refused by policy.
#[tokio::test(start_paused = true)]
async fn caps_match() {
    let policy = CovPolicy {
        max_subscriptions_global: 3,
        max_subscriptions_per_peer: 2,
        max_indefinite_per_peer: 1,
        reserved_capacity: 0,
        ..CovPolicy::default()
    };
    let mut cov = CovParity::start(policy).await;
    let (a, b, c) = (direct(1), direct(2), direct(3));
    let no_space = (
        ErrorClass::RESOURCES,
        ErrorCode::NO_SPACE_TO_ADD_LIST_ELEMENT,
    );
    cov.request(
        "first",
        &a,
        SUBSCRIBE,
        &subscribe(1, ai(), Some(false), Some(0)),
    )
    .await;
    let reply = cov
        .request(
            "second indefinite",
            &a,
            SUBSCRIBE,
            &subscribe(2, ai(), Some(false), Some(0)),
        )
        .await;
    assert_eq!(refused(&reply), no_space);
    cov.request(
        "second",
        &a,
        SUBSCRIBE,
        &subscribe(2, ai(), Some(false), Some(60)),
    )
    .await;
    let reply = cov
        .request(
            "past the peer's quota",
            &a,
            SUBSCRIBE,
            &subscribe(3, ai(), Some(false), Some(60)),
        )
        .await;
    assert_eq!(refused(&reply), no_space);
    cov.request(
        "a renewal adds none",
        &a,
        SUBSCRIBE,
        &subscribe(2, ai(), Some(true), Some(90)),
    )
    .await;
    cov.request(
        "another peer",
        &b,
        SUBSCRIBE,
        &subscribe(1, ai(), Some(false), Some(60)),
    )
    .await;
    let reply = cov
        .request(
            "past the table",
            &c,
            SUBSCRIBE,
            &subscribe(1, ai(), Some(false), Some(60)),
        )
        .await;
    assert_eq!(refused(&reply), no_space);
    cov.stop().await;

    let policy = CovPolicy {
        allow_indefinite_subscriptions: false,
        ..CovPolicy::default()
    };
    let mut cov = CovParity::start(policy).await;
    let reply = cov
        .request(
            "indefinite refused",
            &a,
            SUBSCRIBE,
            &subscribe(1, ai(), Some(false), Some(0)),
        )
        .await;
    assert_eq!(
        refused(&reply),
        (ErrorClass::SERVICES, ErrorCode::VALUE_OUT_OF_RANGE)
    );
    cov.request(
        "finite",
        &a,
        SUBSCRIBE,
        &subscribe(1, ai(), Some(false), Some(60)),
    )
    .await;
    cov.stop().await;

    // Two of three slots set aside for peer 9: once the one unreserved slot
    // is taken, only peer 9 is admitted.
    let reserved = direct(9);
    let policy = CovPolicy {
        max_subscriptions_global: 3,
        reserved_capacity: 2,
        reserved_peers: vec![reserved.mac.clone()],
        ..CovPolicy::default()
    };
    let mut cov = CovParity::start(policy).await;
    let finite = |process| subscribe(process, ai(), Some(false), Some(60));
    cov.request("unreserved", &a, SUBSCRIBE, &finite(1)).await;
    let reply = cov
        .request("past the unreserved share", &b, SUBSCRIBE, &finite(1))
        .await;
    assert_eq!(refused(&reply), no_space);
    cov.request("reserved", &reserved, SUBSCRIBE, &finite(1))
        .await;
    cov.request("reserved again", &reserved, SUBSCRIBE, &finite(2))
        .await;
    let reply = cov
        .request("past the table", &reserved, SUBSCRIBE, &finite(3))
        .await;
    assert_eq!(refused(&reply), no_space);
    cov.stop().await;
}

/// A routed client renewing through another router: one subscription,
/// whose route moves, on both sides.
#[tokio::test(start_paused = true)]
async fn a_renewal_through_another_router_moves_the_route() {
    let mut cov = CovParity::start(CovPolicy::default()).await;
    let body = subscribe(1, ai(), Some(true), Some(600));
    cov.request("through router 1", &routed(1), SUBSCRIBE, &body)
        .await;
    cov.request("through router 2", &routed(2), SUBSCRIBE, &body)
        .await;
    let body = subscribe_property(1, ai(), P::PRESENT_VALUE, None, Some(false), Some(600));
    cov.request("a property, router 1", &routed(1), PROPERTY, &body)
        .await;
    cov.request("a property, router 2", &routed(2), PROPERTY, &body)
        .await;
    let table = cov.parity.reporter.table();
    assert_eq!(table.len(), 2);
    assert!(table.entries().all(|entry| entry.endpoint() == routed(2)));
    cov.stop().await;
}
