//! Source ownership is configuration only: no traffic, records or worker lifecycle.

use super::*;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::traits::BACnetObject;
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::ReceivedNpdu;
use bacnet_types::enums::{AuditLevel, PropertyIdentifier};
use bacnet_types::primitives::PropertyValue;
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

#[derive(Default)]
struct Observations {
    starts: AtomicUsize,
    stops: AtomicUsize,
    sends: AtomicUsize,
}

struct ObservedTransport {
    inner: LoopbackTransport,
    observed: Arc<Observations>,
}

impl TransportPort for ObservedTransport {
    async fn start(&mut self) -> Result<mpsc::Receiver<ReceivedNpdu>, Error> {
        self.observed.starts.fetch_add(1, Ordering::SeqCst);
        self.inner.start().await
    }
    async fn stop(&mut self) -> Result<(), Error> {
        self.observed.stops.fetch_add(1, Ordering::SeqCst);
        self.inner.stop().await
    }
    async fn send_unicast(&self, npdu: &[u8], mac: &[u8]) -> Result<(), Error> {
        self.observed.sends.fetch_add(1, Ordering::SeqCst);
        self.inner.send_unicast(npdu, mac).await
    }
    async fn send_broadcast(&self, npdu: &[u8]) -> Result<(), Error> {
        self.observed.sends.fetch_add(1, Ordering::SeqCst);
        self.inner.send_broadcast(npdu).await
    }
    fn local_mac(&self) -> &[u8] {
        self.inner.local_mac()
    }
}

fn oid(kind: ObjectType, instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, instance).unwrap()
}

fn selected() -> ObjectIdentifier {
    oid(ObjectType::AUDIT_REPORTER, 1)
}
fn target() -> ObjectIdentifier {
    oid(ObjectType::AUDIT_REPORTER, 2)
}

fn database() -> ObjectDatabase {
    let mut db = crate::identity::DeviceIdentity::new(123, 42)
        .unwrap()
        .build_database()
        .unwrap();
    for instance in [1, 2] {
        let mut reporter =
            AuditReporterObject::new(instance, format!("Reporter-{instance}")).unwrap();
        // Silence must not depend on the default disabled audit level.
        reporter.set_audit_level(AuditLevel::AUDIT_ALL).unwrap();
        reporter.set_auditable_operations(
            bacnet_types::bitstring::AuditOperationFlags::from_bits(0xff).unwrap(),
        );
        reporter.set_issue_confirmed_notifications(true);
        db.add(Box::new(reporter)).unwrap();
    }
    db
}

fn session(
    role: SessionRole,
) -> (
    EndpointSession<ObservedTransport>,
    LoopbackTransport,
    Arc<Observations>,
) {
    let (inner, peer) = LoopbackTransport::pair(vec![1], vec![2]);
    let observed = Arc::new(Observations::default());
    let session = EndpointSession::new(
        ObservedTransport {
            inner,
            observed: observed.clone(),
        },
        role,
        SessionConfig::default(),
    )
    .unwrap();
    (session, peer, observed)
}

fn source(db: &ObjectDatabase, oid: ObjectIdentifier) -> bool {
    match db
        .get(&oid)
        .unwrap()
        .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
        .unwrap()
    {
        PropertyValue::Boolean(source) => source,
        value => panic!("invalid source property: {value:?}"),
    }
}

#[path = "source_reporter_forwarding_tests.rs"]
mod forwarding;

#[path = "source_audit_recipient_tests.rs"]
mod recipient;

