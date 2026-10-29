//! External verification and delay with explicit deadline edges (12.31.14–15).
use super::*;
use bacnet_types::enums::PropertyIdentifier as P;
use bacnet_types::primitives::PropertyValue;
use core::time::Duration;

pub(super) fn check(
    db: &ObjectDatabase,
    request: &AuthorizationRequest<'_>,
    mode: AuthorizationMode,
    exemptions: &[AuthorizationExemption],
    result: &mut AuthorizationEvaluation,
) -> Option<(AuthorizationStage, AuthorizationCheckOutcome)> {
    let (stage, exemption, event) = match mode {
        AuthorizationMode::VERIFICATION_REQUIRED => (
            AuthorizationStage::Verification,
            AuthorizationExemption::VERIFICATION,
            AccessEvent::VERIFICATION_REQUIRED,
        ),
        AuthorizationMode::AUTHORIZATION_DELAYED => (
            AuthorizationStage::Delay,
            AuthorizationExemption::AUTHORIZATION_DELAY,
            AccessEvent::AUTHORIZATION_DELAYED,
        ),
        _ => return None,
    };
    if exemptions.contains(&exemption) {
        return Some((stage, AuthorizationCheckOutcome::Exempt(exemption)));
    }
    result.event(event, request.original);
    Some((
        stage,
        evaluate(db, request, stage).unwrap_or_else(AuthorizationCheckOutcome::Unresolved),
    ))
}

fn expired(elapsed: Duration, period: Duration, boundary: AccompanimentDeadline) -> bool {
    match boundary {
        AccompanimentDeadline::Inclusive => elapsed > period,
        AccompanimentDeadline::Exclusive => elapsed >= period,
    }
}

fn evaluate(
    db: &ObjectDatabase,
    request: &AuthorizationRequest<'_>,
    stage: AuthorizationStage,
) -> Result<AuthorizationCheckOutcome, AuthorizationProblem> {
    let AuthorizationExternalPhase::Observed {
        presentation,
        started,
        fallback_period,
        deadline,
        verdict,
    } = request.external_phase
    else {
        return Err(AuthorizationProblem::MissingFact(
            "external authorization phase",
        ));
    };
    if presentation != request.original || started < presentation.at || started > request.now {
        return Err(AuthorizationProblem::Context(
            "external phase identity or start is inconsistent",
        ));
    }
    let period = match read::optional(db, presentation.point, P::VERIFICATION_TIME)? {
        Some(PropertyValue::Unsigned(seconds)) if u32::try_from(seconds).is_ok() => {
            Duration::from_secs(seconds)
        }
        Some(PropertyValue::Unsigned(value)) => {
            return Err(AuthorizationProblem::UnsupportedValue {
                object: presentation.point,
                property: P::VERIFICATION_TIME,
                value,
            });
        }
        Some(_) => {
            return Err(read::malformed(
                presentation.point,
                P::VERIFICATION_TIME,
                "expected Unsigned seconds",
            ));
        }
        None => match fallback_period {
            AuthorizationFact::Known(period) => period,
            AuthorizationFact::Unknown => {
                return Err(AuthorizationProblem::MissingFact("external phase period"));
            }
        },
    };
    let elapsed = request.now - started;
    let timed_out = expired(elapsed, period, deadline);
    let dated = match verdict {
        AuthorizationVerdict::Approved(at) | AuthorizationVerdict::Rejected(at) => {
            if at < started || at > request.now {
                return Err(AuthorizationProblem::Context(
                    "external verdict time is inconsistent",
                ));
            }
            Some(at - started)
        }
        AuthorizationVerdict::DeadlineReached if !timed_out => {
            return Err(AuthorizationProblem::Context(
                "external deadline claimed before expiry",
            ));
        }
        _ => None,
    };
    let timely = dated.is_some_and(|elapsed| !expired(elapsed, period, deadline));
    if matches!(verdict, AuthorizationVerdict::Rejected(_)) && timely {
        return Ok(AuthorizationCheckOutcome::Denied(
            AccessEvent::DENIED_VERIFICATION_FAILED,
        ));
    }
    if stage == AuthorizationStage::Verification {
        if matches!(verdict, AuthorizationVerdict::Approved(_)) && timely {
            Ok(AuthorizationCheckOutcome::Passed)
        } else if timed_out {
            Ok(AuthorizationCheckOutcome::Denied(
                AccessEvent::DENIED_VERIFICATION_TIMEOUT,
            ))
        } else {
            Ok(AuthorizationCheckOutcome::Pending)
        }
    } else if timed_out {
        Ok(AuthorizationCheckOutcome::Passed)
    } else {
        Ok(AuthorizationCheckOutcome::Pending)
    }
}
