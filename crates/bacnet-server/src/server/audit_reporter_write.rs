//! Per-element pre-state selection and successful policy-change exceptions.
use super::*;
use bacnet_objects::property_metadata::has_property;
use bacnet_types::error::RemoteError;

pub(super) struct WriteSelection {
    ordinary: bool,
    captured_revision: Option<(Arc<AuditReporterStatus>, u64)>,
    pub(super) change: Option<(
        ObjectIdentifier,
        PropertyIdentifier,
        bacnet_objects::audit::ObjectAuditPolicy,
    )>,
}
impl WriteSelection {
    pub(super) fn selected(&self, db: &ObjectDatabase, success: bool) -> bool {
        if success
            && self
                .captured_revision
                .as_ref()
                .is_some_and(|(status, before)| status.capture_revision() != *before)
        {
            return false;
        }
        self.ordinary
            || (success
                && self.change.is_some_and(|(oid, property, before)| {
                    let Some(object) = db.get(&oid) else {
                        return false;
                    };
                    let after = object
                        .audit_policy()
                        .map(|policy| policy.object_policy())
                        .unwrap_or_default();
                    match property {
                        PropertyIdentifier::AUDIT_LEVEL => before.level != after.level,
                        PropertyIdentifier::AUDITABLE_OPERATIONS => {
                            before.operations != after.operations
                        }
                        _ => false,
                    }
                }))
    }
}

impl<T: TransportPort + 'static> WriteCommitObserver for WriteAudit<'_, T> {
    fn before(&mut self, db: &ObjectDatabase, write: WriteTarget<'_>) {
        self.pending = None;
        let Some((selected_reporter, reporter_attempt)) = self.select_write(db, write.oid) else {
            return;
        };
        let reporter = &selected_reporter.configuration;
        let device = crate::local_device::initiating_device(&self.config.device);
        let route = recipient(db, device)
            .and_then(|value| self.transactions.audit_routes.get()?.resolve(&value));
        let status = Arc::clone(&selected_reporter.status);
        status.set_configured(route.is_some());
        let Some(object) = db.get(&write.oid) else {
            return;
        };
        // Standard commandable properties use Present_Value + Priority_Array.
        // A Channel's Present_Value is commandable too (Clause 12.53.5) with
        // no Priority_Array, since it passes the priority on to its members,
        // and Audit_Priority_Filter applies to an object's commandable
        // property (Clause 19.6.3): its record carries the priority and a
        // disabled priority drops it, for a WriteProperty and a WriteGroup
        // alike. A supplied priority on Description (etc.) must never filter
        // the write.
        let command_priority = (write.property == PropertyIdentifier::PRESENT_VALUE
            && (write.oid.object_type() == ObjectType::CHANNEL
                || has_property(
                    &object.property_metadata(),
                    PropertyIdentifier::PRIORITY_ARRAY,
                )))
        .then_some(write.priority.unwrap_or(16));
        let object_policy = object
            .audit_policy()
            .map(|policy| policy.object_policy())
            .unwrap_or_default();
        let policy = object_policy.effective_internal(reporter);
        let ordinary = if reporter_attempt {
            true
        } else {
            policy.reports(
                AuditOperation::WRITE,
                Some(write.property),
                command_priority,
            )
        };
        let mandatory = policy.reporter_enabled
            && match write.property {
                PropertyIdentifier::AUDIT_LEVEL => object_policy.level.is_some(),
                PropertyIdentifier::AUDITABLE_OPERATIONS => {
                    object_policy.operations.is_some() && policy.level != AuditLevel::NONE
                }
                _ => false,
            };
        if !ordinary && !mandatory {
            return;
        }
        let selection = WriteSelection {
            ordinary,
            captured_revision: object
                .audit_reporter()
                .filter(|r| r.captures_changes_internal())
                .map(|r| {
                    let status = r.status_internal();
                    let revision = status.capture_revision();
                    (status, revision)
                }),
            change: mandatory.then_some((write.oid, write.property, object_policy)),
        };
        // An index on a scalar reads the whole value, as before objects
        // refused that index themselves (#2274).
        let read_index = write
            .array_index
            .filter(|_| object.is_array_property(write.property));
        let current_value = object
            .read_property(write.property, read_index)
            .ok()
            .and_then(|value| small_value(&value));
        self.pending = Some(Box::new(PendingWrite {
            delay: (!reporter_attempt)
                .then_some(reporter.maximum_send_delay)
                .flatten(),
            selection: Some(selection),
            failure: self.failure_ticket(&status, reporter.confirmed, device, route.clone()),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed,
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation: AuditOperation::WRITE,
                source_comment: None,
                target_comment: None,
                invoke_id: self.invoke_id,
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(device),
                target_object: Some(write.oid),
                target_property: Some(AuditPropertyReference {
                    property_identifier: write.property,
                    property_array_index: write.array_index.map(u64::from),
                }),
                target_priority: command_priority,
                target_value: (write.value.len() <= 32).then(|| write.value.to_vec()),
                current_value,
                result: None,
            },
        }));
    }

    fn commit_policy(
        &mut self,
        db: &mut ObjectDatabase,
        write: WriteTarget<'_>,
        value: &PropertyValue,
    ) -> Option<Result<(), Error>> {
        self.commit_mandatory_policy(db, write, value)
    }

    fn committed(&mut self, db: &mut ObjectDatabase) {
        self.complete(db, None);
    }

    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error) {
        // An unknown transaction outcome is not an execution failure. In
        // particular, never manufacture an Error for a timeout/Reject/Abort.
        if matches!(
            error,
            Error::Timeout(_)
                | Error::Remote(RemoteError::Reject { .. })
                | Error::Remote(RemoteError::Abort { .. })
        ) {
            self.pending = None;
            return;
        }
        self.complete(db, Some(bacnet_device_core::reply::error_fields(error)));
    }
}
