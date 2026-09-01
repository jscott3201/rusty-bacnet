//! Application-owned reset execution for built-in Life Safety objects.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;

use bacnet_types::enums::{
    ErrorClass, ErrorCode, LifeSafetyOperation, LifeSafetyState, SilencedState,
};
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

use crate::traits::LifeSafetyOperationEffect;

use super::{life_safety_error, LifeSafetyPointObject, LifeSafetyZoneObject};

/// Immutable Life Safety Point state supplied to a reset executor.
///
/// Network provenance is deliberately absent. The server-owned authorizer is
/// the authority boundary; this context describes only truthful local state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifeSafetyPointResetContext {
    /// Object receiving the operation.
    pub object_identifier: ObjectIdentifier,
    /// Exact reset variant requested by the peer.
    pub operation: LifeSafetyOperation,
    /// Current `Present_Value`.
    pub present_value: LifeSafetyState,
    /// Current `Tracking_Value`.
    pub tracking_value: LifeSafetyState,
    /// Current `Silenced` value.
    pub silenced: SilencedState,
    /// Current `Operation_Expected`, equal to `operation` when invoked.
    pub operation_expected: LifeSafetyOperation,
}

/// Atomic local-state proposal returned by a Life Safety Point reset executor.
///
/// Omitted properties remain unchanged. No value is inferred from another
/// property, and successful application always clears `Operation_Expected`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LifeSafetyPointResetCommit {
    /// Replacement `Present_Value`, when application truth changed.
    pub present_value: Option<LifeSafetyState>,
    /// Replacement `Tracking_Value`, when application truth changed.
    pub tracking_value: Option<LifeSafetyState>,
    /// Replacement `Silenced`, when application truth changed.
    pub silenced: Option<SilencedState>,
}

/// Immutable Life Safety Zone state supplied to a reset executor.
///
/// Zone `Tracking_Value` is intentionally absent because the built-in Zone
/// object does not model that required property yet. Network provenance is
/// retained only by the server-owned authorization boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifeSafetyZoneResetContext {
    /// Object receiving the operation.
    pub object_identifier: ObjectIdentifier,
    /// Exact reset variant requested by the peer.
    pub operation: LifeSafetyOperation,
    /// Current `Present_Value`.
    pub present_value: LifeSafetyState,
    /// Current `Silenced` value.
    pub silenced: SilencedState,
    /// Current `Operation_Expected`, equal to `operation` when invoked.
    pub operation_expected: LifeSafetyOperation,
}

/// Atomic local-state proposal returned by a Life Safety Zone reset executor.
///
/// Omitted properties remain unchanged. No quiet state or unsilencing is
/// implicit, and successful application always clears `Operation_Expected`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LifeSafetyZoneResetCommit {
    /// Replacement `Present_Value`, when application truth changed.
    pub present_value: Option<LifeSafetyState>,
    /// Replacement `Silenced`, when application truth changed.
    pub silenced: Option<SilencedState>,
}

/// Protocol-classified failure reported by an application reset executor.
///
/// The closed set prevents callbacks from escaping arbitrary protocol errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifeSafetyResetError {
    /// The application does not implement the exact recognized reset variant.
    UnsupportedVariant,
    /// Local physical or application state cannot accept this reset now.
    InvalidOperationInThisState,
    /// Authorization-like denial or an operational executor failure.
    ServiceRequestDenied,
}

/// Synchronous application-owned Life Safety Point reset executor.
///
/// The server invokes this callback while holding the object-database write
/// lock. It must be fast, nonblocking, non-reentrant, and panic-free. Panics are
/// caught where unwinding is supported and fail closed. Because duplicate
/// tracking is bounded and process-local, physical actuation must also be
/// application-idempotent. The callback receives no mutable object reference.
pub type LifeSafetyPointResetExecutor = Arc<
    dyn Fn(&LifeSafetyPointResetContext) -> Result<LifeSafetyPointResetCommit, LifeSafetyResetError>
        + Send
        + Sync,
>;

/// Synchronous application-owned Life Safety Zone reset executor.
///
/// The server invokes this callback while holding the object-database write
/// lock. It must be fast, nonblocking, non-reentrant, and panic-free. Panics are
/// caught where unwinding is supported and fail closed. Because duplicate
/// tracking is bounded and process-local, physical actuation must also be
/// application-idempotent. The callback receives no mutable object reference.
pub type LifeSafetyZoneResetExecutor = Arc<
    dyn Fn(&LifeSafetyZoneResetContext) -> Result<LifeSafetyZoneResetCommit, LifeSafetyResetError>
        + Send
        + Sync,
>;

