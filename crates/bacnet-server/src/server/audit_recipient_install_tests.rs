//! `TargetAudit::install` refuses a database that no longer holds what
//! validation checked, with an error rather than a panic (#2559). Startup
//! validates first, so only a direct call reaches these refusals.
use super::*;
use crate::server::test_transport::TestTransport;
use bacnet_objects::audit::AuditReporterObject;
use bacnet_objects::device::{DeviceConfig, DeviceObject};

fn identity() -> bacnet_objects::DeviceIdentity {
    bacnet_objects::DeviceIdentity::new(10, 0).unwrap()
}

fn reporter_oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AUDIT_REPORTER, 1).unwrap()
}

/// A database with the identity's Device, its recipient provisioned when
/// `recipient`, and an Audit Reporter when `reporter`.
fn database(device: bool, recipient: bool, reporter: bool) -> ObjectDatabase {
    let mut db = ObjectDatabase::new();
    if device {
        let config = DeviceConfig {
            identity: identity(),
            ..Default::default()
        };
        db.add(Box::new(DeviceObject::new(config).unwrap()))
            .unwrap();
    }
    if recipient {
        let logger = ObjectIdentifier::new(ObjectType::DEVICE, 20).unwrap();
        db.get_mut(&ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap())
            .unwrap()
            .device_authority_internal()
            .unwrap()
            .provision_audit_recipient(BACnetRecipient::Device(logger))
            .unwrap();
    }
    if reporter {
        db.add(Box::new(AuditReporterObject::new(1, "reporter").unwrap()))
            .unwrap();
    }
    db
}

/// The text of `install`'s refusal over `db`, after checking it left `db`
/// and the notification transactions as they were: the Device's recipient,
/// the Reporter's configuration epoch, and no Audit routes or batch queue.
fn refusal(mut db: ObjectDatabase) -> String {
    let config = ServerConfig {
        device: identity(),
        audit_reporters: Some(AuditReportersConfig {
            reporters: vec![reporter_oid()],
        }),
        ..Default::default()
    };
    let network = Arc::new(NetworkLayer::new(TestTransport::builder().build()));
    let transactions = NotificationTransactions::new();
    let before = observed(&mut db);
    let installed = TargetAudit::install(
        &mut db,
        &config,
        Arc::new(super::super::audit_recipient_routes::AuditRoutes::default()),
        &network,
        &transactions,
    );
    let text = match installed {
        Err(Error::InvalidConfig(text)) => text.to_string(),
        Err(other) => panic!("unexpected refusal {other:?}"),
        Ok(_) => panic!("install accepted an incomplete database"),
    };
    assert_eq!(observed(&mut db), before, "a refusal changed the database");
    assert!(transactions.audit_routes.get().is_none());
    assert!(transactions.audit_batch.get().is_none());
    text
}

/// The Device's provisioned recipient and the Reporter's configuration
/// epoch, where `db` has them.
fn observed(db: &mut ObjectDatabase) -> (Option<BACnetRecipient>, Option<u64>) {
    let recipient = db
        .get_mut(&ObjectIdentifier::new(ObjectType::DEVICE, 10).unwrap())
        .and_then(|device| device.device_authority_internal())
        .and_then(|authority| authority.provisioned_audit_recipient().cloned());
    let epoch = db
        .get(&reporter_oid())
        .and_then(|object| object.audit_reporter())
        .map(|reporter| reporter.status_internal().configuration_epoch());
    (recipient, epoch)
}

#[tokio::test]
async fn a_selected_reporter_missing_from_the_database_is_refused() {
    let text = refusal(database(true, true, false));
    assert!(text.contains("missing read capability"), "{text}");
}

#[tokio::test]
async fn a_missing_device_is_refused() {
    let text = refusal(database(false, false, true));
    assert!(text.contains("the identity's built-in Device"), "{text}");
}

#[tokio::test]
async fn a_device_without_a_provisioned_recipient_is_refused() {
    let text = refusal(database(true, false, true));
    assert!(
        text.contains("provisioned built-in Device recipient"),
        "{text}"
    );
}
