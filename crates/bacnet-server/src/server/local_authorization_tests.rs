//! The terminal decision must reach the point and its ordinary wire COV report.

use super::*;
use bacnet_objects::access_control::*;
use bacnet_types::constructed::{BACnetAccessRule, BACnetAssignedAccessRights};
use bacnet_types::enums::AccessCredentialDisableReason as Reason;
use std::time::Duration;

fn identifier(kind: ObjectType) -> ObjectIdentifier {
    ObjectIdentifier::new(kind, 1).unwrap()
}

/// Explicit test policy; these choices are not installed server defaults.
fn request(lockout: bool) -> AuthorizationRequest<'static> {
    use AuthorizationCheck::*;
    AuthorizationRequest {
        original: CredentialPresentation {
            sequence: 10,
            credential: identifier(ObjectType::ACCESS_CREDENTIAL),
            point: identifier(ObjectType::ACCESS_POINT),
            at: Duration::from_secs(100),
        },
        wall_time: None,
        now: Duration::from_secs(101),
        order: AuthorizationOrder::new([
            Credential,
            AccessRights,
            Threat,
            Occupancy,
            Passback,
            Lockout,
            Mode,
        ])
        .unwrap(),
        disable_reason_priority: DisableReasonPriority::new([
            Reason::DISABLED,
            Reason::DISABLED_NEEDS_PROVISIONING,
            Reason::DISABLED_UNASSIGNED,
            Reason::DISABLED_NOT_YET_ACTIVE,
            Reason::DISABLED_EXPIRED,
            Reason::DISABLED_LOCKOUT,
            Reason::DISABLED_MAX_DAYS,
            Reason::DISABLED_MAX_USES,
            Reason::DISABLED_INACTIVITY,
            Reason::DISABLED_MANUAL,
        ])
        .unwrap(),
        rights_policy: AuthorizationRightsPolicy::HonorExistingDecision,
        fallback_threat_level: AuthorizationFact::Known(0),
        fallback_lockout: AuthorizationFact::Known(lockout),
        occupancy: AuthorizationOccupancy::NotApplicable,
        passback: AuthorizationPassback::NotApplicable,
        accompaniment_witnesses: &[],
        accompaniment_timing: AccompanimentTiming {
            order: AccompanimentOrder::After,
            fallback_period: Duration::from_secs(10),
            deadline: AccompanimentDeadline::Inclusive,
        },
        max_accompaniment_links: 16,
        companion_authentication: &[],
        external_phase: AuthorizationExternalPhase::NotSupplied,
    }
}

fn native_objects(db: &mut ObjectDatabase) {
    db.add(point(AccessEvent::GRANTED, 1, 7)).unwrap();
    let mut rights = AccessRightsObject::new(1, "local rights").unwrap();
    rights
        .set_positive_access_rules([BACnetAccessRule::new(
            None,
            Some(identifier(ObjectType::ACCESS_POINT).into()),
            true,
        )])
        .unwrap();
    db.add(Box::new(rights)).unwrap();
    let mut credential = AccessCredentialObject::new(1, "local credential").unwrap();
    credential
        .set_assigned_access_rights(vec![BACnetAssignedAccessRights {
            assigned_access_rights: identifier(ObjectType::ACCESS_RIGHTS).into(),
            enable: true,
        }])
        .unwrap();
    db.add(Box::new(credential)).unwrap();
}

async fn terminal_event(lockout: bool, event: AccessEvent) {
    let mut h = Harness::start_with(ServerConfig::default(), native_objects).await;
    let request = request(lockout);
    let oid = request.original.point;
    subscribed(&mut h, oid).await;
    h.set_clock(20);
    let expected = {
        let db = h.server.database().read().await;
        evaluate_authorization(&db, request).unwrap()
    };
    // This independent outcome assertion makes a broken setup fail before the
    // route. In the denial case, successful rights must not become an early grant.
    assert!(matches!(
        expected.rights.as_ref().unwrap().decision,
        AccessRightsDecision::Granted { .. }
    ));
    if lockout {
        assert_eq!(
            expected.outcome,
            AuthorizationOutcome::Denied {
                stage: AuthorizationStage::Criterion(AuthorizationCheck::Lockout),
                access_event: AccessEvent::DENIED_LOCKOUT,
            }
        );
    } else {
        assert_eq!(expected.outcome, AuthorizationOutcome::Granted);
    }
    assert_eq!(expected.events.len(), 1);
    assert_eq!(expected.events[0].access_event, event);

    let result = h.server.authorize_access_local(request, 42).await.unwrap();
    assert_eq!(result.evaluation, expected);
    assert_eq!(
        result.committed_events,
        vec![AccessEventReport {
            event,
            tag: 42,
            time: Some(stamp(20)),
            credential: Some(request.original.credential.into()),
            authentication_factor: None,
        }]
    );
    let wanted = vec![
        (PropertyIdentifier::ACCESS_EVENT, enumerated(event.to_raw())),
        (SF, normal()),
        (PropertyIdentifier::ACCESS_EVENT_TAG, unsigned(42)),
        (PropertyIdentifier::ACCESS_EVENT_TIME, stamp_bytes(20)),
        (
            PropertyIdentifier::ACCESS_EVENT_CREDENTIAL,
            credential_bytes(1),
        ),
        (
            PropertyIdentifier::ACCESS_EVENT_AUTHENTICATION_FACTOR,
            undefined_factor_bytes(),
        ),
    ];
    assert_eq!(values(&h.cov_notification().await, oid), wanted);
    {
        let db = h.server.database().read().await;
        let point = db.get(&oid).unwrap();
        for (property, value) in wanted {
            assert_eq!(encode(point.read_property(property, None).unwrap()), value);
        }
    }
    h.no_notification().await;
    h.server.stop().await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn local_authorization_records_terminal_grant_with_wire_cov() {
    terminal_event(false, AccessEvent::GRANTED).await;
}

#[tokio::test(start_paused = true)]
async fn local_authorization_records_later_denial_after_successful_rights() {
    terminal_event(true, AccessEvent::DENIED_LOCKOUT).await;
}

#[path = "local_authorization_fixture.rs"]
mod fixture;
#[path = "local_authorization_publication_tests.rs"]
mod publication;

#[path = "local_authorization_boundary.rs"]
mod boundary;
#[path = "local_authorization_diagnostics_tests.rs"]
mod diagnostics;

#[path = "local_authorization_contention_tests.rs"]
mod contention;
#[path = "local_authorization_denial_tests.rs"]
mod denials;
#[path = "local_authorization_lifecycle_tests.rs"]
mod lifecycle;

#[path = "local_authorization_stamp_tests.rs"]
mod stamps;
