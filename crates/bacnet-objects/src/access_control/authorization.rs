//! Pure authorization composition for one authenticated local presentation.

use super::*;
use crate::database::ObjectDatabase;
use bacnet_types::enums::{AuthorizationExemption, AuthorizationMode, ObjectType};

#[path = "authorization_checks.rs"]
mod checks;
#[path = "authorization_credential.rs"]
mod credential;
#[path = "authorization_phase.rs"]
mod phase;
#[path = "authorization_read.rs"]
mod read;
#[path = "authorization_rights.rs"]
mod rights;

/// Evaluate the complete supplied local authorization context without locks,
/// awaits, I/O, clock acquisition, or property writes (Clauses 12.31/12.35).
///
/// The request already represents an authenticated presentation. Every required
/// criterion is evaluated in the explicit complete order, stopping at the first
/// denial, pending dependency, or unresolved fact. The mode's external phase
/// follows successful criteria. NONE returns NoAuthorization, never Granted.
///
/// Original and companion validity use the same supplied wall time. Status and
/// reasons are read once each and must agree with that time's validity result;
/// a changing object's clock is not repaired or retried into a grant. This is
/// consistent evidence at the supplied moment, not an atomic snapshot of an
/// arbitrary application-owned ClockReader.
///
/// Assigned rights and accompaniment are computed internally under this shared
/// database borrow. Only each relied-on companion's exact authentication tuple
/// and credential prerequisites are additionally checked; child transactions or
/// recursive whole-controller authorization are not created.
///
/// PASSBACK exemption removes enforcement while preserving a known SOFT
/// detection (Clauses 12.35.25/12.32.21). As this API's evidence contract, missing
/// or unusable optional reporting facts retain
/// [`AuthorizationEvaluation::passback_reporting_problem`] without blocking an
/// exempt check. This does not define a standard-wide unknown-evidence policy;
/// unknown facts for required criteria still stop the pass.
///
/// Only [`AuthorizationOutcome::Granted`] permits access. Returned root intents
/// are deterministic for fixed reads/evidence and contain at most four events,
/// with terminal grant/denial last. The application owns publication, exactly-once
/// handling, pending state, counters/history, and any physical action. Existing
/// rights/accompaniment helpers retain their independent stage-only semantics.
///
/// # Errors
/// An original identifier that does not name a local Access Credential or Access
/// Point returns OBJECT / UNKNOWN_OBJECT. Missing/malformed/unsupported required
/// evidence is retained in an Unresolved evaluation rather than a fabricated
/// DENIED_OTHER. Request ordering and reason priority are validated at construction.
pub fn evaluate_authorization(
    db: &ObjectDatabase,
    request: AuthorizationRequest<'_>,
) -> Result<AuthorizationEvaluation, Error> {
    read::object(
        db,
        request.original.credential,
        ObjectType::ACCESS_CREDENTIAL,
    )?;
    read::object(db, request.original.point, ObjectType::ACCESS_POINT)?;
    let mut result = AuthorizationEvaluation {
        original: request.original,
        mode: None,
        outcome: AuthorizationOutcome::NoAuthorization,
        checks: Vec::new(),
        rights: None,
        accompaniment: None,
        credentials: Vec::new(),
        companion_diagnostics: Vec::new(),
        passback_reporting_problem: None,
        events: Vec::new(),
    };
    let mode = match read::mode(db, request.original.point) {
        Ok(mode) => mode,
        Err(problem) => {
            result.record(
                AuthorizationStage::Admission,
                AuthorizationCheckOutcome::Unresolved(problem),
            );
            return Ok(result);
        }
    };
    result.mode = Some(mode);
    if mode == AuthorizationMode::NONE {
        return Ok(result);
    }
    if request.now < request.original.at {
        result.record(
            AuthorizationStage::Admission,
            AuthorizationCheckOutcome::Unresolved(AuthorizationProblem::Context(
                "original presentation is after observation time",
            )),
        );
        return Ok(result);
    }
    let exemptions = match read::exemptions(db, request.original.credential) {
        Ok(exemptions) => exemptions,
        Err(problem) => {
            result.record(
                AuthorizationStage::Admission,
                AuthorizationCheckOutcome::Unresolved(problem),
            );
            return Ok(result);
        }
    };
    for check in request.order.checks() {
        let outcome = match *check {
            AuthorizationCheck::Credential => {
                credential::check(db, &request, request.original, &mut result).map(|denial| {
                    denial.map_or(AuthorizationCheckOutcome::Passed, |(_, event)| {
                        AuthorizationCheckOutcome::Denied(event)
                    })
                })
            }
            AuthorizationCheck::AccessRights => {
                rights::check(db, &request, mode, &exemptions, &mut result)
            }
            other => checks::check(db, &request, mode, &exemptions, other, &mut result),
        }
        .unwrap_or_else(AuthorizationCheckOutcome::Unresolved);
        if !result.record(AuthorizationStage::Criterion(*check), outcome) {
            return Ok(result);
        }
    }
    if let Some((stage, outcome)) = phase::check(db, &request, mode, &exemptions, &mut result)
        && !result.record(stage, outcome)
    {
        return Ok(result);
    }
    result.outcome = AuthorizationOutcome::Granted;
    result.event(AccessEvent::GRANTED, request.original);
    Ok(result)
}

impl AuthorizationEvaluation {
    fn record(&mut self, stage: AuthorizationStage, outcome: AuthorizationCheckOutcome) -> bool {
        self.checks.push(AuthorizationCheckEvidence {
            stage,
            outcome: outcome.clone(),
        });
        match outcome {
            AuthorizationCheckOutcome::Passed
            | AuthorizationCheckOutcome::Exempt(_)
            | AuthorizationCheckOutcome::NotApplicable => true,
            AuthorizationCheckOutcome::Denied(access_event) => {
                self.outcome = AuthorizationOutcome::Denied {
                    stage,
                    access_event,
                };
                self.event(access_event, self.original);
                false
            }
            AuthorizationCheckOutcome::Pending => {
                self.outcome = AuthorizationOutcome::Pending(stage);
                false
            }
            AuthorizationCheckOutcome::Unresolved(problem) => {
                self.outcome = AuthorizationOutcome::Unresolved { stage, problem };
                false
            }
        }
    }

    fn event(&mut self, access_event: AccessEvent, presentation: CredentialPresentation) {
        self.events.push(AuthorizationEventIntent {
            original: self.original,
            access_event,
            presentation,
        });
    }
}