async fn success(role: SessionRole) {
    let (session, peer, observed) = session(role);
    let mut session = session
        .with_database(database())
        .with_identity(crate::identity::DeviceIdentity::new(123, 42).unwrap())
        .with_source_audit_reporter(selected());
    assert!(!source(
        &*session.database.as_ref().unwrap().read().await,
        selected()
    ));
    assert_eq!(observed.starts.load(Ordering::SeqCst), 0);
    assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
    session.start().await.unwrap();
    assert!(session.is_running());
    assert!(session.client().is_some());
    assert_eq!(session.server().is_some(), role == SessionRole::Both);
    let db = session.database.as_ref().unwrap();
    let weak = Arc::downgrade(db);
    assert_eq!(
        Arc::strong_count(db),
        if role == SessionRole::Both { 2 } else { 1 }
    );
    {
        let db = db.read().await;
        assert!(source(&db, selected()));
        assert!(!db.get(&selected()).unwrap().is_deleteable());
        assert!(!db.get(&selected()).unwrap().is_createable());
        assert!(!source(&db, target()));
        assert!(db.get(&target()).unwrap().is_deleteable());
    }
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
    assert_eq!(session.active_leases(), 0);
    assert!(session.start().await.is_err());
    assert_eq!(observed.starts.load(Ordering::SeqCst), 1);
    if role == SessionRole::Both {
        // The responder sees the same promoted object, not a second database.
        let mut peer =
            EndpointSession::new(peer, SessionRole::ClientOnly, SessionConfig::default()).unwrap();
        peer.start().await.unwrap();
        let ack = peer
            .client()
            .unwrap()
            .read_property(
                &[1],
                selected(),
                PropertyIdentifier::AUDIT_SOURCE_REPORTER,
                None,
            )
            .await
            .unwrap();
        assert_eq!(ack.property_value.as_slice(), &[0x11]); // BACnet application Boolean TRUE
        peer.stop().await.unwrap();
    }
    session.stop().await.unwrap();
    assert_eq!(observed.stops.load(Ordering::SeqCst), 1);
    assert!(!session.is_running());
    assert!(source(
        &*session.database.as_ref().unwrap().read().await,
        selected()
    ));
    drop(session);
    assert!(weak.upgrade().is_none());
}

#[tokio::test(start_paused = true)]
async fn client_only_retains_source_database_and_startup_is_silent() {
    success(SessionRole::ClientOnly).await;
}

#[tokio::test(start_paused = true)]
async fn both_shares_source_database_with_responder_and_startup_is_silent() {
    success(SessionRole::Both).await;
}

