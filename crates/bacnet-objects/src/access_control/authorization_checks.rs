//! Ordered operational criteria (Clauses 12.31, 12.32 and 12.35).
use super::*;
use bacnet_types::constructed::BACnetDeviceObjectReference;
use bacnet_types::enums::{AccessPassbackMode, ObjectType, PropertyIdentifier as P};
use bacnet_types::primitives::PropertyValue;
use core::time::Duration;

type Outcome = Result<AuthorizationCheckOutcome, AuthorizationProblem>;

pub(super) fn check(
    db: &ObjectDatabase,
    request: &AuthorizationRequest<'_>,
    mode: AuthorizationMode,
    exemptions: &[AuthorizationExemption],
    criterion: AuthorizationCheck,
    result: &mut AuthorizationEvaluation,
) -> Outcome {
    let exemption = match criterion {
        AuthorizationCheck::Occupancy => Some(AuthorizationExemption::OCCUPANCY_CHECK),
        AuthorizationCheck::Lockout => Some(AuthorizationExemption::LOCKOUT),
        AuthorizationCheck::Mode if mode == AuthorizationMode::DENY_ALL => {
            Some(AuthorizationExemption::DENY)
        }
        _ => None,
    };
    if let Some(exemption) = exemption.filter(|e| exemptions.contains(e)) {
        return Ok(AuthorizationCheckOutcome::Exempt(exemption));
    }
    match criterion {
        AuthorizationCheck::Threat => threat(db, request),
        AuthorizationCheck::Occupancy => occupancy(db, request),
        AuthorizationCheck::Passback => {
            if exemptions.contains(&AuthorizationExemption::PASSBACK) {
                // Exemption removes enforcement, but an established SOFT
                // detection remains reportable (Clauses 12.35.25/12.32.21).
                // Unknown optional reporting evidence is diagnostic only.
                if let Err(problem) = passback(db, request, result, true) {
                    result.passback_reporting_problem = Some(problem);
                }
                Ok(AuthorizationCheckOutcome::Exempt(
                    AuthorizationExemption::PASSBACK,
                ))
            } else {
                passback(db, request, result, false)
            }
        }
        AuthorizationCheck::Lockout => Ok(denial_if(
            read::boolean(
                db,
                request.original.point,
                P::LOCKOUT,
                request.fallback_lockout,
                "point lockout",
            )?,
            AccessEvent::DENIED_LOCKOUT,
        )),
        AuthorizationCheck::Mode => Ok(denial_if(
            mode == AuthorizationMode::DENY_ALL,
            AccessEvent::DENIED_DENY_ALL,
        )),
        AuthorizationCheck::Credential | AuthorizationCheck::AccessRights => {
            unreachable!("controller dispatches credential and rights separately")
        }
    }
}

fn denial_if(denied: bool, event: AccessEvent) -> AuthorizationCheckOutcome {
    if denied {
        AuthorizationCheckOutcome::Denied(event)
    } else {
        AuthorizationCheckOutcome::Passed
    }
}

fn level(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: P,
    fallback: AuthorizationFact<u32>,
) -> Result<u64, AuthorizationProblem> {
    let value = match read::optional(db, oid, property)? {
        Some(PropertyValue::Unsigned(value)) => value,
        Some(_) => {
            return Err(read::malformed(
                oid,
                property,
                "expected Unsigned threat value",
            ));
        }
        None => match fallback {
            AuthorizationFact::Known(value) => u64::from(value),
            AuthorizationFact::Unknown => {
                return Err(AuthorizationProblem::MissingFact("point threat level"));
            }
        },
    };
    if value > 100 {
        return Err(AuthorizationProblem::UnsupportedValue {
            object: oid,
            property,
            value,
        });
    }
    Ok(value)
}

fn threat(db: &ObjectDatabase, request: &AuthorizationRequest<'_>) -> Outcome {
    let point = level(
        db,
        request.original.point,
        P::THREAT_LEVEL,
        request.fallback_threat_level,
    )?;
    // Clause 12.31.22: zero disables the comparison. No credential authority
    // evidence is required when this point imposes no threat restriction.
    if point == 0 {
        return Ok(AuthorizationCheckOutcome::NotApplicable);
    }
    let authority = level(
        db,
        request.original.credential,
        P::THREAT_AUTHORITY,
        AuthorizationFact::Known(0),
    )?;
    Ok(denial_if(
        authority < point,
        AccessEvent::DENIED_THREAT_LEVEL,
    ))
}

