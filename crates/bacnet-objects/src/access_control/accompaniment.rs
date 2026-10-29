//! Local accompaniment stage (Clause 12.34.11), separate from authorization.
use super::accompaniment_types::*;
use super::{AccessRightsDecision, evaluate_access_rights};
use crate::database::ObjectDatabase;
use crate::prelude::*;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{AccessEvent, ObjectType};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;
use core::time::Duration;

#[path = "accompaniment_read.rs"]
mod read;
#[path = "accompaniment_timing.rs"]
mod timing;

/// Evaluate local accompaniment evidence without writing objects, obtaining a
/// clock, performing I/O, or publishing an event. Rights are recomputed in this
/// database borrow; a caller-provided grant is never accepted as authority.
///
/// This is only an accompaniment stage. Authentication, credential activity,
/// other authorization checks, pending transactions, and exactly-once event
/// publication belong to the caller. See [`AccompanimentRequest`] for explicit
/// timing and bounded-chain evidence; no default order or timeout is installed.
///
/// A direct attempt using the original credential is incorrect. A chain that
/// returns to an earlier credential after a distinct companion is unresolved;
/// no cyclic authorization fixed point is inferred. Missing witness rows also
/// remain unresolved, while an explicit Waiting row can reach a verified expiry.
/// Intents refer only to the requested original transaction, including when a
/// child's failure invalidates its companion.
///
/// # Example
/// ```
/// use bacnet_objects::{access_control::*, database::ObjectDatabase};
/// use bacnet_types::{
///     constructed::{BACnetAccessRule, BACnetAssignedAccessRights},
///     enums::ObjectType,
///     primitives::ObjectIdentifier,
/// };
/// use core::time::Duration;
/// # fn main() -> Result<(), bacnet_types::error::Error> {
/// let id = |kind, n| ObjectIdentifier::new(kind, n).unwrap();
/// let point = id(ObjectType::ACCESS_POINT, 1);
/// let mut db = ObjectDatabase::new();
/// db.add(Box::new(AccessPointObject::new(1, "entry")?))?;
/// for n in 1..=2 {
///     let rights_id = id(ObjectType::ACCESS_RIGHTS, n);
///     let mut rights = AccessRightsObject::new(n, format!("rights-{n}"))?;
///     rights.set_positive_access_rules([
///         BACnetAccessRule::new(None, Some(point.into()), true),
///     ])?;
///     if n == 1 {
///         rights.set_accompaniment(Some(id(ObjectType::ACCESS_CREDENTIAL, 2).into()))?;
///     }
///     let mut card = AccessCredentialObject::new(n, format!("card-{n}"))?;
///     card.set_assigned_access_rights(vec![BACnetAssignedAccessRights {
///         assigned_access_rights: rights_id.into(), enable: true,
///     }])?;
///     db.add(Box::new(rights))?;
///     db.add(Box::new(card))?;
/// }
/// let original = CredentialPresentation {
///     sequence: 1, credential: id(ObjectType::ACCESS_CREDENTIAL, 1),
///     point, at: Duration::from_secs(20),
/// };
/// let companion = CredentialPresentation {
///     sequence: 2, credential: id(ObjectType::ACCESS_CREDENTIAL, 2),
///     point, at: Duration::from_secs(21),
/// };
/// let witnesses = [AccompanimentWitness {
///     original: original.sequence,
///     evidence: AccompanimentEvidence::Presented(companion),
/// }];
/// let result = evaluate_accompaniment(&db, AccompanimentRequest {
///     original, now: companion.at, witnesses: &witnesses,
///     timing: AccompanimentTiming {
///         order: AccompanimentOrder::After,
///         fallback_period: Duration::from_secs(5),
///         deadline: AccompanimentDeadline::Inclusive,
///     },
///     max_links: 8,
/// })?;
/// assert_eq!(result.outcome, AccompanimentOutcome::Satisfied);
/// assert_eq!(result.events.len(), 1);
/// // The application still checks authentication and all other authorization
/// // criteria before acting; it owns transaction state and event publication.
/// # Ok(())
/// # }
/// ```
///
/// # Errors
/// Original credential/point or original rights-read errors are returned from
/// [`evaluate_access_rights`]. Unavailable accompaniment/companion evidence is an
/// [`AccompanimentOutcome::Unresolved`] result instead.
pub fn evaluate_accompaniment(
    db: &ObjectDatabase,
    request: AccompanimentRequest<'_>,
) -> Result<AccompanimentEvaluation, Error> {
    let original_rights =
        evaluate_access_rights(db, request.original.credential, request.original.point)?;
    Ok(evaluate_with_rights(db, request, original_rights))
}

/// Internal composition seam: only a rights result computed in this database
/// borrow by a trusted sibling evaluator reaches here. No public grant input.
pub(super) fn evaluate_with_rights(
    db: &ObjectDatabase,
    request: AccompanimentRequest<'_>,
    original_rights: super::AccessRightsEvaluation,
) -> AccompanimentEvaluation {
    let mut result = AccompanimentEvaluation {
        original: request.original,
        original_rights,
        requirement: None,
        companions: Vec::new(),
        outcome: AccompanimentOutcome::NotEvaluated,
        events: Vec::new(),
    };
    let selected = match result.original_rights.decision {
        AccessRightsDecision::Denied { .. } => return result,
        AccessRightsDecision::Exempt => {
            result.outcome = AccompanimentOutcome::NotRequired;
            return result;
        }
        AccessRightsDecision::Granted { rule } => rule.access_rights,
    };
    result.outcome = match follow(db, &request, &mut result, selected) {
        Ok(outcome) => outcome,
        Err(problem) => AccompanimentOutcome::Unresolved(problem),
    };
    result
}

