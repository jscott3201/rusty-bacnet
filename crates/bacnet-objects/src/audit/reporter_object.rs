//! Reporter property projection, separated to keep audit.rs within the LOC cap.

use super::*;

/// Opaque, one-use authorization from database-wide source validation.
/// No public constructor or general mutable source flag is exposed.
#[doc(hidden)]
pub struct SourceReporterBinding {
    oid: ObjectIdentifier,
}

impl AuditReporterObject {
    /// Designate the sole source Reporter under exclusive pre-start DB ownership.
    /// The endpoint separately validates its client role and local Device. This
    /// changes ownership only, never filters, destinations, records or delivery.
    #[doc(hidden)]
    pub fn designate_source_internal(
        db: &mut crate::database::ObjectDatabase,
        selected: ObjectIdentifier,
    ) -> Result<(), Error> {
        if selected.object_type() != ObjectType::AUDIT_REPORTER {
            return Err(Error::Encoding(
                "source selection must be an Audit Reporter".into(),
            ));
        }
        let object = db.get(&selected).ok_or_else(|| {
            Error::Encoding("selected source Audit Reporter is absent from the database".into())
        })?;
        if !object
            .audit_reporter_internal()
            .is_some_and(|reporter| reporter.oid == selected)
        {
            return Err(Error::Encoding(
                "selected object lacks the Audit Reporter capability".into(),
            ));
        }
        // Include downstream Reporters without our capability: their visible
        // source property must also rule out a conflict. Unknown state fails closed.
        for (oid, object) in db.iter_objects() {
            if oid != selected && oid.object_type() == ObjectType::AUDIT_REPORTER {
                match object.read_property(PropertyIdentifier::AUDIT_SOURCE_REPORTER, None) {
                    Ok(PropertyValue::Boolean(false)) => {}
                    Ok(PropertyValue::Boolean(true)) => {
                        return Err(Error::Encoding(
                            "database already contains a conflicting source Audit Reporter".into(),
                        ))
                    }
                    _ => {
                        return Err(Error::Encoding(
                            "cannot determine another Audit Reporter's source ownership".into(),
                        ))
                    }
                }
            }
        }
        // No mutation until every database-wide check has passed. The hook is
        // opt-in and atomic on failure; the built-in Reporter only sets one bit.
        db.get_mut(&selected)
            .expect("selected Reporter was validated under exclusive database ownership")
            .bind_audit_source_internal(SourceReporterBinding { oid: selected })
    }
}

impl BACnetObject for AuditReporterObject {
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        Some(self)
    }

    fn bind_audit_source_internal(&mut self, binding: SourceReporterBinding) -> Result<(), Error> {
        if binding.oid != self.oid {
            return Err(Error::Encoding(
                "source binding belongs to another Audit Reporter".into(),
            ));
        }
        self.source_reporter = true;
        Ok(())
    }

    fn is_deleteable(&self) -> bool {
        !self.source_reporter
    }

    fn configure_audit_reporter_internal(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
    ) -> Result<(), Error> {
        // The only fallible setter validates before mutation; the remaining
        // settings are already typed and cannot fail.
        self.set_audit_level(level)?;
        self.set_auditable_operations(operations);
        self.set_issue_confirmed_notifications(confirmed);
        Ok(())
    }

    fn configure_audit_reporter_with_filters_internal(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
        selectors: Option<Vec<BACnetObjectSelector>>,
        priorities: BACnetPriorityFilter,
    ) -> Result<(), Error> {
        // The legacy hook validates before mutation; typed filters cannot fail.
        self.configure_audit_reporter_internal(level, operations, confirmed)?;
        self.set_monitored_objects(selectors);
        self.set_audit_priority_filter(priorities);
        Ok(())
    }

    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        reporter_metadata::effective_properties(self.monitored_objects.is_some())
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::AUDIT_REPORTER.to_raw(),
            )),
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![if self.reliability() == Reliability::NO_FAULT_DETECTED {
                    0
                } else {
                    0x40
                }],
            }),
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability().to_raw()))
            }
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(EventState::NORMAL.to_raw()))
            }
            p if p == PropertyIdentifier::AUDIT_LEVEL => {
                Ok(PropertyValue::Enumerated(self.audit_level.to_raw()))
            }
            p if p == PropertyIdentifier::AUDIT_SOURCE_REPORTER => {
                Ok(PropertyValue::Boolean(self.source_reporter))
            }
            p if p == PropertyIdentifier::AUDITABLE_OPERATIONS => {
                let (unused_bits, data) = self.auditable_operations.to_bacnet();
                Ok(PropertyValue::BitString { unused_bits, data })
            }
            p if p == PropertyIdentifier::AUDIT_PRIORITY_FILTER => {
                let (unused_bits, data) = self.audit_priority_filter.to_bacnet();
                Ok(PropertyValue::BitString { unused_bits, data })
            }
            p if p == PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS => {
                Ok(PropertyValue::Boolean(self.issue_confirmed_notifications))
            }
            p if p == PropertyIdentifier::MONITORED_OBJECTS && self.monitored_objects.is_some() => {
                let selectors = self.monitored_objects.as_ref().expect("presence checked");
                match array_index {
                    None => Ok(PropertyValue::List(
                        selectors
                            .iter()
                            .map(BACnetObjectSelector::encode_property_value)
                            .collect(),
                    )),
                    Some(0) => Ok(PropertyValue::Unsigned(selectors.len() as u64)),
                    Some(index) => selectors
                        .get((index - 1) as usize)
                        .map(BACnetObjectSelector::encode_property_value)
                        .ok_or_else(crate::common::invalid_array_index_error),
                }
            }
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(s) = value {
                self.description = s;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(&self.property_metadata())
    }
}

#[cfg(test)]
#[path = "source_reporter_tests.rs"]
mod source_reporter_tests;
