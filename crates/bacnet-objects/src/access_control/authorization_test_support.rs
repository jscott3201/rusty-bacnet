//! Explicit independent fixtures; defaults below are test choices, never policy.
pub(super) use super::accompaniment_test_support::{
    Read, anytime, assignment, companion, credential, database, encoded_reference, oid, original,
    override_read, point, presented, rights, set_assignments, set_rights,
};
use super::*;
use crate::clock::{ClockFrame, ClockReader};
use crate::database::ObjectDatabase;
pub(super) use bacnet_types::enums::{
    AccessCredentialDisableReason as Reason, AuthorizationExemption as Exemption,
    AuthorizationMode as Mode, PropertyIdentifier as P,
};
pub(super) use std::time::Duration;

pub(super) const CHECKS: [AuthorizationCheck; 7] = [
    AuthorizationCheck::Credential,
    AuthorizationCheck::AccessRights,
    AuthorizationCheck::Threat,
    AuthorizationCheck::Occupancy,
    AuthorizationCheck::Passback,
    AuthorizationCheck::Lockout,
    AuthorizationCheck::Mode,
];
pub(super) const REASONS: [Reason; 10] = [
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
];
pub(super) fn request<'a>() -> AuthorizationRequest<'a> {
    AuthorizationRequest {
        original: original(),
        wall_time: None,
        now: Duration::from_secs(101),
        order: AuthorizationOrder::new(CHECKS).unwrap(),
        disable_reason_priority: DisableReasonPriority::new(REASONS).unwrap(),
        rights_policy: AuthorizationRightsPolicy::HonorExistingDecision,
        fallback_threat_level: AuthorizationFact::Known(0),
        fallback_lockout: AuthorizationFact::Known(false),
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
pub(super) fn eval(
    db: &ObjectDatabase,
    request: AuthorizationRequest<'_>,
) -> AuthorizationEvaluation {
    evaluate_authorization(db, request).unwrap()
}
pub(super) fn granted(result: &AuthorizationEvaluation) {
    assert_eq!(result.outcome, AuthorizationOutcome::Granted, "{result:?}");
    assert_eq!(
        result.events.last().unwrap().access_event,
        AccessEvent::GRANTED
    );
    assert_eq!(result.events.last().unwrap().presentation, original());
}
pub(super) fn denied(result: &AuthorizationEvaluation, event: AccessEvent) {
    assert!(
        matches!(result.outcome, AuthorizationOutcome::Denied { access_event, .. } if access_event == event),
        "{result:?}"
    );
    assert_eq!(result.events.last().unwrap().access_event, event);
    assert_eq!(result.events.last().unwrap().presentation, original());
    assert!(
        result
            .events
            .iter()
            .all(|e| e.access_event != AccessEvent::GRANTED)
    );
}
pub(super) fn unresolved(result: &AuthorizationEvaluation) {
    assert!(
        matches!(result.outcome, AuthorizationOutcome::Unresolved { .. }),
        "{result:?}"
    );
    assert!(
        result
            .events
            .iter()
            .all(|e| e.access_event != AccessEvent::GRANTED)
    );
}
pub(super) fn events(result: &AuthorizationEvaluation) -> Vec<AccessEvent> {
    result.events.iter().map(|e| e.access_event).collect()
}
pub(super) fn value(db: &mut ObjectDatabase, oid: ObjectIdentifier, p: P, value: PropertyValue) {
    override_read(db, oid, p, Read::Value(value), true);
}
pub(super) fn mode(db: &mut ObjectDatabase, mode: Mode) {
    value(
        db,
        point(),
        P::AUTHORIZATION_MODE,
        PropertyValue::Enumerated(mode.to_raw()),
    );
}
pub(super) fn exempt(db: &mut ObjectDatabase, n: u32, exemptions: &[Exemption]) {
    value(
        db,
        credential(n),
        P::AUTHORIZATION_EXEMPTIONS,
        PropertyValue::List(
            exemptions
                .iter()
                .map(|e| PropertyValue::Enumerated(e.to_raw()))
                .collect(),
        ),
    );
}
pub(super) fn reasons(db: &mut ObjectDatabase, n: u32, reasons: &[Reason]) {
    value(
        db,
        credential(n),
        P::REASON_FOR_DISABLE,
        PropertyValue::List(
            reasons
                .iter()
                .map(|r| PropertyValue::Enumerated(r.to_raw()))
                .collect(),
        ),
    );
    value(
        db,
        credential(n),
        P::CREDENTIAL_STATUS,
        PropertyValue::Enumerated(if reasons.is_empty() { 1 } else { 0 }),
    );
}
pub(super) fn frame(hour: u8) -> ClockFrame {
    ClockFrame {
        local_date: Date {
            year: 126,
            month: 10,
            day: 23,
            day_of_week: 5,
        },
        local_time: Time {
            hour,
            minute: 0,
            second: 0,
            hundredths: 0,
        },
        utc_offset: 0,
        daylight_savings_status: false,
    }
}
pub(super) fn date_time(hour: u8) -> PropertyValue {
    let frame = frame(hour);
    PropertyValue::List(vec![
        PropertyValue::Date(frame.local_date),
        PropertyValue::Time(frame.local_time),
    ])
}
pub(super) struct FixedClock(pub Option<ClockFrame>);
impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        self.0
    }
}
pub(super) fn window_card(n: u32) -> AccessCredentialObject {
    let mut card = AccessCredentialObject::new(n, format!("window-{n}")).unwrap();
    card.set_assigned_access_rights(vec![assignment(rights(n), true)])
        .unwrap();
    card.set_activation_time(frame(10).local_date, frame(10).local_time)
        .unwrap();
    card.set_expiration_time(frame(12).local_date, frame(12).local_time)
        .unwrap();
    card
}
pub(super) fn install_window(db: &mut ObjectDatabase, n: u32) {
    db.remove(&credential(n)).unwrap();
    db.add(Box::new(window_card(n))).unwrap();
}
pub(super) fn zone() -> ObjectIdentifier {
    oid(ObjectType::ACCESS_ZONE, 1)
}
pub(super) fn add_zone(db: &mut ObjectDatabase, count: u64, lower: u64, upper: u64) {
    let mut object = AccessZoneObject::new(1, "zone").unwrap();
    object.set_entry_points(vec![point()]).unwrap();
    object.set_exit_points(vec![point()]).unwrap();
    object.set_occupancy_count_enable(true);
    object.set_occupancy_count(count);
    object.set_occupancy_limits(lower, upper).unwrap();
    db.add(Box::new(object)).unwrap();
}
pub(super) fn snapshot(db: &ObjectDatabase) -> Vec<(ObjectIdentifier, P, String)> {
    let mut ids = db.list_objects();
    ids.sort_by_key(|id| (id.object_type().to_raw(), id.instance_number()));
    ids.into_iter()
        .flat_map(|id| {
            let object = db.get(&id).unwrap();
            object
                .property_list()
                .into_owned()
                .into_iter()
                .map(move |p| (id, p, format!("{:?}", object.read_property(p, None))))
        })
        .collect()
}