fn witness(
    request: &AccompanimentRequest<'_>,
    sequence: u64,
) -> Result<AccompanimentEvidence, AccompanimentProblem> {
    let mut entries = request
        .witnesses
        .iter()
        .filter(|witness| witness.original == sequence);
    let evidence = entries
        .next()
        .ok_or(AccompanimentProblem::MissingWitness {
            presentation: sequence,
        })?
        .evidence;
    if entries.next().is_some() {
        return Err(AccompanimentProblem::AmbiguousWitness {
            presentation: sequence,
        });
    }
    Ok(evidence)
}
fn deny(
    result: &mut AccompanimentEvaluation,
    current: CredentialPresentation,
    missing: bool,
) -> AccompanimentOutcome {
    let missing_original = missing && current == result.original;
    let event = if missing_original {
        AccessEvent::DENIED_NO_ACCOMPANIMENT
    } else {
        AccessEvent::DENIED_INCORRECT_ACCOMPANIMENT
    };
    result.events.push(AccompanimentEventIntent {
        original: result.original,
        access_event: event,
        presentation: result.original,
    });
    if missing_original {
        AccompanimentOutcome::DeniedNoCompanion
    } else {
        AccompanimentOutcome::DeniedIncorrectCompanion {
            presentation: current.sequence,
        }
    }
}
fn follow(
    db: &ObjectDatabase,
    request: &AccompanimentRequest<'_>,
    result: &mut AccompanimentEvaluation,
    mut selected: ObjectIdentifier,
) -> Result<AccompanimentOutcome, AccompanimentProblem> {
    let mut current = request.original;
    let mut path = vec![current];
    let mut period = None;
    let mut links = 0;
    loop {
        let requirement = read::requirement(db, selected)?;
        if let Some(child) = result.companions.last_mut() {
            child.requirement = requirement.clone();
        } else {
            result.requirement = requirement.clone();
        }
        let Some(requirement) = requirement else {
            return Ok(if links == 0 {
                AccompanimentOutcome::NotRequired
            } else {
                AccompanimentOutcome::Satisfied
            });
        };
        if links >= request.max_links {
            return Err(AccompanimentProblem::LinkLimit {
                max_links: request.max_links,
            });
        }
        let period = match period {
            Some(period) => period,
            None => {
                let value = read::period(db, current.point, request.timing.fallback_period)?;
                period = Some(value);
                value
            }
        };
        let expired = timing::expired(current, request.now, period, request.timing.deadline)?;
        let candidate = match witness(request, current.sequence)? {
            AccompanimentEvidence::Waiting if !expired => {
                read::available(db, &requirement)?;
                return Ok(AccompanimentOutcome::Waiting {
                    presentation: current.sequence,
                });
            }
            AccompanimentEvidence::DeadlineReached if !expired => {
                return Err(timing::problem(
                    current.sequence,
                    AccompanimentPresentationFailure::PrematureDeadline,
                ));
            }
            AccompanimentEvidence::Waiting | AccompanimentEvidence::DeadlineReached => {
                read::available(db, &requirement)?;
                return Ok(deny(result, current, true));
            }
            AccompanimentEvidence::Presented(candidate) => candidate,
        };
        if let Some(previous) = path
            .iter()
            .find(|previous| previous.sequence == candidate.sequence)
            && *previous != candidate
        {
            return Err(timing::problem(
                candidate.sequence,
                AccompanimentPresentationFailure::ReusedSequence,
            ));
        }
        if !timing::eligible(current, candidate, request, period, &path)? {
            read::available(db, &requirement)?;
            return Ok(deny(result, current, true));
        }
        if candidate.credential.object_type() != ObjectType::ACCESS_CREDENTIAL {
            return Err(AccompanimentProblem::Reference {
                reference: candidate.credential.into(),
                reason: AccompanimentReferenceFailure::WrongType,
            });
        }
        // Only the explicitly requested transaction publishes intents. Child
        // dependency inspection never creates a child's BY/denial transaction.
        if links == 0 {
            result.events.push(AccompanimentEventIntent {
                original: result.original,
                access_event: AccessEvent::ACCOMPANIMENT_BY,
                presentation: candidate,
            });
        }
        read::available(db, &requirement)?;
        if candidate.credential == current.credential {
            return Ok(deny(result, current, false));
        }
        if !read::matches(db, &requirement, candidate.credential)? {
            return Ok(deny(result, current, false));
        }
        // Re-entering an established dependency is unresolved even when its
        // earlier presentation would also violate this link's order policy.
        if path.iter().any(|previous| {
            previous.credential == candidate.credential || previous.sequence == candidate.sequence
        }) {
            path.push(candidate);
            return Err(AccompanimentProblem::CyclicRequirement { path });
        }
        if !timing::ordered(current, candidate, request.timing.order) {
            return Ok(deny(result, current, false));
        }
        let rights =
            evaluate_access_rights(db, candidate.credential, current.point).map_err(|error| {
                AccompanimentProblem::Rights {
                    presentation: candidate.sequence,
                    detail: error.to_string(),
                }
            })?;
        let decision = rights.decision;
        result.companions.push(AccompanimentCompanion {
            presentation: candidate,
            rights,
            requirement: None,
        });
        match decision {
            AccessRightsDecision::Denied { .. } => return Ok(deny(result, current, false)),
            AccessRightsDecision::Exempt => return Ok(AccompanimentOutcome::Satisfied),
            AccessRightsDecision::Granted { rule } => selected = rule.access_rights,
        }
        path.push(candidate);
        current = candidate;
        links += 1;
    }
}
