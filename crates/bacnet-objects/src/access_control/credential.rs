use std::sync::Arc;

use bacnet_encoding::constructed::{
    encode_assigned_access_rights, encode_credential_authentication_factor,
};
use bacnet_types::constructed::{BACnetAssignedAccessRights, BACnetCredentialAuthenticationFactor};
use bacnet_types::enums::{
    AccessCredentialDisable, AccessCredentialDisableReason, AuthorizationExemption,
};
use bytes::BytesMut;

use super::credential_rules::{self as rules, WindowLimit};
use super::*;
use crate::clock::ClockReader;

// AccessCredentialObject (type 32)
// ---------------------------------------------------------------------------

/// BACnet Access Credential object (type 32).
///
/// Represents a credential (card, fob, biometric, etc.) used for access
/// control. Table 12-40 has no Present_Value row, so the object serves none
/// (#979).
///
/// Credential_Status is derived, not stored (Clause 12.35.8): INACTIVE while
/// Reason_For_Disable lists anything, ACTIVE otherwise, so it is read-only.
/// Reason_For_Disable is the sorted union of three sources (#1073):
///
/// - the reasons the application raises with
///   [`add_disable_reason`](Self::add_disable_reason);
/// - the reason the current Credential_Disable value adds (Clause 12.35.13),
///   so changing that property drops the reason its previous value added;
/// - DISABLED_NOT_YET_ACTIVE and DISABLED_EXPIRED, judged against the
///   database's wall clock each time the list is read, the way the Calendar
///   follows the date (Clauses 12.35.11 and 12.35.12). Without a clock frame
///   naming a real moment the window adds nothing.
///
/// The network may write Global_Identifier, Credential_Disable,
/// Activation_Time and Expiration_Time. Authentication_Factors and
/// Assigned_Access_Rights are BACnetARRAYs the application provisions.
///
/// The optional Authorization_Exemptions row (Clause 12.35.25; #1331) is
/// served once the application sets it with
/// [`set_authorization_exemptions`](Self::set_authorization_exemptions), and
/// is read-only over the network. With ACCESS_RIGHTS listed,
/// [`evaluate_access_rights`] reports the credential exempt from the Access
/// Rights check.
pub struct AccessCredentialObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    global_identifier: u32,
    /// Reasons raised by the application, sorted by value without repeats.
    local_reasons: Vec<AccessCredentialDisableReason>,
    credential_disable: AccessCredentialDisable,
    activation_time: WindowLimit,
    expiration_time: WindowLimit,
    authentication_factors: Vec<BACnetCredentialAuthenticationFactor>,
    assigned_access_rights: Vec<BACnetAssignedAccessRights>,
    /// Authorization_Exemptions, served only while it holds a list.
    authorization_exemptions: Option<Vec<AuthorizationExemption>>,
    status_flags: StatusFlags,
    reliability: Reliability,
    /// The database's wall clock, the source of the current moment.
    clock: Option<Arc<dyn ClockReader>>,
}

