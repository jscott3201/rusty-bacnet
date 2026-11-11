//! The reliability-evaluation capability: an object's own judgement of its
//! Reliability, and the Reliability its application works out for it.

use bacnet_types::enums::Reliability;
use bacnet_types::error::Error;

use super::not_supported;
use crate::traits::ReliabilityEvaluation;

/// An object that evaluates its own Reliability, or takes a Reliability its
/// application works out.
///
/// The object returns itself from
/// [`BACnetObject::reliability_eval`](crate::traits::BACnetObject::reliability_eval).
/// The bundled server's fault detector visits every object every ten
/// seconds while it holds the database's write guard: it skips an object
/// whose evaluation is inhibited, otherwise calls
/// [`evaluate_reliability`](Self::evaluate_reliability), and reports each
/// [`ReliabilityEvaluation::Changed`] as a Reliability change (which an
/// intrinsic reporter may turn into a fault transition). Clause 3.2
/// describes reliability evaluation as the object's assessment of its own
/// reliability.
///
/// Every method has a default that opts out, so an object overrides only
/// what it models. Among the built-in objects, the Analog, Binary and
/// Multi-state Inputs, Outputs and Values, the Integer Value, Loop,
/// Schedule, Life Safety Point and Zone, Access Zone, Credential Data Input
/// and Accumulator lend it.
///
/// Reliability has two owners. While Out_Of_Service is TRUE a client may
/// write Reliability to simulate a fault, and the object's own evaluation
/// leaves that value alone; while it is FALSE, Reliability is the object's,
/// and only this capability changes it.
pub trait ReliabilityEval {
    /// Evaluate and, when necessary, change this object's `Reliability`.
    ///
    /// The object decides whether and how its internal conditions affect
    /// Reliability, such as a FAULT_OUT_OF_RANGE algorithm or a Schedule's
    /// consistency check, and makes the change itself. It returns
    /// [`ReliabilityEvaluation::Changed`] with the old and new values only
    /// once that change succeeded, and [`ReliabilityEvaluation::Unchanged`]
    /// when it changed nothing, including while Out_Of_Service is TRUE.
    ///
    /// An `Err` must leave every value unchanged; the bundled server logs it
    /// once per distinct error and reports no change. The default changes
    /// nothing.
    fn evaluate_reliability(&mut self) -> Result<ReliabilityEvaluation, Error> {
        Ok(ReliabilityEvaluation::Unchanged)
    }

    /// Whether periodic evaluation is inhibited now, through the optional
    /// Reliability_Evaluation_Inhibit property. The bundled server then
    /// skips [`evaluate_reliability`](Self::evaluate_reliability) for the
    /// object; reporting a Reliability change already made is unaffected.
    /// The default is `false`.
    fn reliability_evaluation_inhibited(&self) -> bool {
        false
    }

    /// Apply a `Reliability` worked out inside the device, such as a
    /// hardware integration's NO_SENSOR, apart from the network's
    /// [`write_property`](crate::traits::BACnetObject::write_property).
    ///
    /// The route is the in-service complement of a client's write: the
    /// built-in objects refuse it with PROPERTY / WRITE_ACCESS_DENIED while
    /// Out_Of_Service is TRUE (and while Reliability_Evaluation_Inhibit is
    /// TRUE, where they model it), and with PROPERTY / VALUE_OUT_OF_RANGE for
    /// a value outside BACnetReliability. A Reliability applied this way
    /// stays until this route changes it again; the object's own evaluation
    /// does not clear it. An `Err` leaves Reliability unchanged. The default
    /// refuses with [`not_supported`].
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
