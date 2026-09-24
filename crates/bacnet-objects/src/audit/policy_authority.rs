//! Concrete AV/BV policy preparation; no generic object callbacks during commit.
use super::ObjectAuditPolicy;
use bacnet_types::{enums::PropertyIdentifier, error::Error, primitives::PropertyValue};

/// Borrowed built-in policy field. Only this crate can construct the authority.
#[doc(hidden)]
pub struct AuditPolicyAuthority<'a>(&'a mut ObjectAuditPolicy);

/// A validated candidate tied to the original built-in field until assignment.
#[doc(hidden)]
pub struct PreparedAuditPolicyWrite<'a> {
    target: &'a mut ObjectAuditPolicy,
    next: ObjectAuditPolicy,
}

impl<'a> AuditPolicyAuthority<'a> {
    pub(crate) fn new(policy: &'a mut ObjectAuditPolicy) -> Self {
        Self(policy)
    }

    /// Validate with the ordinary writer on a copy. Only actual changes to the
    /// two mandatory setting properties yield a prepared assignment.
    pub fn prepare(
        self,
        property: PropertyIdentifier,
        index: Option<u32>,
        value: &PropertyValue,
        priority: Option<u8>,
    ) -> Result<Option<PreparedAuditPolicyWrite<'a>>, Error> {
        if !matches!(
            property,
            PropertyIdentifier::AUDIT_LEVEL | PropertyIdentifier::AUDITABLE_OPERATIONS
        ) {
            return Ok(None);
        }
        let mut next = *self.0;
        next.write(property, index, value, priority)
            .expect("policy property")?;
        Ok((next != *self.0).then_some(PreparedAuditPolicyWrite {
            target: self.0,
            next,
        }))
    }
}

impl PreparedAuditPolicyWrite<'_> {
    /// Assign the validated policy without allocation, callbacks or fallible work.
    pub fn commit(self) {
        *self.target = self.next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bacnet_types::{bitstring::AuditOperationFlags, enums::AuditLevel};

    #[test]
    fn mandatory_policy_candidate_preserves_validation_and_drop_atomicity() {
        for present in [false, true] {
            let initial = ObjectAuditPolicy {
                level: present.then_some(AuditLevel::NONE),
                operations: present.then_some(AuditOperationFlags::empty()),
                ..Default::default()
            };
            for (property, value) in [
                (
                    PropertyIdentifier::AUDIT_LEVEL,
                    PropertyValue::Enumerated(AuditLevel::AUDIT_ALL.to_raw()),
                ),
                (PropertyIdentifier::AUDIT_LEVEL, PropertyValue::Null),
                (
                    PropertyIdentifier::AUDIT_LEVEL,
                    PropertyValue::Boolean(true),
                ),
                (
                    PropertyIdentifier::AUDITABLE_OPERATIONS,
                    PropertyValue::BitString {
                        unused_bits: 6,
                        data: vec![0xc0],
                    },
                ),
                (
                    PropertyIdentifier::AUDITABLE_OPERATIONS,
                    PropertyValue::BitString {
                        unused_bits: 3,
                        data: vec![1],
                    },
                ),
                (
                    PropertyIdentifier::AUDITABLE_OPERATIONS,
                    PropertyValue::Null,
                ),
            ] {
                for index in [None, Some(0), Some(1)] {
                    for priority in [None, Some(0), Some(1), Some(16), Some(17)] {
                        let mut ordinary = initial;
                        let expected = ordinary.write(property, index, &value, priority).unwrap();
                        let mut actual = initial;
                        let prepared = AuditPolicyAuthority::new(&mut actual)
                            .prepare(property, index, &value, priority);
                        assert_eq!(
                            prepared.as_ref().err().map(ToString::to_string),
                            expected.as_ref().err().map(ToString::to_string)
                        );
                        if let Ok(candidate) = prepared {
                            assert_eq!(candidate.is_some(), ordinary != initial);
                        }
                        assert_eq!(actual, initial);
                        if expected.is_ok() {
                            if let Some(candidate) = AuditPolicyAuthority::new(&mut actual)
                                .prepare(property, index, &value, priority)
                                .unwrap()
                            {
                                candidate.commit();
                            }
                            assert_eq!(actual, ordinary);
                        }
                    }
                }
            }
        }
    }
}