impl AccessCredentialObject {
    /// Create a new Access Credential object: no disable reasons (so
    /// ACTIVE), Credential_Disable NONE, an open validity window, no global
    /// identifier and empty arrays.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            global_identifier: 0,
            local_reasons: Vec::new(),
            credential_disable: AccessCredentialDisable::NONE,
            activation_time: WindowLimit::OPEN,
            expiration_time: WindowLimit::OPEN,
            authentication_factors: Vec::new(),
            assigned_access_rights: Vec::new(),
            authorization_exemptions: None,
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
            clock: None,
        })
    }

    /// Set Global_Identifier; 0 means none is assigned (Clause 12.35.5).
    pub fn set_global_identifier(&mut self, value: u32) {
        self.global_identifier = value;
    }

    /// Set Credential_Disable. A value outside the four named ones and the
    /// vendor range 64 to 65535 is refused with VALUE_OUT_OF_RANGE.
    pub fn set_credential_disable(&mut self, value: AccessCredentialDisable) -> Result<(), Error> {
        rules::check_credential_disable(value)?;
        self.credential_disable = value;
        Ok(())
    }

    /// Set Activation_Time. Every octet X'FF' leaves the start open; a
    /// partly specified value is refused with VALUE_OUT_OF_RANGE.
    pub fn set_activation_time(&mut self, date: Date, time: Time) -> Result<(), Error> {
        self.activation_time = WindowLimit::new(date, time)?;
        Ok(())
    }

    /// Set Expiration_Time, with the same rules as
    /// [`set_activation_time`](Self::set_activation_time).
    pub fn set_expiration_time(&mut self, date: Date, time: Time) -> Result<(), Error> {
        self.expiration_time = WindowLimit::new(date, time)?;
        Ok(())
    }

    /// The stored Activation_Time.
    pub fn activation_time(&self) -> (Date, Time) {
        self.activation_time.parts()
    }

    /// The stored Expiration_Time.
    pub fn expiration_time(&self) -> (Date, Time) {
        self.expiration_time.parts()
    }

    /// Raise a disable reason on behalf of the application, such as
    /// DISABLED_LOCKOUT after too many retries. Raising one already raised
    /// does nothing.
    ///
    /// Refused with VALUE_OUT_OF_RANGE: a value outside the named reasons and
    /// the vendor range, and DISABLED_NOT_YET_ACTIVE and DISABLED_EXPIRED,
    /// which follow Activation_Time and Expiration_Time instead.
    pub fn add_disable_reason(
        &mut self,
        reason: AccessCredentialDisableReason,
    ) -> Result<(), Error> {
        rules::check_local_reason(reason)?;
        if let Err(index) = self
            .local_reasons
            .binary_search_by_key(&reason.to_raw(), |r| r.to_raw())
        {
            self.local_reasons.insert(index, reason);
        }
        Ok(())
    }

    /// Withdraw a reason the application raised; `false` if it wasn't
    /// raised. Reasons from Credential_Disable or the validity window stay.
    pub fn remove_disable_reason(&mut self, reason: AccessCredentialDisableReason) -> bool {
        let before = self.local_reasons.len();
        self.local_reasons.retain(|r| *r != reason);
        self.local_reasons.len() != before
    }

    /// Reason_For_Disable now: every source's reasons, sorted by value
    /// without repeats.
    pub fn reason_for_disable(&self) -> Vec<AccessCredentialDisableReason> {
        let frame = self.clock.as_ref().and_then(|clock| clock.read_clock());
        let mut reasons = self.local_reasons.clone();
        reasons.extend(rules::credential_disable_reason(self.credential_disable));
        reasons.extend(rules::window_reasons(
            frame,
            self.activation_time,
            self.expiration_time,
        ));
        reasons.sort_by_key(|r| r.to_raw());
        reasons.dedup();
        reasons
    }

    /// Credential_Status now: INACTIVE while any disable reason holds.
    pub fn credential_status(&self) -> BinaryPV {
        if self.reason_for_disable().is_empty() {
            BinaryPV::ACTIVE
        } else {
            BinaryPV::INACTIVE
        }
    }

    /// Replace Authentication_Factors. Each element's disable value must be
    /// named or in the vendor range and its format type one of the 25
    /// named; otherwise VALUE_OUT_OF_RANGE and nothing changes.
    pub fn set_authentication_factors(
        &mut self,
        factors: Vec<BACnetCredentialAuthenticationFactor>,
    ) -> Result<(), Error> {
        factors
            .iter()
            .try_for_each(rules::check_authentication_factor)?;
        self.authentication_factors = factors;
        Ok(())
    }

    /// The stored Authentication_Factors.
    pub fn authentication_factors(&self) -> &[BACnetCredentialAuthenticationFactor] {
        &self.authentication_factors
    }

    /// Replace Assigned_Access_Rights. Each element must reference an Access
    /// Rights object (or carry instance 4194303, the unused marker), and a
    /// device identifier, if present, must be a Device; otherwise
    /// VALUE_OUT_OF_RANGE and nothing changes.
    pub fn set_assigned_access_rights(
        &mut self,
        rights: Vec<BACnetAssignedAccessRights>,
    ) -> Result<(), Error> {
        rights
            .iter()
            .try_for_each(rules::check_assigned_access_rights)?;
        self.assigned_access_rights = rights;
        Ok(())
    }

    /// The stored Assigned_Access_Rights.
    pub fn assigned_access_rights(&self) -> &[BACnetAssignedAccessRights] {
        &self.assigned_access_rights
    }

    /// Set Authorization_Exemptions (Clause 12.35.25), the authorization
    /// checks this credential is exempt from. `Some` serves the optional row,
    /// an empty list included, and adds it to Property_List; `None` leaves it
    /// out, as a new credential does. The list is read-only over the network.
    ///
    /// A value outside the seven named checks and the vendor range 64 to 255
    /// is refused with VALUE_OUT_OF_RANGE, keeping the list set before.
    pub fn set_authorization_exemptions(
        &mut self,
        exemptions: Option<Vec<AuthorizationExemption>>,
    ) -> Result<(), Error> {
        if let Some(list) = &exemptions {
            list.iter()
                .copied()
                .try_for_each(rules::check_authorization_exemption)?;
        }
        self.authorization_exemptions = exemptions;
        Ok(())
    }

    /// The stored Authorization_Exemptions, `None` while the row is left out.
    pub fn authorization_exemptions(&self) -> Option<&[AuthorizationExemption]> {
        self.authorization_exemptions.as_deref()
    }
}

