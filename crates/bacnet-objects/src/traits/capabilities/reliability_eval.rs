//! The reliability-evaluation capability: an object's own judgement of its
//! Reliability, and the Reliability its application works out for it.

use bacnet_types::enums::Reliability;
use bacnet_types::error::Error;

use super::not_supported;
use crate::traits::ReliabilityEvaluation;

/// An object that evaluates its own Reliability, or takes one its
/// application derives.
///
/// The object returns itself from
/// [`BACnetObject::reliability_eval`](crate::traits::BACnetObject::reliability_eval).
/// The bundled server's fault detector visits every object under the
/// database's write guard; it skips an object whose evaluation is
/// inhibited and otherwise evaluates it. Clause 3.2 describes reliability
/// evaluation as the object's assessment of its own reliability.
///
/// Every method has a default that opts out, so an object overrides only
/// what it models. Not public API yet (#2179).
#[doc(hidden)]
pub trait ReliabilityEval {
    /// Evaluate and, when necessary, change this object's `Reliability`.
    ///
    /// The object decides whether and how its internal conditions affect
    /// Reliability and makes the change itself, returning
    /// [`ReliabilityEvaluation::Changed`] only once that change succeeded;
    /// [`ReliabilityEvaluation::Unchanged`] means it changed nothing. An
    /// `Err` leaves every value unchanged. The default changes nothing.
    fn evaluate_reliability(&mut self) -> Result<ReliabilityEvaluation, Error> {
        Ok(ReliabilityEvaluation::Unchanged)
    }

    /// Whether periodic evaluation is inhibited now, through the optional
    /// Reliability_Evaluation_Inhibit property. Reporting a Reliability
    /// change already made is unaffected. The default is `false`.
    fn reliability_evaluation_inhibited(&self) -> bool {
        false
    }

    /// Apply a `Reliability` worked out inside the device, apart from the
    /// network's [`write_property`](crate::traits::BACnetObject::write_property).
    ///
    /// Ownership is symmetric: a client may write Reliability while
    /// Out_Of_Service is TRUE, and this route only while it is FALSE. The
    /// default refuses with [`not_supported`].
    fn set_reliability(&mut self, _reliability: Reliability) -> Result<(), Error> {
        Err(not_supported())
    }
}

/// The accessor of an object that lends itself as its [`ReliabilityEval`],
/// for inside its `BACnetObject` impl.
macro_rules! lend_reliability_eval {
    () => {
        fn reliability_eval(&mut self) -> Option<&mut dyn $crate::traits::ReliabilityEval> {
            Some(self)
        }
    };
}

pub(crate) use lend_reliability_eval;