pub(super) fn is_reset_operation(operation: LifeSafetyOperation) -> bool {
    matches!(
        operation,
        LifeSafetyOperation::RESET
            | LifeSafetyOperation::RESET_ALARM
            | LifeSafetyOperation::RESET_FAULT
    )
}

fn reset_error(error: LifeSafetyResetError) -> Error {
    match error {
        LifeSafetyResetError::UnsupportedVariant => {
            life_safety_error(ErrorCode::VALUE_OUT_OF_RANGE)
        }
        LifeSafetyResetError::InvalidOperationInThisState => {
            life_safety_error(ErrorCode::INVALID_OPERATION_IN_THIS_STATE)
        }
        LifeSafetyResetError::ServiceRequestDenied => Error::Protocol {
            class: ErrorClass::SERVICES.to_raw() as u32,
            code: ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32,
        },
    }
}

fn optional_functionality_error() -> Error {
    life_safety_error(ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED)
}

fn invalid_operation_error() -> Error {
    life_safety_error(ErrorCode::INVALID_OPERATION_IN_THIS_STATE)
}

fn commit_value_error() -> Error {
    life_safety_error(ErrorCode::VALUE_OUT_OF_RANGE)
}

fn valid_life_safety_state(state: LifeSafetyState) -> bool {
    LifeSafetyState::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == state)
        || (256..=65_535).contains(&state.to_raw())
}

fn valid_silenced_state(state: SilencedState) -> bool {
    SilencedState::ALL_NAMED
        .iter()
        .any(|&(_, named)| named == state)
}

impl LifeSafetyPointObject {
    pub(super) fn apply_reset_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationEffect, Error> {
        if !is_reset_operation(operation) || self.operation_expected != operation.to_raw() {
            return Err(invalid_operation_error());
        }
        let executor = self
            .reset_executor
            .clone()
            .ok_or_else(optional_functionality_error)?;
        let context = LifeSafetyPointResetContext {
            object_identifier: self.oid,
            operation,
            present_value: LifeSafetyState::from_raw(self.present_value),
            tracking_value: LifeSafetyState::from_raw(self.tracking_value),
            silenced: SilencedState::from_raw(self.silenced),
            operation_expected: LifeSafetyOperation::from_raw(self.operation_expected),
        };
        let result = match catch_unwind(AssertUnwindSafe(|| executor(&context))) {
            Ok(result) => result,
            Err(_) => return Err(reset_error(LifeSafetyResetError::ServiceRequestDenied)),
        };
        let commit = result.map_err(reset_error)?;

        if commit
            .present_value
            .is_some_and(|value| !valid_life_safety_state(value))
            || commit
                .tracking_value
                .is_some_and(|value| !valid_life_safety_state(value))
            || commit
                .silenced
                .is_some_and(|value| !valid_silenced_state(value))
        {
            return Err(commit_value_error());
        }

        if let Some(value) = commit.present_value {
            self.present_value = value.to_raw();
        }
        if let Some(value) = commit.tracking_value {
            self.tracking_value = value.to_raw();
        }
        if let Some(value) = commit.silenced {
            self.silenced = value.to_raw();
        }
        self.operation_expected = LifeSafetyOperation::NONE.to_raw();
        Ok(LifeSafetyOperationEffect::Applied)
    }
}

impl LifeSafetyZoneObject {
    pub(super) fn apply_reset_operation(
        &mut self,
        operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationEffect, Error> {
        if !is_reset_operation(operation) || self.operation_expected != operation.to_raw() {
            return Err(invalid_operation_error());
        }
        let executor = self
            .reset_executor
            .clone()
            .ok_or_else(optional_functionality_error)?;
        let context = LifeSafetyZoneResetContext {
            object_identifier: self.oid,
            operation,
            present_value: LifeSafetyState::from_raw(self.present_value),
            silenced: SilencedState::from_raw(self.silenced),
            operation_expected: LifeSafetyOperation::from_raw(self.operation_expected),
        };
        let result = match catch_unwind(AssertUnwindSafe(|| executor(&context))) {
            Ok(result) => result,
            Err(_) => return Err(reset_error(LifeSafetyResetError::ServiceRequestDenied)),
        };
        let commit = result.map_err(reset_error)?;

        if commit
            .present_value
            .is_some_and(|value| !valid_life_safety_state(value))
            || commit
                .silenced
                .is_some_and(|value| !valid_silenced_state(value))
        {
            return Err(commit_value_error());
        }

        if let Some(value) = commit.present_value {
            self.present_value = value.to_raw();
        }
        if let Some(value) = commit.silenced {
            self.silenced = value.to_raw();
        }
        self.operation_expected = LifeSafetyOperation::NONE.to_raw();
        Ok(LifeSafetyOperationEffect::Applied)
    }
}
