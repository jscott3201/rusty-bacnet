//! Monotonic differences avoid deadline addition/overflow and wall-clock reads.
use super::*;

pub(super) fn problem(
    presentation: u64,
    reason: AccompanimentPresentationFailure,
) -> AccompanimentProblem {
    AccompanimentProblem::Presentation {
        presentation,
        reason,
    }
}
pub(super) fn inside(delta: Duration, period: Duration, edge: AccompanimentDeadline) -> bool {
    match edge {
        AccompanimentDeadline::Inclusive => delta <= period,
        AccompanimentDeadline::Exclusive => delta < period,
    }
}
pub(super) fn expired(
    current: CredentialPresentation,
    now: Duration,
    period: Duration,
    edge: AccompanimentDeadline,
) -> Result<bool, AccompanimentProblem> {
    let delta = now
        .checked_sub(current.at)
        .ok_or_else(|| problem(current.sequence, AccompanimentPresentationFailure::Future))?;
    Ok(!inside(delta, period, edge))
}
/// False is a late attempted companion, so it cannot reopen an expired wait.
/// Stale-before, wrong-point and inconsistent evidence remain unresolved.
pub(super) fn eligible(
    current: CredentialPresentation,
    candidate: CredentialPresentation,
    request: &AccompanimentRequest<'_>,
    period: Duration,
    path: &[CredentialPresentation],
) -> Result<bool, AccompanimentProblem> {
    let invalid = |reason| problem(candidate.sequence, reason);
    if current.point != candidate.point {
        return Err(invalid(AccompanimentPresentationFailure::WrongPoint));
    }
    if candidate.at > request.now {
        return Err(invalid(AccompanimentPresentationFailure::Future));
    }
    // Either-order chains may zigzag, so adjacent agreement alone cannot
    // establish one consistent presentation history. Equal times remain valid.
    if path.iter().any(|previous| {
        (candidate.sequence < previous.sequence && candidate.at > previous.at)
            || (candidate.sequence > previous.sequence && candidate.at < previous.at)
    }) {
        return Err(invalid(AccompanimentPresentationFailure::NonMonotonic));
    }
    let delta = candidate.at.abs_diff(current.at);
    if inside(delta, period, request.timing.deadline) {
        return Ok(true);
    }
    if candidate.at >= current.at {
        Ok(false)
    } else {
        Err(invalid(AccompanimentPresentationFailure::OutsideWindow))
    }
}
pub(super) fn ordered(
    current: CredentialPresentation,
    candidate: CredentialPresentation,
    order: AccompanimentOrder,
) -> bool {
    match order {
        AccompanimentOrder::Before => candidate.sequence < current.sequence,
        AccompanimentOrder::After => candidate.sequence > current.sequence,
        AccompanimentOrder::Either => candidate.sequence != current.sequence,
    }
}
