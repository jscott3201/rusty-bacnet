//! The fault pass: each object's opt-in reliability evaluation
//! ([`ReliabilityEval`](bacnet_objects::traits::ReliabilityEval)), visited
//! once. A host evaluates each object whose Reliability changed as after a
//! write ([`after_write`](super::after_write)), which commits the FAULT
//! transition, or the return from it, that the change causes (Clause
//! 13.2.2), then fans COV out for it (#2535); a [`sweep`](super::sweep)
//! would find the change too, later.

use alloc::vec::Vec;
use bacnet_objects::database::ObjectDatabase;
use bacnet_objects::traits::ReliabilityEvaluation;
use bacnet_types::enums::Reliability;
use bacnet_types::error::Error;
use bacnet_types::primitives::ObjectIdentifier;

/// A reliability change detected by the fault detector.
#[derive(Debug, Clone, PartialEq)]
pub struct ReliabilityChange {
    /// The object whose reliability changed.
    pub object_id: ObjectIdentifier,
    /// Previous reliability value.
    pub old_reliability: Reliability,
    /// New reliability value.
    pub new_reliability: Reliability,
}

/// What one [`fault_pass`] found.
#[derive(Debug, Default)]
pub struct FaultPass {
    /// The changes object hooks applied, in visiting order.
    pub changes: Vec<ReliabilityChange>,
    /// The objects whose hook failed this pass, with its error, in visiting
    /// order. Each made no change; a host decides what to log.
    pub failures: Vec<(ObjectIdentifier, Error)>,
}

/// Visit every object's opt-in reliability evaluation once.
///
/// An object that lends no evaluation, or whose evaluation is inhibited, is
/// left alone. Only a hook result of [`ReliabilityEvaluation::Changed`]
/// produces a [`ReliabilityChange`].
pub fn fault_pass(db: &mut ObjectDatabase) -> FaultPass {
    let mut pass = FaultPass::default();
    db.for_each_object_mut(|oid, object| {
        // An object that lends no evaluation changes nothing: the old
        // `Unchanged` default.
        let Some(evaluation) = object.reliability_eval() else {
            return;
        };
        if evaluation.reliability_evaluation_inhibited() {
            return;
        }
        match evaluation.evaluate_reliability() {
            Ok(ReliabilityEvaluation::Unchanged) => {}
            Ok(ReliabilityEvaluation::Changed {
                old_reliability,
                new_reliability,
            }) => {
                pass.changes.push(ReliabilityChange {
                    object_id: oid,
                    old_reliability,
                    new_reliability,
                });
            }
            Err(error) => pass.failures.push((oid, error)),
        }
    });
    pass
}
