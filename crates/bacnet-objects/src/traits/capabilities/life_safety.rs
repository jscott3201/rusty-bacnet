//! The Life Safety capability: the LifeSafetyOperation service's effect and
//! the operation an object expects next.

use bacnet_types::enums::{ErrorClass, ErrorCode, LifeSafetyOperation};
use bacnet_types::error::Error;

use crate::traits::LifeSafetyOperationOutcome;

/// A Life Safety Point or Zone: an object a LifeSafetyOperation acts on.
///
/// The object returns itself from
/// [`BACnetObject::life_safety`](crate::traits::BACnetObject::life_safety).
/// Both methods default to OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
/// the error a host also returns for an object that lends none. Not public
/// API yet (#2179).
#[doc(hidden)]
pub trait LifeSafety {
    /// Apply a LifeSafetyOperation atomically to this object.
    ///
    /// Implementations must leave the object unchanged when returning `Err`.
    /// They run synchronously under the object-database write lock and must
    /// be fast, nonblocking, and panic-free. External or irreversible
    /// actuation also needs an application-owned idempotency/replay
    /// contract. Successful implementations return an outcome with exact,
    /// ordered, deduplicated committed property changes. `AlreadyApplied`
    /// leaves all state unchanged and reports no deltas. The bundled server
    /// uses these changes for COV after releasing the database lock and
    /// dispatching the service response.
    fn apply_life_safety_operation(
        &mut self,
        _operation: LifeSafetyOperation,
    ) -> Result<LifeSafetyOperationOutcome, Error> {
        Err(not_supported())
    }

    /// Set the next LifeSafetyOperation expected by trusted local logic.
    ///
    /// This is an application-facing state channel, not a network property
    /// write.
    fn set_life_safety_operation_expected(
        &mut self,
        _operation: LifeSafetyOperation,
    ) -> Result<(), Error> {
        Err(not_supported())
    }
}

/// What the defaults answer: OBJECT / OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED.
fn not_supported() -> Error {
    Error::protocol(
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    )
}