fn occupancy(db: &ObjectDatabase, request: &AuthorizationRequest<'_>) -> Outcome {
    let zones = match request.occupancy {
        AuthorizationOccupancy::NotApplicable => {
            return Ok(AuthorizationCheckOutcome::NotApplicable);
        }
        AuthorizationOccupancy::Unknown => {
            return Err(AuthorizationProblem::MissingFact(
                "relevant occupancy zones and direction",
            ));
        }
        AuthorizationOccupancy::Zones([]) => {
            return Err(AuthorizationProblem::Context(
                "empty relevant occupancy zone set",
            ));
        }
        AuthorizationOccupancy::Zones(zones) => zones,
    };
    for (index, context) in zones.iter().enumerate() {
        if zones[..index]
            .iter()
            .any(|old| old.zone == context.zone && old.direction == context.direction)
        {
            return Err(AuthorizationProblem::Context(
                "duplicate occupancy zone and direction",
            ));
        }
        let outcome = occupancy_zone(db, request.original.point, *context)?;
        if matches!(outcome, AuthorizationCheckOutcome::Denied(_)) {
            return Ok(outcome);
        }
    }
    Ok(AuthorizationCheckOutcome::Passed)
}

fn occupancy_zone(
    db: &ObjectDatabase,
    point: ObjectIdentifier,
    context: AuthorizationOccupancyZone,
) -> Outcome {
    let (enforced, bound, topology, point_zone, event) = match context.direction {
        AuthorizationDirection::Entering => (
            P::OCCUPANCY_UPPER_LIMIT_ENFORCED,
            P::OCCUPANCY_UPPER_LIMIT,
            P::ENTRY_POINTS,
            P::ZONE_TO,
            AccessEvent::DENIED_UPPER_OCCUPANCY_LIMIT,
        ),
        AuthorizationDirection::Leaving => (
            P::OCCUPANCY_LOWER_LIMIT_ENFORCED,
            P::OCCUPANCY_LOWER_LIMIT,
            P::EXIT_POINTS,
            P::ZONE_FROM,
            AccessEvent::DENIED_LOWER_OCCUPANCY_LIMIT,
        ),
    };
    if !read::boolean(
        db,
        point,
        enforced,
        context.fallback_enforced,
        "occupancy enforcement",
    )? {
        return Ok(AuthorizationCheckOutcome::NotApplicable);
    }
    read::object(db, context.zone, ObjectType::ACCESS_ZONE)
        .map_err(|e| read::error(context.zone, bound, e))?;
    if let Some(value) = read::optional(db, point, point_zone)? {
        let reference: BACnetDeviceObjectReference =
            crate::device_reference::decode_reference(&value)
                .map_err(|e| read::error(point, point_zone, e))?;
        if reference.object_identifier != context.zone
            || !db.local_device().is_local(reference.device_identifier)
        {
            return Err(AuthorizationProblem::Context(
                "point zone disagrees with occupancy context",
            ));
        }
    }
    let references: Vec<BACnetDeviceObjectReference> =
        crate::device_reference::decode_references(&read::required(db, context.zone, topology)?)
            .map_err(|e| read::error(context.zone, topology, e))?;
    if !references.iter().any(|reference| {
        reference.object_identifier == point
            && db.local_device().is_local(reference.device_identifier)
    }) {
        return Err(AuthorizationProblem::Context(
            "zone topology does not establish requested direction",
        ));
    }
    let limit = read::unsigned(db, context.zone, bound)?;
    if limit == 0 {
        return Ok(AuthorizationCheckOutcome::NotApplicable);
    }
    if !read::boolean(
        db,
        context.zone,
        P::OCCUPANCY_COUNT_ENABLE,
        AuthorizationFact::Unknown,
        "occupancy counting enabled",
    )? {
        return Err(AuthorizationProblem::MissingFact(
            "current occupancy while counting is disabled",
        ));
    }
    let count = read::unsigned(db, context.zone, P::OCCUPANCY_COUNT)?;
    Ok(denial_if(
        match context.direction {
            AuthorizationDirection::Entering => count >= limit,
            AuthorizationDirection::Leaving => count <= limit,
        },
        event,
    ))
}

