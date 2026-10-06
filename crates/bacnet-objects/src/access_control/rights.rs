use bacnet_encoding::constructed::encode_access_rule;
use bacnet_types::constructed::BACnetAccessRule;
use bacnet_types::enums::{
    AccessRuleLocationSpecifier, AccessRuleTimeRangeSpecifier, ErrorClass, ErrorCode,
};

use super::*;
use crate::durable::DurableWrites;

#[path = "rights_persistence.rs"]
mod persistence;
#[path = "rights_saving.rs"]
mod saving;
pub use persistence::{AccessRightsPersistence, AccessRightsSnapshot, FileAccessRightsPersistence};

// AccessRightsObject (type 34)
// ---------------------------------------------------------------------------

/// The most rules either rule array holds, a resource cap. A longer list,
/// from a setter or a network write, is RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY.
pub const MAX_ACCESS_RULES: usize = 1024;

/// BACnet Access Rights object (type 34).
///
/// Holds the positive and negative access rules that credentials and users
/// are assigned. Both rule arrays are BACnetARRAYs of `BACnetAccessRule`.
/// The application provisions them with
/// [`set_positive_access_rules`](Self::set_positive_access_rules) and
/// [`set_negative_access_rules`](Self::set_negative_access_rules), and peers
/// write them whole, one element at a time, or resize them at index 0, with
/// the setters' checks. Enable (property 133, `LOG_ENABLE` in
/// `PropertyIdentifier`) switches the whole object: while it is FALSE every
/// rule in both arrays counts as disabled (Clause 12.34.8). The optional
/// Accompaniment row (#1393) is served once the application sets it with
/// [`set_accompaniment`](Self::set_accompaniment). The object stores the
/// rules, the flag and the accompaniment without evaluating them; the
/// application checks a credential against the rules with
/// [`evaluate_access_rights`] (#1331).
///
/// # Restarts
///
/// An object built with [`new`](Self::new) keeps everything in memory, so a
/// restart brings back what the application configures. One built with
/// [`with_persistence`](Self::with_persistence) saves each write of the two
/// arrays, Enable and Accompaniment in an [`AccessRightsPersistence`] before
/// serving it, and serves the saved values when built again (#1392). A saved
/// value wins over the configured one: the setter for a property a write set
/// ([`property_saved`](Self::property_saved)) checks its argument but leaves
/// the property alone. To change a saved value, write the property.
pub struct AccessRightsObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    global_identifier: u64,
    enable: bool,
    positive_access_rules: Vec<BACnetAccessRule>,
    negative_access_rules: Vec<BACnetAccessRule>,
    /// Accompaniment, served only while it holds a reference.
    accompaniment: Option<BACnetDeviceObjectReference>,
    status_flags: StatusFlags,
    reliability: Reliability,
    /// Where written rules, Enable and Accompaniment are saved, with
    /// persistence.
    storage: Option<saving::Storage>,
    /// Saved writes taken, so a staged write can tell whether another came
    /// between.
    writes: u64,
    /// The saved properties a write has set, now or before a restart.
    written: saving::Written,
}