async fn rejected(
    session: &mut EndpointSession<ObservedTransport>,
    observed: &Observations,
    expected: &str,
) {
    let before = if let Some(db) = &session.database {
        let db = db.read().await;
        Some(
            db.iter_objects()
                .filter(|(oid, _)| oid.object_type() == ObjectType::AUDIT_REPORTER)
                .map(|(oid, object)| {
                    (
                        oid,
                        object
                            .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
                            .ok(),
                        object.is_deleteable(),
                        std::ptr::from_ref(object).cast::<()>(),
                    )
                })
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    for _ in 0..2 {
        assert!(
            matches!(session.start().await, Err(Error::Encoding(message)) if message.contains(expected))
        );
        assert_eq!(
            session.lifecycle.load(Ordering::Acquire),
            Lifecycle::Ready as u8
        );
        assert!(!session.is_running());
        assert!(session.requester.is_none() && session.responder.is_none());
        assert!(session.dispatch_task.is_none() && session.egress.is_none());
        assert_eq!(observed.starts.load(Ordering::SeqCst), 0);
        assert_eq!(observed.sends.load(Ordering::SeqCst), 0);
        if let Some(before) = &before {
            let db = session.database.as_ref().unwrap().read().await;
            for (oid, value, deleteable, original) in before {
                let object = db.get(oid).unwrap();
                assert_eq!(
                    *original,
                    std::ptr::from_ref(object).cast::<()>(),
                    "validation replaced an object"
                );
                assert_eq!(
                    &object
                        .read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None)
                        .ok(),
                    value
                );
                assert_eq!(object.is_deleteable(), *deleteable);
            }
        }
    }
}

#[tokio::test]
async fn server_only_rejects_source_without_consuming_start() {
    let (session, _peer, observed) = session(SessionRole::ServerOnly);
    let mut session = session
        .with_database(database())
        .with_source_audit_reporter(selected());
    rejected(&mut session, &observed, "requires a client role").await;
}

#[tokio::test]
async fn missing_database_can_be_attached_after_validation_failure() {
    let (session, _peer, observed) = session(SessionRole::ClientOnly);
    let mut session = session.with_source_audit_reporter(selected());
    rejected(
        &mut session,
        &observed,
        "requires an attached local database",
    )
    .await;
    assert!(session.database.is_none());
    session = session.with_database(database());
    session.start().await.unwrap();
    session.stop().await.unwrap();
    assert_eq!(observed.starts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn device_validation_is_atomic_and_correctable() {
    for case in ["missing", "multiple", "mismatch"] {
        let (session, _peer, observed) = session(SessionRole::Both);
        let mut db = database();
        if case == "missing" {
            db.remove(&oid(ObjectType::DEVICE, 123));
        }
        if case == "multiple" {
            let mut other = crate::identity::DeviceIdentity::new(456, 42)
                .unwrap()
                .build_database()
                .unwrap();
            let device = other.remove(&oid(ObjectType::DEVICE, 456)).unwrap();
            // build_database uses the instance in the default Device name.
            db.add(device).unwrap();
        }
        let identity =
            crate::identity::DeviceIdentity::new(if case == "mismatch" { 456 } else { 123 }, 42)
                .unwrap();
        let mut session = session
            .with_database(db)
            .with_identity(identity)
            .with_source_audit_reporter(selected());
        rejected(
            &mut session,
            &observed,
            if case == "mismatch" {
                "does not match session identity"
            } else {
                "requires exactly one local Device"
            },
        )
        .await;
        session = session
            .with_database(database())
            .with_identity(crate::identity::DeviceIdentity::new(123, 42).unwrap());
        session.start().await.unwrap();
        session.stop().await.unwrap();
    }
}

// Reporter-shaped downstream object without the opt-in Reporter capability.
struct ForeignReporter {
    oid: ObjectIdentifier,
    source: Option<bool>,
    capability: Option<AuditReporterObject>,
}

impl BACnetObject for ForeignReporter {
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        self.capability.as_ref()
    }
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }
    fn object_name(&self) -> &str {
        self.capability
            .as_ref()
            .map_or("Foreign Reporter", |reporter| reporter.object_name())
    }
    fn read_property(&self, p: PropertyIdentifier, _: Option<u32>) -> Result<PropertyValue, Error> {
        if p == PropertyIdentifier::AUDIT_SOURCE_REPORTER {
            if let Some(source) = self.source {
                return Ok(PropertyValue::Boolean(source));
            }
        }
        Err(Error::Encoding("unsupported property".into()))
    }
    fn write_property(
        &mut self,
        _: PropertyIdentifier,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Encoding("not writable".into()))
    }
    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Borrowed(&[])
    }
}

#[tokio::test]
async fn selection_validation_is_atomic_and_correctable() {
    for case in [
        "absent",
        "wrong type",
        "non capable",
        "mismatched capability",
    ] {
        let (session, _peer, observed) = session(SessionRole::Both);
        let mut db = database();
        if matches!(case, "non capable" | "mismatched capability") {
            db.add(Box::new(ForeignReporter {
                oid: selected(),
                source: Some(false),
                capability: (case == "mismatched capability")
                    .then(|| AuditReporterObject::new(3, "Wrong capability identity").unwrap()),
            }))
            .unwrap();
        }
        let selection = match case {
            "absent" => oid(ObjectType::AUDIT_REPORTER, 99),
            "wrong type" => oid(ObjectType::DEVICE, 123),
            _ => selected(),
        };
        let expected = match case {
            "absent" => "absent from the database",
            "wrong type" => "must be an Audit Reporter",
            _ => "lacks the Audit Reporter capability",
        };
        let mut session = session
            .with_database(db)
            .with_source_audit_reporter(selection);
        rejected(&mut session, &observed, expected).await;
        session = session
            .with_database(database())
            .with_source_audit_reporter(selected());
        session.start().await.unwrap();
        session.stop().await.unwrap();
    }
}