fn passback(
    db: &ObjectDatabase,
    request: &AuthorizationRequest<'_>,
    result: &mut AuthorizationEvaluation,
    report_only: bool,
) -> Outcome {
    let (zone, fallback_mode, fallback_timeout, observation) = match request.passback {
        AuthorizationPassback::NotApplicable => {
            return Ok(AuthorizationCheckOutcome::NotApplicable);
        }
        AuthorizationPassback::Unknown => {
            return Err(AuthorizationProblem::MissingFact("passback applicability"));
        }
        AuthorizationPassback::Zone {
            zone,
            fallback_mode,
            fallback_timeout_minutes,
            observation,
        } => (zone, fallback_mode, fallback_timeout_minutes, observation),
    };
    read::object(db, zone, ObjectType::ACCESS_ZONE)
        .map_err(|e| read::error(zone, P::PASSBACK_MODE, e))?;
    let mode = match read::optional(db, zone, P::PASSBACK_MODE)? {
        Some(PropertyValue::Enumerated(raw)) => AccessPassbackMode::from_raw(raw),
        Some(_) => {
            return Err(read::malformed(
                zone,
                P::PASSBACK_MODE,
                "expected passback enumeration",
            ));
        }
        None => match fallback_mode {
            AuthorizationFact::Known(mode) => mode,
            AuthorizationFact::Unknown => {
                return Err(AuthorizationProblem::MissingFact("passback mode"));
            }
        },
    };
    match mode {
        AccessPassbackMode::PASSBACK_OFF => return Ok(AuthorizationCheckOutcome::NotApplicable),
        AccessPassbackMode::HARD_PASSBACK if report_only => {
            // There is no HARD informational event. An exempt presentation
            // needs neither its enforcement observation nor its timeout.
            return Ok(AuthorizationCheckOutcome::NotApplicable);
        }
        AccessPassbackMode::HARD_PASSBACK | AccessPassbackMode::SOFT_PASSBACK => (),
        _ => {
            return Err(AuthorizationProblem::UnsupportedValue {
                object: zone,
                property: P::PASSBACK_MODE,
                value: mode.to_raw().into(),
            });
        }
    }
    let observation = match observation {
        AuthorizationFact::Known(value) => value,
        AuthorizationFact::Unknown => {
            return Err(AuthorizationProblem::MissingFact(
                "passback violation observation",
            ));
        }
    };
    if observation.presentation != request.original || observation.zone != zone {
        return Err(AuthorizationProblem::Context(
            "passback observation belongs to another presentation or zone",
        ));
    }
    if report_only && !observation.violation {
        // A matching known nonviolation needs no expiry evidence for reporting.
        return Ok(AuthorizationCheckOutcome::Passed);
    }
    if observation
        .last_successful_entry
        .is_some_and(|entry| entry > request.original.at)
    {
        return Err(AuthorizationProblem::Context(
            "successful entry is after the presentation",
        ));
    }
    let timeout = match read::optional(db, zone, P::PASSBACK_TIMEOUT)? {
        Some(PropertyValue::Unsigned(value)) if u32::try_from(value).is_ok() => value,
        Some(PropertyValue::Unsigned(value)) => {
            return Err(AuthorizationProblem::UnsupportedValue {
                object: zone,
                property: P::PASSBACK_TIMEOUT,
                value,
            });
        }
        Some(_) => {
            return Err(read::malformed(
                zone,
                P::PASSBACK_TIMEOUT,
                "expected Unsigned timeout minutes",
            ));
        }
        None => u64::from(fallback_timeout.unwrap_or(0)),
    };
    if !observation.violation {
        return Ok(AuthorizationCheckOutcome::Passed);
    }
    if timeout > 0 {
        let entry = observation
            .last_successful_entry
            .ok_or(AuthorizationProblem::MissingFact(
                "last successful entry for passback expiry",
            ))?;
        if request.now - entry >= Duration::from_secs(timeout * 60) {
            return Ok(AuthorizationCheckOutcome::Passed);
        }
    }
    if mode == AccessPassbackMode::HARD_PASSBACK {
        Ok(AuthorizationCheckOutcome::Denied(
            AccessEvent::DENIED_PASSBACK,
        ))
    } else {
        result.event(AccessEvent::PASSBACK_DETECTED, request.original);
        Ok(AuthorizationCheckOutcome::Passed)
    }
}
