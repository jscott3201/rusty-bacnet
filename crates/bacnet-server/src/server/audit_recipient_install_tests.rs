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

/// The text of `install`'s refusal over `db`.
fn refusal(mut db: ObjectDatabase) -> String {
    let config = ServerConfig {
        device: identity(),
        audit_reporters: Some(AuditReportersConfig {
            reporters: vec![reporter_oid()],
        }),
        ..Default::default()
    };
    let network = Arc::new(NetworkLayer::new(TestTransport::builder().build()));
    let installed = TargetAudit::install(
        &mut db,
        &config,
        Arc::new(super::super::audit_recipient_routes::AuditRoutes::default()),
        &network,
        &NotificationTransactions::new(),
    );
    match installed {
        Err(Error::Encoding(text)) => text,
        Err(other) => panic!("unexpected refusal {other:?}"),
        Ok(_) => panic!("install accepted an incomplete database"),
    }
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
