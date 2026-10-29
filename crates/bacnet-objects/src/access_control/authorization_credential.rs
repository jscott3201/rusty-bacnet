//! Credential evidence at one supplied wall-clock moment (Clause 12.35).

use super::super::credential_rules::{WindowLimit, window_reasons};
use super::*;
use bacnet_types::enums::{
    AccessCredentialDisableReason as Reason, BinaryPV, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::primitives::PropertyValue;

/// Read each clock-sensitive served value once. Never retry a contradictory
/// reader or remove time-owned disable reasons to manufacture activity.
pub(super) fn check(
    db: &ObjectDatabase,
    request: &AuthorizationRequest<'_>,
    presentation: CredentialPresentation,
    result: &mut AuthorizationEvaluation,
) -> Result<Option<(Reason, AccessEvent)>, AuthorizationProblem> {
    let oid = presentation.credential;
    read::object(db, oid, ObjectType::ACCESS_CREDENTIAL)
        .map_err(|e| read::error(oid, P::CREDENTIAL_STATUS, e))?;
    let status = match read::required(db, oid, P::CREDENTIAL_STATUS)? {
        PropertyValue::Enumerated(raw) => BinaryPV::from_raw(raw),
        _ => {
            return Err(read::malformed(
                oid,
                P::CREDENTIAL_STATUS,
                "expected credential status enumeration",
            ));
        }
    };
    let reasons = match read::required(db, oid, P::REASON_FOR_DISABLE)? {
        PropertyValue::List(values) => values
            .into_iter()
            .map(|value| match value {
                PropertyValue::Enumerated(raw) => Ok(Reason::from_raw(raw)),
                _ => Err(read::malformed(
                    oid,
                    P::REASON_FOR_DISABLE,
                    "expected Enumerated disable reason",
                )),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(read::malformed(
                oid,
                P::REASON_FOR_DISABLE,
                "expected disable reason list",
            ));
        }
    };
    let activation = WindowLimit::from_property(read::required(db, oid, P::ACTIVATION_TIME)?)
        .map_err(|e| read::error(oid, P::ACTIVATION_TIME, e))?;
    let expiration = WindowLimit::from_property(read::required(db, oid, P::EXPIRATION_TIME)?)
        .map_err(|e| read::error(oid, P::EXPIRATION_TIME, e))?;
    let open = activation == WindowLimit::OPEN && expiration == WindowLimit::OPEN;
    let frame = request
        .wall_time
        .filter(|frame| frame.is_valid_actual_datetime());
    let derived: Option<Vec<_>> =
        (open || frame.is_some()).then(|| window_reasons(frame, activation, expiration).collect());
    result.credentials.push(AuthorizationCredentialEvidence {
        presentation,
        status,
        reasons: reasons.clone(),
        window_reasons: derived.clone(),
    });
    if !matches!(status, BinaryPV::ACTIVE | BinaryPV::INACTIVE) {
        return Err(AuthorizationProblem::UnsupportedValue {
            object: oid,
            property: P::CREDENTIAL_STATUS,
            value: status.to_raw().into(),
        });
    }
    let mut seen = [false; 10];
    for reason in &reasons {
        let index = reason.to_raw() as usize;
        let Some(slot) = seen.get_mut(index) else {
            return Err(AuthorizationProblem::UnsupportedValue {
                object: oid,
                property: P::REASON_FOR_DISABLE,
                value: reason.to_raw().into(),
            });
        };
        if core::mem::replace(slot, true) {
            return Err(read::malformed(
                oid,
                P::REASON_FOR_DISABLE,
                "duplicate disable reason",
            ));
        }
    }
    if (status == BinaryPV::ACTIVE) != reasons.is_empty() {
        return Err(AuthorizationProblem::InconsistentCredentialEvidence {
            credential: oid,
            detail: "status disagrees with served disable reasons",
        });
    }
    let derived = derived.ok_or(AuthorizationProblem::MissingFact(
        "valid wall time for finite credential limits",
    ))?;
    for reason in [Reason::DISABLED_NOT_YET_ACTIVE, Reason::DISABLED_EXPIRED] {
        if reasons.contains(&reason) != derived.contains(&reason) {
            return Err(AuthorizationProblem::InconsistentCredentialEvidence {
                credential: oid,
                detail: "served time-owned reasons disagree with supplied wall time",
            });
        }
    }
    Ok(request
        .disable_reason_priority
        .reasons()
        .iter()
        .find(|reason| reasons.contains(reason))
        .map(|reason| (*reason, denial(*reason))))
}

/// Table 12-41's reasons, plus expiration from Clauses 12.35.12/12.31.27.
/// Unknown reasons are rejected before this closed mapping is used.
fn denial(reason: Reason) -> AccessEvent {
    match reason {
        Reason::DISABLED => AccessEvent::DENIED_CREDENTIAL_DISABLED,
        Reason::DISABLED_NEEDS_PROVISIONING => AccessEvent::DENIED_CREDENTIAL_NOT_PROVISIONED,
        Reason::DISABLED_UNASSIGNED => AccessEvent::DENIED_CREDENTIAL_UNASSIGNED,
        Reason::DISABLED_NOT_YET_ACTIVE => AccessEvent::DENIED_CREDENTIAL_NOT_YET_ACTIVE,
        Reason::DISABLED_EXPIRED => AccessEvent::DENIED_CREDENTIAL_EXPIRED,
        Reason::DISABLED_LOCKOUT => AccessEvent::DENIED_CREDENTIAL_LOCKOUT,
        Reason::DISABLED_MAX_DAYS => AccessEvent::DENIED_CREDENTIAL_MAX_DAYS,
        Reason::DISABLED_MAX_USES => AccessEvent::DENIED_CREDENTIAL_MAX_USES,
        Reason::DISABLED_INACTIVITY => AccessEvent::DENIED_CREDENTIAL_INACTIVITY,
        Reason::DISABLED_MANUAL => AccessEvent::DENIED_CREDENTIAL_MANUAL_DISABLE,
        _ => unreachable!("only checked standard reasons reach denial selection"),
    }
}
