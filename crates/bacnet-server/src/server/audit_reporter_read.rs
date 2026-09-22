//! Value-free read intents. No delivery admission occurs under a read guard.
use super::*;

pub(in crate::server) struct ReadAuditIntent(PendingWrite);

impl<T: TransportPort + 'static> WriteAudit<'_, T> {
    pub(in crate::server) fn read_intent(
        &self,
        db: &ObjectDatabase,
        target: ObjectIdentifier,
        property: PropertyIdentifier,
        index: Option<u32>,
        result: Option<(ErrorClass, ErrorCode)>,
    ) -> Option<ReadAuditIntent> {
        let profile = self.config.audit_reporter.as_ref()?;
        let reporter = db.get(&profile.reporter)?.audit_reporter_internal()?;
        let device = local_device(db);
        let status = reporter.status_internal();
        status.set_configured(device.is_some() && self.route.is_some());
        let device = device?;
        // Reuse the existing local property classification: Present_Value is
        // operational, other properties configuration, proprietary levels ALL.
        // The mandatory Reporter WRITE bypass does not apply to READ. Target
        // selection still uses the existing Reporter/Monitored_Objects rules.
        let level = match reporter.read_property(PropertyIdentifier::AUDIT_LEVEL, None) {
            Ok(PropertyValue::Enumerated(level)) => AuditLevel::from_raw(level),
            _ => return None,
        };
        let operations =
            match reporter.read_property(PropertyIdentifier::AUDITABLE_OPERATIONS, None) {
                Ok(PropertyValue::BitString { unused_bits, data }) => {
                    AuditOperationFlags::from_bacnet(unused_bits, &data).ok()?
                }
                _ => return None,
            };
        if level == AuditLevel::NONE
            || (level == AuditLevel::AUDIT_CONFIG && property == PropertyIdentifier::PRESENT_VALUE)
            || !operations.contains(AuditOperation::READ)
            || !reporter.monitors_object_internal(target)
        {
            return None;
        }
        Some(ReadAuditIntent(PendingWrite {
            status,
            confirmed: reporter.confirmed_internal(),
            notification: BACnetAuditNotification {
                source_timestamp: None,
                target_timestamp: None,
                source_device: self.source.clone(),
                source_object: None,
                operation: AuditOperation::READ,
                source_comment: None,
                target_comment: None,
                invoke_id: Some(self.invoke_id),
                source_user_id: None,
                source_user_role: None,
                target_device: BACnetRecipient::Device(device),
                target_object: Some(target),
                target_property: Some(AuditPropertyReference {
                    property_identifier: property,
                    property_array_index: index.map(u64::from),
                }),
                target_priority: None,
                target_value: None,
                current_value: None,
                result,
            },
        }))
    }

    /// Only completed, unsegmented response paths may submit these intents.
    /// The batch is bounded by RPM's result budget (one intent for RP).
    pub(in crate::server) async fn admit_reads(
        &self,
        db: &RwLock<ObjectDatabase>,
        mut intents: Vec<ReadAuditIntent>,
    ) {
        if intents.is_empty() || self.route.is_none() {
            return;
        }
        {
            // Clockless timestamps consume the existing DB-local sequence.
            // Release this short write guard too before admission/encoding.
            let mut db = db.write().await;
            for intent in &mut intents {
                intent.0.notification.target_timestamp =
                    Some(super::super::event_timestamp::sample_event_timestamp(&mut db).timestamp);
            }
        }
        for intent in intents {
            self.admit(intent.0);
        }
    }
}