/// Encode each element of a constructed array as its own framed value.
fn framed<T>(items: &[T], encode: fn(&mut BytesMut, &T)) -> Vec<PropertyValue> {
    items
        .iter()
        .map(|item| {
            let mut buf = BytesMut::new();
            encode(&mut buf, item);
            PropertyValue::ApplicationData(buf.to_vec())
        })
        .collect()
}

impl BACnetObject for AccessCredentialObject {
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
        // Table 12-40 has no Out_Of_Service (#1064), and Clause 12.35 holds the
        // OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ACCESS_CREDENTIAL.to_raw(),
            )),
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier.into()))
            }
            p if p == PropertyIdentifier::CREDENTIAL_STATUS => {
                Ok(PropertyValue::Enumerated(self.credential_status().to_raw()))
            }
            p if p == PropertyIdentifier::REASON_FOR_DISABLE => Ok(PropertyValue::List(
                self.reason_for_disable()
                    .into_iter()
                    .map(|reason| PropertyValue::Enumerated(reason.to_raw()))
                    .collect(),
            )),
            p if p == PropertyIdentifier::AUTHENTICATION_FACTORS => common::read_array(
                framed(
                    &self.authentication_factors,
                    encode_credential_authentication_factor,
                ),
                array_index,
            ),
            p if p == PropertyIdentifier::ACTIVATION_TIME => Ok(self.activation_time.to_property()),
            p if p == PropertyIdentifier::EXPIRATION_TIME => Ok(self.expiration_time.to_property()),
            p if p == PropertyIdentifier::CREDENTIAL_DISABLE => {
                Ok(PropertyValue::Enumerated(self.credential_disable.to_raw()))
            }
            p if p == PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS => common::read_array(
                framed(&self.assigned_access_rights, encode_assigned_access_rights),
                array_index,
            ),
            p if p == PropertyIdentifier::AUTHORIZATION_EXEMPTIONS => {
                match &self.authorization_exemptions {
                    Some(list) => Ok(PropertyValue::List(
                        list.iter()
                            .map(|exemption| PropertyValue::Enumerated(exemption.to_raw()))
                            .collect(),
                    )),
                    None => Err(common::unknown_property_error()),
                }
            }
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
                let PropertyValue::Unsigned(raw) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.global_identifier = common::u64_to_u32(raw)?;
                Ok(())
            }
            p if p == PropertyIdentifier::CREDENTIAL_DISABLE => {
                let PropertyValue::Enumerated(raw) = value else {
                    return Err(common::invalid_data_type_error());
                };
                self.set_credential_disable(AccessCredentialDisable::from_raw(raw))
            }
            p if p == PropertyIdentifier::ACTIVATION_TIME => {
                self.activation_time = WindowLimit::from_property(value)?;
                Ok(())
            }
            p if p == PropertyIdentifier::EXPIRATION_TIME => {
                self.expiration_time = WindowLimit::from_property(value)?;
                Ok(())
            }
            // Credential_Status follows Reason_For_Disable (Clause 12.35.8),
            // so it, like the list itself, is refused as read-only here.
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        super::metadata_identity::for_access_credential_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }
}

// ---------------------------------------------------------------------------
