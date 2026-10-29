//! Internally computed rights and bounded companion prerequisites.
use super::*;
use bacnet_types::enums::PropertyIdentifier as P;

pub(super) fn check(
    db: &ObjectDatabase,
    request: &AuthorizationRequest<'_>,
    mode: AuthorizationMode,
    exemptions: &[AuthorizationExemption],
    result: &mut AuthorizationEvaluation,
) -> Result<AuthorizationCheckOutcome, AuthorizationProblem> {
    if mode == AuthorizationMode::GRANT_ACTIVE {
        return Ok(AuthorizationCheckOutcome::NotApplicable);
    }
    if exemptions.contains(&AuthorizationExemption::ACCESS_RIGHTS) {
        return Ok(AuthorizationCheckOutcome::Exempt(
            AuthorizationExemption::ACCESS_RIGHTS,
        ));
    }
    let rights = evaluate_access_rights(db, request.original.credential, request.original.point)
        .map_err(|e| read::error(request.original.credential, P::ASSIGNED_ACCESS_RIGHTS, e))?;
    result.rights = Some(rights.clone());
    diagnostics(request, request.original, &rights)?;
    match rights.decision {
        AccessRightsDecision::Denied { access_event, .. } => {
            return Ok(AuthorizationCheckOutcome::Denied(access_event));
        }
        AccessRightsDecision::Exempt => {
            return Err(AuthorizationProblem::Context(
                "original exemption evidence changed during rights evaluation",
            ));
        }
        AccessRightsDecision::Granted { .. } => (),
    }
    let accompaniment = super::super::accompaniment::evaluate_with_rights(
        db,
        AccompanimentRequest {
            original: request.original,
            now: request.now,
            witnesses: request.accompaniment_witnesses,
            timing: request.accompaniment_timing,
            max_links: request.max_accompaniment_links,
        },
        rights,
    );
    result.accompaniment = Some(accompaniment.clone());
    for intent in &accompaniment.events {
        if intent.access_event == AccessEvent::ACCOMPANIMENT_BY {
            result.event(intent.access_event, intent.presentation);
        }
    }
    for companion in &accompaniment.companions {
        diagnostics(request, companion.presentation, &companion.rights)?;
        // A rights-denied child is already a known-invalid accompaniment; its
        // authority is never relied on, so no authentication attestation is used.
        if matches!(
            companion.rights.decision,
            AccessRightsDecision::Denied { .. }
        ) {
            break;
        }
        if let Some(failure) = prerequisite(db, request, companion, result)? {
            result
                .companion_diagnostics
                .push(CompanionAuthorizationDiagnostic {
                    presentation: companion.presentation,
                    failure,
                });
            return Ok(AuthorizationCheckOutcome::Denied(
                AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT,
            ));
        }
    }
    Ok(match accompaniment.outcome {
        AccompanimentOutcome::NotRequired | AccompanimentOutcome::Satisfied => {
            AuthorizationCheckOutcome::Passed
        }
        AccompanimentOutcome::Waiting { .. } => AuthorizationCheckOutcome::Pending,
        AccompanimentOutcome::DeniedNoCompanion => {
            AuthorizationCheckOutcome::Denied(AccessEvent::DENIED_NO_ACCOMPANIMENT)
        }
        AccompanimentOutcome::DeniedIncorrectCompanion { .. } => {
            AuthorizationCheckOutcome::Denied(AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT)
        }
        AccompanimentOutcome::Unresolved(problem) => {
            return Err(AuthorizationProblem::Accompaniment(problem));
        }
        AccompanimentOutcome::NotEvaluated => {
            return Err(AuthorizationProblem::Context(
                "granted rights did not evaluate accompaniment",
            ));
        }
    })
}

fn diagnostics(
    request: &AuthorizationRequest<'_>,
    presentation: CredentialPresentation,
    rights: &AccessRightsEvaluation,
) -> Result<(), AuthorizationProblem> {
    if request.rights_policy == AuthorizationRightsPolicy::RequireCompleteLocalEvidence
        && !rights.unresolved.is_empty()
    {
        Err(AuthorizationProblem::ExcludedRights { presentation })
    } else {
        Ok(())
    }
}

fn prerequisite(
    db: &ObjectDatabase,
    request: &AuthorizationRequest<'_>,
    companion: &AccompanimentCompanion,
    result: &mut AuthorizationEvaluation,
) -> Result<Option<CompanionPrerequisiteFailure>, AuthorizationProblem> {
    let presentation = companion.presentation;
    let problem = |reason| AuthorizationProblem::CompanionAuthentication {
        presentation,
        reason,
    };
    let mut attestations = request
        .companion_authentication
        .iter()
        .filter(|attestation| attestation.presentation == presentation);
    let attestation = attestations
        .next()
        .ok_or_else(|| problem(CompanionAuthenticationProblem::Missing))?;
    if attestations.next().is_some() {
        return Err(problem(CompanionAuthenticationProblem::Duplicate));
    }
    match attestation.authentication {
        CompanionAuthentication::Unknown => {
            return Err(problem(CompanionAuthenticationProblem::Unknown));
        }
        CompanionAuthentication::Rejected => {
            return Ok(Some(CompanionPrerequisiteFailure::AuthenticationRejected));
        }
        CompanionAuthentication::Authenticated => (),
    }
    let exemptions = read::exemptions(db, presentation.credential)?;
    if exemptions.contains(&AuthorizationExemption::ACCESS_RIGHTS)
        != matches!(companion.rights.decision, AccessRightsDecision::Exempt)
    {
        return Err(AuthorizationProblem::Context(
            "companion exemption evidence changed during rights evaluation",
        ));
    }
    Ok(
        credential::check(db, request, presentation, result)?.map(|(reason, access_event)| {
            CompanionPrerequisiteFailure::CredentialDenied {
                reason,
                access_event,
            }
        }),
    )
}