impl AccessRightsObject {
    /// Create a new Access Rights object with no access rules, enabled. It
    /// keeps what peers write in memory only; see
    /// [`with_persistence`](Self::with_persistence).
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            global_identifier: 0,
            enable: true,
            positive_access_rules: Vec::new(),
            negative_access_rules: Vec::new(),
            accompaniment: None,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            storage: None,
            writes: 0,
            written: saving::Written::default(),
        })
    }

    /// Set Enable. FALSE disables every rule in both arrays (Clause
    /// 12.34.8) and leaves each rule's own enable flag alone, so TRUE again
    /// brings the rules back as they were. The default, TRUE, is a local
    /// choice; the standard names none.
    ///
    /// This configures the object and is not saved. With persistence, once
    /// a write has set Enable and it was saved, the saved value wins and
    /// this does nothing.
    pub fn set_enable(&mut self, enable: bool) {
        if !self.keeps_saved(PropertyIdentifier::LOG_ENABLE) {
            self.enable = enable;
        }
    }

    /// The stored Enable flag.
    pub fn enable(&self) -> bool {
        self.enable
    }

    /// Replace Positive_Access_Rules, the rules that grant access.
    ///
    /// The whole list is refused, keeping the rules set before, when it holds
    /// more than [`MAX_ACCESS_RULES`] rules (NO_SPACE_TO_WRITE_PROPERTY), or
    /// with VALUE_OUT_OF_RANGE when any rule (Clause 12.34.9.1):
    ///
    /// - names a device that isn't a Device in either reference (#1285);
    /// - holds a specifier outside its two named values;
    /// - is SPECIFIED for a member whose reference is left out;
    /// - is ALWAYS or ALL for a member whose reference is present and not
    ///   unspecified;
    /// - is SPECIFIED for a location that names neither an Access Point nor
    ///   an Access Zone and isn't unspecified.
    ///
    /// A reference is unspecified when its object identifier, and its device
    /// identifier if it has one, carry instance 4194303. A SPECIFIED member
    /// may hold one, standing for nothing to match yet. The time range may
    /// name a property of any object; a Schedule's Present_Value is typical.
    ///
    /// This configures the object and is not saved. With persistence, once
    /// a write has set the array and it was saved, the saved rules win: the
    /// rules given here are still checked, but not stored.
    pub fn set_positive_access_rules(
        &mut self,
        rules: impl IntoIterator<Item = BACnetAccessRule>,
    ) -> Result<(), Error> {
        let rules = checked_rules(rules)?;
        if !self.keeps_saved(PropertyIdentifier::POSITIVE_ACCESS_RULES) {
            self.positive_access_rules = rules;
        }
        Ok(())
    }

    /// Replace Negative_Access_Rules, the rules that deny access, with the
    /// same checks, and the same rule for saved rules, as
    /// [`set_positive_access_rules`](Self::set_positive_access_rules).
    pub fn set_negative_access_rules(
        &mut self,
        rules: impl IntoIterator<Item = BACnetAccessRule>,
    ) -> Result<(), Error> {
        let rules = checked_rules(rules)?;
        if !self.keeps_saved(PropertyIdentifier::NEGATIVE_ACCESS_RULES) {
            self.negative_access_rules = rules;
        }
        Ok(())
    }

    /// The stored Positive_Access_Rules.
    pub fn positive_access_rules(&self) -> &[BACnetAccessRule] {
        &self.positive_access_rules
    }

    /// The stored Negative_Access_Rules.
    pub fn negative_access_rules(&self) -> &[BACnetAccessRule] {
        &self.negative_access_rules
    }

    /// Set Accompaniment (Clause 12.34.11), the object a second credential
    /// presented with the first has to match for these rights to grant
    /// access: an Access Rights object the second credential holds, the
    /// Access Credential itself, or the Access User who owns it, here or in
    /// another device. A reference whose object, and device if it names one,
    /// carry instance 4194303 asks for no accompaniment. `None` leaves the
    /// optional property out, as a new object does; once set, the property
    /// is in Property_List and peers can write it.
    ///
    /// Refused with VALUE_OUT_OF_RANGE, keeping the value set before: a
    /// device member that isn't a Device (#1285), or an object of any other
    /// type unless the reference asks for no accompaniment.
    ///
    /// This configures the object and is not saved. With persistence, once
    /// a write has set Accompaniment and it was saved, the saved reference
    /// wins: the one given here is still checked, but not stored, and `None`
    /// leaves the property in place. To lift a saved requirement, write the
    /// no-accompaniment reference (instance 4194303), which keeps the row;
    /// to drop the row itself, remove the storage file, which also drops the
    /// saved rules and Enable.
    pub fn set_accompaniment(
        &mut self,
        accompaniment: Option<BACnetDeviceObjectReference>,
    ) -> Result<(), Error> {
        if let Some(reference) = &accompaniment {
            check_accompaniment(reference)?;
        }
        if !self.keeps_saved(PropertyIdentifier::ACCOMPANIMENT) {
            self.accompaniment = accompaniment;
        }
        Ok(())
    }

    /// The stored Accompaniment, `None` while the property is left out.
    pub fn accompaniment(&self) -> Option<&BACnetDeviceObjectReference> {
        self.accompaniment.as_ref()
    }
}

