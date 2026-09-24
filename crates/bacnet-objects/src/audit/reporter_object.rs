//! Reporter property projection, separated to keep audit.rs within the LOC cap.

use super::*;

impl BACnetObject for AuditReporterObject {
    fn audit_reporter_internal(&self) -> Option<&AuditReporterObject> {
        Some(self)
    }

    fn configure_audit_reporter_internal(
        &mut self,
        level: AuditLevel,
        operations: AuditOperationFlags,
        confirmed: bool,
        selectors: Option<Vec<BACnetObjectSelector>>,
        priorities: BACnetPriorityFilter,
        maximum_send_delay: Option<crate::audit::AuditSendDelay>,
    ) -> Result<(), Error> {
        let mut next = self.configuration_internal();
        next.audit_level = level;
        next.auditable_operations = operations;
        next.confirmed = confirmed;
        next.monitored_objects = selectors;
        next.audit_priority_filter = priorities;
        next.maximum_send_delay = maximum_send_delay;
        self.change_configuration(next, None)
    }

    fn audit_reporter_authority_internal(&mut self) -> Option<AuditReporterAuthority<'_>> {
        Some(AuditReporterAuthority(self))
    }
    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn property_metadata(&self) -> Cow<'_, [PropertyMetadata]> {
        reporter_metadata::effective_properties(
            self.configuration_internal().monitored_objects.is_some(),
            self.configuration_internal().maximum_send_delay.is_some(),
        )
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        let configuration = self.configuration_internal();
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => Ok(PropertyValue::CharacterString(
                configuration.description.clone(),
            )),
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
            p if p == PropertyIdentifier::AUDIT_LEVEL => Ok(PropertyValue::Enumerated(
                configuration.audit_level.to_raw(),
            )),
            p if p == PropertyIdentifier::AUDIT_SOURCE_REPORTER => {
                Ok(PropertyValue::Boolean(false))
            }
            p if p == PropertyIdentifier::AUDITABLE_OPERATIONS => {
                let (unused_bits, data) = configuration.auditable_operations.to_bacnet();
                Ok(PropertyValue::BitString { unused_bits, data })
            }
            p if p == PropertyIdentifier::AUDIT_PRIORITY_FILTER => {
                let (unused_bits, data) = configuration.audit_priority_filter.to_bacnet();
                Ok(PropertyValue::BitString { unused_bits, data })
            }
            p if p == PropertyIdentifier::ISSUE_CONFIRMED_NOTIFICATIONS => {
                Ok(PropertyValue::Boolean(self.status.confirmed()))
            }
            p if p == PropertyIdentifier::MONITORED_OBJECTS
                && configuration.monitored_objects.is_some() =>
            {
                let selectors = configuration
                    .monitored_objects
                    .as_ref()
                    .expect("presence checked");
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
            p if matches!(
                p,
                PropertyIdentifier::MAXIMUM_SEND_DELAY | PropertyIdentifier::SEND_NOW
            ) && configuration.maximum_send_delay.is_some() =>
            {
                if array_index.is_some() {
                    return Err(crate::common::property_is_not_an_array_error());
                }
                if p == PropertyIdentifier::SEND_NOW {
                    Ok(PropertyValue::Boolean(self.status.send_now()))
                } else {
                    Ok(PropertyValue::Unsigned(u64::from(
                        configuration.maximum_send_delay.unwrap().seconds(),
                    )))
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
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if matches!(
            property,
            PropertyIdentifier::DESCRIPTION
                | PropertyIdentifier::MAXIMUM_SEND_DELAY
                | PropertyIdentifier::SEND_NOW
        ) {
            return AuditReporterAuthority(self).write_property(property, value, array_index, None);
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
