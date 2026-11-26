//! The rest of a refused start's paths (#2752): target Audit's installation
//! refused once the link is up, a link that also fails to stop, and a
//! refusal dropped without taking its database, which goes off the runtime.
use std::borrow::Cow;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc as std_mpsc;
use std::thread::ThreadId;

use super::*;
use bacnet_objects::audit::AuditReporterAuthority;
use bacnet_objects::property_metadata::PropertyMetadata;
use bacnet_objects::traits::BACnetObject;

/// An Audit Reporter that lends its authority to the start's two route
/// checks and refuses it to the installation that follows them, once.
struct RefusesInstall {
    inner: AuditReporterObject,
    borrows: Arc<AtomicUsize>,
}

impl BACnetObject for RefusesInstall {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(
        &self,
        property: PropertyIdentifier,
        index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        self.inner.read_property(property, index)
    }
    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        self.inner.write_property(property, index, value, priority)
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        self.inner.property_metadata()
    }
    fn audit_reporter(&self) -> Option<&AuditReporterObject> {
        Some(&self.inner)
    }
    fn audit_reporter_mut(&mut self) -> Option<AuditReporterAuthority<'_>> {
        if self.borrows.fetch_add(1, Ordering::AcqRel) == 2 {
            return None;
        }
        self.inner.audit_reporter_mut()
    }
}

/// The Device with an Audit recipient, a [`RefusesInstall`] Reporter
/// counting into `borrows`, and a value.
fn refusing_audit(borrows: &Arc<AtomicUsize>) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    let mut device = device();
    let logger = oid(ObjectType::DEVICE, 20);
    device
        .provision_audit_recipient(BACnetRecipient::Device(logger))
        .unwrap();
    db.add(Box::new(device)).unwrap();
    db.add(Box::new(RefusesInstall {
        inner: AuditReporterObject::new(1, "reporter").unwrap(),
        borrows: Arc::clone(borrows),
    }))
    .unwrap();
    db.add(Box::new(BinaryValueObject::new(1, "BV-1").unwrap()))
        .unwrap();
    db
}

fn audited() -> ServerConfig {
    ServerConfig {
        audit_reporters: Some(AuditReportersConfig {
            reporters: vec![oid(ObjectType::AUDIT_REPORTER, 1)],
        }),
        ..config()
    }
}

fn denied(error: &Error) -> bool {
    error.remote()
        == Some(RemoteError::Error {
            class: ErrorClass::SERVICES,
            code: ErrorCode::SERVICE_REQUEST_DENIED,
        })
}

/// Target Audit refused as it installs, after the link started: the link
/// is let go of and the database comes back, which the next start serves.
#[tokio::test(start_paused = true)]
async fn an_audit_installation_refused_after_the_link_started_hands_the_database_back() {
    let borrows = Arc::new(AtomicUsize::new(0));
    let db = refusing_audit(&borrows);
    let objects: Vec<_> = db.object_list().collect();
    let link = TestTransport::new();
    let handle = link.handle();
    let (error, db) = refused(BACnetServer::start(audited(), db, link).await, &objects);
    assert_eq!(borrows.load(Ordering::Acquire), 3, "refused at the third");
    assert!(denied(&error), "{error}");
    assert_eq!(handle.stops(), 1, "the started link is let go of");
    let mut server = BACnetServer::start(audited(), db, TestTransport::new())
        .await
        .unwrap();
    serves(&server, &objects).await;
    server.stop().await.unwrap();
}

/// A refused start whose link then fails to stop keeps its own error, and
/// hands the database back: a port in use, and target Audit refused as it
/// installs. The failed stop is only logged.
#[tokio::test(start_paused = true)]
async fn a_refusal_whose_link_fails_to_stop_keeps_its_own_error() {
    let stuck = || async {
        Err(Error::Transport(std::io::Error::other(
            "the link would not stop",
        )))
    };
    let in_use_link = TestTransport::builder()
        .start_gate(|| async { Err(Error::Transport(std::io::ErrorKind::AddrInUse.into())) })
        .on_stop(stuck)
        .build();
    let handle = in_use_link.handle();
    let objects: Vec<_> = database().object_list().collect();
    let (error, _) = refused(
        BACnetServer::start(config(), database(), in_use_link).await,
        &objects,
    );
    assert!(in_use(&error), "{error}");
    assert_eq!(handle.stops(), 1, "the link was told to stop");

    let borrows = Arc::new(AtomicUsize::new(0));
    let db = refusing_audit(&borrows);
    let objects: Vec<_> = db.object_list().collect();
    let link = TestTransport::builder().on_stop(stuck).build();
    let handle = link.handle();
    let (error, _) = refused(BACnetServer::start(audited(), db, link).await, &objects);
    assert!(denied(&error), "{error}");
    assert_eq!(handle.stops(), 1, "the link was told to stop");
}

/// An object that says which thread drops it.
struct DropWitness {
    dropped: std_mpsc::Sender<ThreadId>,
}

impl Drop for DropWitness {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}

impl BACnetObject for DropWitness {
    fn object_identifier(&self) -> ObjectIdentifier {
        oid(ObjectType::BINARY_VALUE, 9)
    }
    fn object_name(&self) -> &str {
        "Witness"
    }
    fn read_property(
        &self,
        _property: PropertyIdentifier,
        _index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::UNKNOWN_PROPERTY,
        ))
    }
    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::protocol(
            ErrorClass::PROPERTY,
            ErrorCode::WRITE_ACCESS_DENIED,
        ))
    }
    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        crate::test_rows::read_only(&[])
    }
}

/// A refusal dropped whole, or turned into [`Error`], lets go of its
/// database off the runtime: the objects never drop on the runtime's thread,
/// the test's own on this current-thread runtime. Objects with storage wait
/// for their saves as they drop.
#[tokio::test]
async fn a_refusal_lets_go_of_its_database_off_the_runtime() {
    for converted in [false, true] {
        let (dropped, dropped_on) = std_mpsc::channel();
        let mut db = database();
        db.add(Box::new(DropWitness { dropped })).unwrap();
        let Err(refused) = BACnetServer::start(config(), db, port_in_use()).await else {
            panic!("the start should be refused");
        };
        if converted {
            drop(Error::from(refused));
        } else {
            drop(refused);
        }
        let thread = tokio::task::spawn_blocking(move || {
            dropped_on.recv_timeout(std::time::Duration::from_secs(5))
        })
        .await
        .unwrap()
        .expect("the database was dropped");
        assert_ne!(
            thread,
            std::thread::current().id(),
            "converted into Error: {converted}"
        );
    }
}

#[path = "start_phase_paths_tests.rs"]
mod phase_paths;
