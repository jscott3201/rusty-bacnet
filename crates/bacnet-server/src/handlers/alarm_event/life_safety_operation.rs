//! LifeSafetyOperation service handler.
//!
use super::super::*;
use bacnet_objects::traits::LifeSafetyOperationEffect;
use bacnet_services::life_safety::LifeSafetyOperationRequest;
use bacnet_types::enums::{ErrorClass, ErrorCode, LifeSafetyOperation};

use crate::life_safety_cov::LifeSafetyCovChange;

/// Detailed internal service result retained through response dispatch.
pub(crate) struct LifeSafetyOperationHandlerResult {
    pub(crate) applied_object_identifiers: Vec<ObjectIdentifier>,
    pub(crate) cov_changes: Vec<LifeSafetyCovChange>,
}

/// Handle a LifeSafetyOperation request.
///
/// Targeted requests return the exact Clause 13.13 object error. Targetless
/// reset requests attempt only Life Safety Point and Zone objects; targetless
/// silence/unsilence retains its generic legacy traversal. Successful
/// per-object mutations are retained. Returned identifiers are objects whose
/// state changed.
pub fn handle_life_safety_operation(
    db: &mut ObjectDatabase,
    request: &LifeSafetyOperationRequest,
) -> Result<Vec<ObjectIdentifier>, Error> {
    handle_life_safety_operation_detailed(db, request)
        .map(|result| result.applied_object_identifiers)
}

/// Handle a LifeSafetyOperation while retaining exact known property deltas.
pub(crate) fn handle_life_safety_operation_detailed(
    db: &mut ObjectDatabase,
    request: &LifeSafetyOperationRequest,
) -> Result<LifeSafetyOperationHandlerResult, Error> {
    validate_life_safety_operation(request.request)?;

    if let Some(oid) = request.object_identifier {
        let object = db
            .get_mut(&oid)
            .ok_or_else(|| life_safety_error(ErrorClass::OBJECT, ErrorCode::UNKNOWN_OBJECT))?;
        if is_reset_operation(request.request) && !is_life_safety_object(oid) {
            return Err(life_safety_error(
                ErrorClass::OBJECT,
                ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
            ));
        }
        return match object.apply_life_safety_operation_detailed(request.request)? {
            outcome if outcome.effect == LifeSafetyOperationEffect::Applied => {
                let cov_changes = LifeSafetyCovChange::new(oid, outcome.changed_properties)
                    .into_iter()
                    .collect();
                Ok(LifeSafetyOperationHandlerResult {
                    applied_object_identifiers: vec![oid],
                    cov_changes,
                })
            }
            _ => Ok(LifeSafetyOperationHandlerResult {
                applied_object_identifiers: Vec::new(),
                cov_changes: Vec::new(),
            }),
        };
    }

    let mut object_ids = db.list_objects();
    if is_reset_operation(request.request) {
        object_ids.retain(|oid| is_life_safety_object(*oid));
    }
    object_ids.sort_by_key(|oid| (oid.object_type().to_raw(), oid.instance_number()));
    let attempted = object_ids.len();
    let mut changed = Vec::new();
    let mut cov_changes = Vec::new();
    let mut already_applied = 0usize;
    let mut failed = 0usize;
    for oid in object_ids {
        let Some(object) = db.get_mut(&oid) else {
            continue;
        };
        match object.apply_life_safety_operation_detailed(request.request) {
            Ok(outcome) if outcome.effect == LifeSafetyOperationEffect::Applied => {
                changed.push(oid);
                if let Some(change) = LifeSafetyCovChange::new(oid, outcome.changed_properties) {
                    cov_changes.push(change);
                }
            }
            Ok(_) => already_applied += 1,
            Err(_) => failed += 1,
        }
    }
    tracing::debug!(
        operation = request.request.to_raw(),
        attempted,
        applied = changed.len(),
        already_applied,
        failed,
        "completed all-applicable LifeSafetyOperation"
    );
    Ok(LifeSafetyOperationHandlerResult {
        applied_object_identifiers: changed,
        cov_changes,
    })
}

/// Validate the standard operations accepted by the service.
///
/// `NONE` and reserved/unknown values are rejected. All three reset variants
/// are delegated distinctly to configured built-in Point/Zone executors.
pub fn validate_life_safety_operation(operation: LifeSafetyOperation) -> Result<(), Error> {
    if (LifeSafetyOperation::SILENCE.to_raw()..=LifeSafetyOperation::UNSILENCE_VISUAL.to_raw())
        .contains(&operation.to_raw())
    {
        Ok(())
    } else {
        Err(life_safety_error(
            ErrorClass::OBJECT,
            ErrorCode::VALUE_OUT_OF_RANGE,
        ))
    }
}

fn is_reset_operation(operation: LifeSafetyOperation) -> bool {
    matches!(
        operation,
        LifeSafetyOperation::RESET
            | LifeSafetyOperation::RESET_ALARM
            | LifeSafetyOperation::RESET_FAULT
    )
}

fn is_life_safety_object(oid: ObjectIdentifier) -> bool {
    matches!(
        oid.object_type(),
        ObjectType::LIFE_SAFETY_POINT | ObjectType::LIFE_SAFETY_ZONE
    )
}

pub(crate) fn life_safety_error(class: ErrorClass, code: ErrorCode) -> Error {
    Error::Protocol {
        class: class.to_raw() as u32,
        code: code.to_raw() as u32,
    }
}
