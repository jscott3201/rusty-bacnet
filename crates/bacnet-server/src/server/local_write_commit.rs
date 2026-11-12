//! Every local write's object step except a WriteGroup's: bacnet-device-core's
//! [`apply`](bacnet_device_core::write::local::apply) (#2446), with the
//! server's Audit around a property write and what the write staged released
//! as the object step ends, ahead of Audit's record of it.

use super::*;
use crate::server::durable_writes::HeldStage;
use bacnet_device_core::write::local::{self, LocalChange, Observed};
use bacnet_objects::traits::LocalInputValue;

/// The change `write` makes with `value`, for bacnet-device-core.
fn change(write: LocalWrite<'_>, value: PropertyValue) -> LocalChange {
    let input = match write {
        LocalWrite::Property {
            property,
            array_index,
            priority,
            source,
        } => {
            return LocalChange::Property {
                property,
                array_index,
                value,
                priority,
                source,
            };
        }
        LocalWrite::ApplicationPresentValue => LocalInputValue::present_value(value),
        LocalWrite::ApplicationControlledVariableValue => {
            LocalInputValue::ControlledVariableValue(value)
        }
        LocalWrite::ApplicationAveragingSample => LocalInputValue::AveragingSample(Some(value)),
        LocalWrite::ApplicationAveragingMiss => LocalInputValue::AveragingSample(None),
        LocalWrite::ApplicationTrackingValue => LocalInputValue::TrackingValue(value),
        LocalWrite::ApplicationProgramState => LocalInputValue::ProgramState(value),
        LocalWrite::ApplicationAccessInput(input) => LocalInputValue::AccessControl(input.clone()),
        LocalWrite::ApplicationPulses(pulses) => LocalInputValue::Pulses(pulses),
        LocalWrite::ApplicationReliability(reliability) => {
            return LocalChange::Reliability(reliability);
        }
        LocalWrite::WriteGroup { .. } => unreachable!("a WriteGroup commits in the server"),
    };
    LocalChange::Input(input)
}

/// A local property write's observer: its Audit, unless the Device's
/// recipient sink owns the write's record, and what the write staged, which
/// it releases as the object step ends.
struct LocalObserver<'a, T: TransportPort> {
    audit: Option<audit_reporter::WriteAudit<'a, T>>,
    staged: Option<HeldStage>,
}

impl<T: TransportPort> LocalObserver<'_, T> {
    /// Release what the write staged, once.
    fn release(&mut self, db: &mut ObjectDatabase) {
        if let Some(staged) = self.staged.take() {
            staged.release(db);
        }
    }
}

impl<T: TransportPort + 'static> WriteCommitObserver for LocalObserver<'_, T> {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>) {
        if let Some(audit) = &mut self.audit {
            audit.before(db, write);
        }
    }

    fn commit_policy(
        &mut self,
        db: &mut ObjectDatabase,
        write: WriteTarget<'_>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.audit.as_mut()?.commit_policy(db, write, value)
    }

    fn committed(&mut self, db: &mut ObjectDatabase) {
        self.release(db);
        if let Some(audit) = &mut self.audit {
            audit.committed(db);
        }
    }

    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error) {
        self.release(db);
        if let Some(audit) = &mut self.audit {
            audit.failed(db, error);
        }
    }
}

impl<T: TransportPort + 'static> LocalWriter<'_, T> {
    /// Commit `write` of `value` to `oid` under the write's guard: every
    /// local write but a WriteGroup's. A property write is audited, unless
    /// it is the Device's recipient sink's, which exclusively owns its
    /// old/new pair; an application input or Reliability is not. What was staged goes back
    /// as the object step ends, or once a check refuses the write first.
    pub(super) fn commit_local(
        &self,
        db: &mut ObjectDatabase,
        oid: &ObjectIdentifier,
        write: LocalWrite<'_>,
        value: PropertyValue,
        staged: HeldStage,
    ) -> Result<Applied, Error> {
        let audit = match write {
            LocalWrite::Property { property, .. } => {
                let recipient_sink = property == PropertyIdentifier::AUDIT_NOTIFICATION_RECIPIENT
                    && db
                        .get_mut(oid)
                        .and_then(|object| object.device_authority_internal())
                        .is_some();
                (!recipient_sink).then(|| {
                    audit_reporter::WriteAudit::local(
                        self.config,
                        self.network,
                        self.notification_transactions,
                    )
                })
            }
            _ => None,
        };
        let octets = if audit.is_some() {
            audit_reporter::small_value(&value).unwrap_or_default()
        } else {
            Vec::new()
        };
        let property = matches!(write, LocalWrite::Property { .. });
        let mut observer = LocalObserver {
            audit,
            staged: Some(staged),
        };
        let observed = property.then(|| Observed {
            observer: &mut observer,
            octets: &octets,
        });
        let applied = local::apply(db, *oid, change(write, value), observed);
        // Not yet released: a check refused the write before the object
        // step, or it was an input, which reports to no observer.
        observer.release(db);
        applied
    }
}
