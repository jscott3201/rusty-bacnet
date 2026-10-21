use super::*;
use crate::mutation::{MutationService, MutationTarget, MutationTrust};
use bacnet_services::audit::AuditNotificationRequest;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_services::object_mgmt::DeleteObjectRequest;
use bacnet_transport::port::TransportProvenance;
use bacnet_types::enums::{ConfirmedServiceChoice, LifeSafetyOperation, ObjectType};
use bacnet_types::primitives::ObjectIdentifier;
use bacnet_types::MacAddr;

struct Defaults;
impl ServerPolicy for Defaults {}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::AUDIT_LOG, 1).unwrap()
}

fn mutation() -> MutationAuthorizationContext {
    MutationAuthorizationContext {
        source_mac: MacAddr::from_slice(&[1]),
        source_network: None,
        provenance: TransportProvenance::unverified(),
        trust: MutationTrust::Unverified,
        invoke_id: Some(1),
        service_choice: MutationService::Confirmed(ConfirmedServiceChoice::DELETE_OBJECT),
        target: MutationTarget::DeleteObject(DeleteObjectRequest {
            object_identifier: oid(),
        }),
    }
}

fn life_safety() -> LifeSafetyOperationAuthorizationContext {
    LifeSafetyOperationAuthorizationContext {
        source_mac: MacAddr::from_slice(&[1]),
        source_network: None,
        provenance: TransportProvenance::unverified(),
        invoke_id: 1,
        request: LifeSafetyOperationRequest {
            requesting_process_identifier: 1,
            requesting_source: String::new(),
            request: LifeSafetyOperation::RESET,
            object_identifier: None,
        },
    }
}

fn audit() -> AuditNotificationAuthorizationContext {
    AuditNotificationAuthorizationContext {
        source_mac: MacAddr::from_slice(&[1]),
        source_network: None,
        provenance: TransportProvenance::unverified(),
        invoke_id: 1,
        audit_log_sink: oid(),
        request: AuditNotificationRequest {
            notifications: Vec::new(),
        },
    }
}

fn unconfirmed_audit() -> UnconfirmedAuditNotificationAuthorizationContext {
    UnconfirmedAuditNotificationAuthorizationContext {
        source_mac: MacAddr::from_slice(&[1]),
        source_network: None,
        provenance: TransportProvenance::unverified(),
        audit_log_sink: oid(),
        request: AuditNotificationRequest {
            notifications: Vec::new(),
        },
    }
}

/// Each decision as `policy` answers it.
fn decisions(policy: &dyn ServerPolicy) -> [bool; 4] {
    [
        policy.authorize_mutation(&mutation()),
        policy.authorize_life_safety_operation(&life_safety()),
        policy.authorize_audit_notification(&audit()),
        policy.authorize_unconfirmed_audit_notification(&unconfirmed_audit()),
    ]
}

#[test]
fn defaults_allow_mutations_and_deny_the_rest() {
    assert_eq!(decisions(&Defaults), [true, false, false, false]);
    assert_eq!(decisions(&FnPolicy::new()), [true, false, false, false]);
    assert_eq!(mutation().principal(), None);
    assert_eq!(life_safety().principal(), None);
    assert_eq!(audit().principal(), None);
    assert_eq!(unconfirmed_audit().principal(), None);
}

#[test]
fn each_closure_decides_only_its_own_service() {
    let flipped = FnPolicy::new()
        .mutation(|_| false)
        .life_safety_operation(|_| true)
        .audit_notification(|_| true)
        .unconfirmed_audit_notification(|_| true);
    assert_eq!(decisions(&flipped), [false, true, true, true]);
    let one = FnPolicy::new().audit_notification(|_| true);
    assert_eq!(decisions(&one), [true, false, true, false]);
    assert_eq!(
        format!("{one:?}"),
        "FnPolicy { mutation: false, life_safety_operation: false, \
         audit_notification: true, unconfirmed_audit_notification: false }"
    );
}

#[test]
fn shared_and_boxed_policies_forward_every_decision() {
    let flipped = || {
        FnPolicy::new()
            .mutation(|_| false)
            .life_safety_operation(|_| true)
    };
    let shared: Arc<dyn ServerPolicy> = Arc::new(flipped());
    let boxed: Box<dyn ServerPolicy> = Box::new(flipped());
    let concrete = Arc::new(flipped());
    for policy in [&shared as &dyn ServerPolicy, &boxed, &concrete] {
        assert_eq!(decisions(policy), [false, true, false, false]);
        assert!(policy.decides_mutations());
    }
    assert!(Defaults.decides_mutations());
    assert!(!FnPolicy::new().decides_mutations());
    assert!(!Arc::new(FnPolicy::new().audit_notification(|_| true)).decides_mutations());
}