/// Refuse with VALUE_OUT_OF_RANGE an Accompaniment the object can't hold
/// (see [`AccessRightsObject::set_accompaniment`]). Clause 12.34.11 gives a
/// meaning to a reference to an Access Rights, Access Credential or Access
/// User object only, so any other type is refused unless the reference is
/// unspecified.
pub(super) fn check_accompaniment(reference: &BACnetDeviceObjectReference) -> Result<(), Error> {
    crate::device_reference::check_device_member(reference.device_identifier)?;
    let named = matches!(
        reference.object_identifier.object_type(),
        ObjectType::ACCESS_RIGHTS | ObjectType::ACCESS_CREDENTIAL | ObjectType::ACCESS_USER
    );
    if named || unspecified(reference.object_identifier, reference.device_identifier) {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// `rules` collected once they fit [`MAX_ACCESS_RULES`] and every one has
/// passed [`check_access_rule`].
pub(super) fn checked_rules(
    rules: impl IntoIterator<Item = BACnetAccessRule>,
) -> Result<Vec<BACnetAccessRule>, Error> {
    let rules: Vec<BACnetAccessRule> = rules.into_iter().collect();
    check_rule_count(rules.len())?;
    rules.iter().try_for_each(check_access_rule)?;
    Ok(rules)
}

/// Refuse a rule array longer than [`MAX_ACCESS_RULES`] with RESOURCES /
/// NO_SPACE_TO_WRITE_PROPERTY.
pub(super) fn check_rule_count(count: usize) -> Result<(), Error> {
    if count > MAX_ACCESS_RULES {
        Err(common::protocol_error(
            ErrorClass::RESOURCES,
            ErrorCode::NO_SPACE_TO_WRITE_PROPERTY,
        ))
    } else {
        Ok(())
    }
}

/// Refuse with VALUE_OUT_OF_RANGE a rule the setters' rules turn away (see
/// [`AccessRightsObject::set_positive_access_rules`]). The device members go
/// through the shared `check_device_member` first.
pub(super) fn check_access_rule(rule: &BACnetAccessRule) -> Result<(), Error> {
    let time_range_device = rule.time_range.as_ref().and_then(|r| r.device_identifier);
    crate::device_reference::check_device_member(time_range_device)?;
    let location_device = rule.location.as_ref().and_then(|r| r.device_identifier);
    crate::device_reference::check_device_member(location_device)?;

    let time_range_ok = match rule.time_range_specifier {
        AccessRuleTimeRangeSpecifier::SPECIFIED => rule.time_range.is_some(),
        AccessRuleTimeRangeSpecifier::ALWAYS => rule
            .time_range
            .as_ref()
            .is_none_or(|r| unspecified(r.object_identifier, r.device_identifier)),
        _ => false,
    };
    let location_ok = match rule.location_specifier {
        AccessRuleLocationSpecifier::SPECIFIED => rule.location.as_ref().is_some_and(|r| {
            matches!(
                r.object_identifier.object_type(),
                ObjectType::ACCESS_POINT | ObjectType::ACCESS_ZONE
            ) || unspecified(r.object_identifier, r.device_identifier)
        }),
        AccessRuleLocationSpecifier::ALL => rule
            .location
            .as_ref()
            .is_none_or(|r| unspecified(r.object_identifier, r.device_identifier)),
        _ => false,
    };
    if time_range_ok && location_ok {
        Ok(())
    } else {
        Err(common::value_out_of_range_error())
    }
}

/// Whether a rule's reference, or Accompaniment, is unspecified: its object,
/// and its device if it names one, both carry the reserved instance number
/// 4194303. The rights evaluation reads an unused Assigned_Access_Rights
/// element by the same rule.
pub(super) fn unspecified(object: ObjectIdentifier, device: Option<ObjectIdentifier>) -> bool {
    let unused = |oid: ObjectIdentifier| oid.instance_number() == ObjectIdentifier::MAX_INSTANCE;
    unused(object) && device.is_none_or(unused)
}

/// A rule array as `common::read_array` serves it: each rule in its Clause
/// 21 form.
fn rule_values(rules: &[BACnetAccessRule]) -> Vec<PropertyValue> {
    rules
        .iter()
        .map(|rule| {
            let mut buf = BytesMut::new();
            encode_access_rule(&mut buf, rule);
            PropertyValue::ApplicationData(buf.to_vec())
        })
        .collect()
}

impl BACnetObject for AccessRightsObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        &self.name
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        // Table 12-39 has no Out_Of_Service (#1064), and Clause 12.34 holds the
        // OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ACCESS_RIGHTS.to_raw(),
            )),
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier))
            }
            // Table 12-39's Enable row is property 133, which the enum names
            // after the log objects' Log_Enable.
            p if p == PropertyIdentifier::LOG_ENABLE => Ok(PropertyValue::Boolean(self.enable)),
            p if p == PropertyIdentifier::POSITIVE_ACCESS_RULES => {
                common::read_array(rule_values(&self.positive_access_rules), array_index)
            }
            p if p == PropertyIdentifier::NEGATIVE_ACCESS_RULES => {
                common::read_array(rule_values(&self.negative_access_rules), array_index)
            }
            p if p == PropertyIdentifier::ACCOMPANIMENT => match &self.accompaniment {
                Some(reference) => Ok(crate::device_reference::reference_value(reference)),
                None => Err(common::unknown_property_error()),
            },
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                if let PropertyValue::Unsigned(v) = value {
                    self.global_identifier = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            // Table 12-39 makes Enable and both rule arrays R rows and
            // Accompaniment an O row, so taking their writes is this
            // implementation's choice (Clause 12.1.2 leaves it open): a head
            // end provisions access rights over the network. With
            // persistence, each is saved before it is served.
            p if p == PropertyIdentifier::LOG_ENABLE
                || p == PropertyIdentifier::POSITIVE_ACCESS_RULES
                || p == PropertyIdentifier::NEGATIVE_ACCESS_RULES =>
            {
                self.write_saved(property, array_index, value)
            }
            // A write can't add the optional row; only the application can.
            p if p == PropertyIdentifier::ACCOMPANIMENT => {
                if self.accompaniment.is_none() {
                    return Err(common::unknown_property_error());
                }
                self.write_saved(property, array_index, value)
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        super::metadata_identity::for_access_rights_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn advance_monotonic_time_internal(&mut self, now: std::time::Duration) -> bool {
        self.expire_staged_write(now);
        false
    }

    fn durable_writes_internal(&mut self) -> Option<&mut dyn DurableWrites> {
        Some(self)
    }
}

#[cfg(test)]
#[path = "rights_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "rights_test_storage.rs"]
mod test_storage;

#[cfg(test)]
#[path = "rights_persistence_tests.rs"]
mod persistence_tests;

#[cfg(test)]
#[path = "rights_staging_tests.rs"]
mod staging_tests;

#[cfg(test)]
#[path = "rights_fold_tests.rs"]
mod fold_tests;

#[cfg(test)]
#[path = "rights_accompaniment_tests.rs"]
mod accompaniment_tests;

// ---------------------------------------------------------------------------
