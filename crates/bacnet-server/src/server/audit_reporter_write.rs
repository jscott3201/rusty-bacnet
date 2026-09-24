//! Per-element pre-state selection and successful policy-change exceptions.
use super::*;

pub(super) struct WriteSelection {
    ordinary: bool,
    change: Option<(
        ObjectIdentifier,
        PropertyIdentifier,
        bacnet_objects::audit::ObjectAuditPolicy,
    )>,
}
impl WriteSelection {
    pub(super) fn selected(&self, db: &ObjectDatabase, success: bool) -> bool {
        self.ordinary
            || (success
                && self.change.is_some_and(|(oid, property, before)| {
                    let Some(object) = db.get(&oid) else {
                        return false;
                    };
                    let after = object.audit_object_policy_internal();
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
        let Some(profile) = &self.config.audit_reporter else {
            return;
        };
        let Some(reporter) = db
            .get(&profile.reporter)
            .and_then(|object| object.audit_reporter_internal())
        else {
            return;
        };
        let device = local_device(db);
        let route = device
            .and_then(|device| recipient(db, device))
            .and_then(|value| self.transactions.audit_routes.get()?.resolve(&value));
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && route.is_some());
        let Some(device) = device else { return };
        let Some(object) = db.get(&write.oid) else {
            return;
        };
        // Standard commandable properties use Present_Value + Priority_Array.
        // A supplied priority on Description (etc.) must never filter the write.
        let command_priority = (write.property == PropertyIdentifier::PRESENT_VALUE
            && object
                .property_list()
                .contains(&PropertyIdentifier::PRIORITY_ARRAY))
        .then_some(write.priority.unwrap_or(16));
        let object_policy = object.audit_object_policy_internal();
        let policy = object_policy.effective_internal(reporter);
        let ordinary = if write.oid.object_type() == ObjectType::AUDIT_REPORTER {
            reporter.reports_write_internal(write.property, command_priority, true)
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
        if !reporter.monitors_object_internal(write.oid) || (!ordinary && !mandatory) {
            return;
        }
        let selection = WriteSelection {
            ordinary,
            change: mandatory.then_some((write.oid, write.property, object_policy)),
        };
        let current_value = object
            .read_property(write.property, write.array_index)
            .ok()
            .and_then(|value| small_value(&value));
        self.pending = Some(PendingWrite {
            selection: Some(selection),
            failure: self.failure_ticket(
                &status,
                reporter.confirmed_internal(),
                device,
                route.clone(),
            ),
            route,
            completion: status.begin_delivery(),
            status,
            confirmed: reporter.confirmed_internal(),
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
                target_value: (!write.value.is_empty() && write.value.len() <= 32)
                    .then(|| write.value.to_vec()),
                current_value,
                result: None,
            },
        });
    }

    fn committed(&mut self, db: &mut ObjectDatabase) {
        self.complete(db, None);
    }

    fn failed(&mut self, db: &mut ObjectDatabase, error: &Error) {
        // An unknown transaction outcome is not an execution failure. In
        // particular, never manufacture an Error for a timeout/Reject/Abort.
        if matches!(
            error,
            Error::Timeout(_) | Error::Reject { .. } | Error::Abort { .. }
        ) {
            self.pending = None;
            return;
        }
        self.complete(
            db,
            Some(super::requests::confirmed_response::error_fields(error)),
        );
    }
}