#[tokio::test]
async fn conflicting_or_unknown_foreign_source_is_rejected_before_mutation() {
    for state in [Some(true), None] {
        let (session, _peer, observed) = session(SessionRole::ClientOnly);
        let mut db = database();
        db.add(Box::new(ForeignReporter {
            oid: target(),
            source: state,
            capability: None,
        }))
        .unwrap();
        let mut session = session
            .with_database(db)
            .with_source_audit_reporter(selected());
        rejected(
            &mut session,
            &observed,
            if state.is_some() {
                "conflicting source"
            } else {
                "cannot determine"
            },
        )
        .await;
    }
}

#[tokio::test]
async fn existing_source_is_idempotent_but_conflicting_selection_is_not() {
    let (session, _peer, observed) = session(SessionRole::Both);
    let mut db = database();
    db.add(Box::new(ForeignReporter {
        oid: target(),
        source: Some(true),
        capability: Some(AuditReporterObject::new(2, "Existing source").unwrap()),
    }))
    .unwrap();
    let mut session = session
        .with_database(db)
        .with_source_audit_reporter(selected());
    rejected(&mut session, &observed, "conflicting source").await;
    session = session.with_source_audit_reporter(target());
    session.start().await.unwrap();
    {
        let db = session.database.as_ref().unwrap().read().await;
        assert!(source(&db, target()));
        assert!(!db.get(&target()).unwrap().is_deleteable());
    }
    session.stop().await.unwrap();
}

#[tokio::test]
async fn source_database_is_released_on_drop_without_stop() {
    for role in [SessionRole::ClientOnly, SessionRole::Both] {
        let (session, _peer, _) = session(role);
        let mut session = session
            .with_database(database())
            .with_source_audit_reporter(selected());
        session.start().await.unwrap();
        let weak = Arc::downgrade(session.database.as_ref().unwrap());
        drop(session);
        // Dispatch owns a responder clone until its abort is observed.
        tokio::task::yield_now().await;
        assert!(weak.upgrade().is_none());
    }
}

#[tokio::test]
async fn duplicate_sources_assembled_locally_are_rejected_atomically() {
    let (session, _peer, observed) = session(SessionRole::Both);
    let mut first = database();
    // Downstream objects may already claim source roles; no built-in promotion
    // primitive is needed (or available) to exercise duplicate validation.
    for instance in [1, 2] {
        first
            .add(Box::new(ForeignReporter {
                oid: oid(ObjectType::AUDIT_REPORTER, instance),
                source: Some(true),
                capability: Some(
                    AuditReporterObject::new(instance, format!("Source-{instance}")).unwrap(),
                ),
            }))
            .unwrap();
    }
    let mut session = session
        .with_database(first)
        .with_source_audit_reporter(selected());
    rejected(&mut session, &observed, "conflicting source").await;
}

#[tokio::test]
#[should_panic(expected = "endpoint configuration must precede startup")]
async fn source_selection_cannot_change_after_start() {
    let (session, _peer, _) = session(SessionRole::ClientOnly);
    let mut session = session
        .with_database(database())
        .with_source_audit_reporter(selected());
    session.start().await.unwrap();
    let _ = session.with_source_audit_reporter(target());
}

#[tokio::test]
async fn repeated_pre_start_selection_only_wraps_the_final_choice() {
    let (session, _peer, _) = session(SessionRole::ClientOnly);
    let mut session = session
        .with_database(database())
        .with_source_audit_reporter(selected())
        .with_source_audit_reporter(target());
    session.start().await.unwrap();
    {
        let db = session.database.as_ref().unwrap().read().await;
        assert!(!source(&db, selected()));
        assert!(db.get(&selected()).unwrap().is_deleteable());
        assert!(source(&db, target()));
        assert!(!db.get(&target()).unwrap().is_deleteable());
    }
    session.stop().await.unwrap();
}
