use super::*;

// AccessUserObject (type 35)
// ---------------------------------------------------------------------------

/// BACnet Access User object (type 35).
///
/// Stands for whoever or whatever is granted access (someone, a team, a
/// tracked item) and the credentials it holds. Its kind lives in User_Type;
/// Table 12-38 has no Present_Value, Assigned_Access_Rights or Out_Of_Service
/// row, so the object serves none of them (#1064).
///
/// Credentials, Members and Member_Of are lists of
/// `BACnetDeviceObjectReference` (Clauses 12.33.12 to 12.33.14; #1394), so an
/// entry may name an object in another device. Credentials names the user's
/// Access Credential objects. Members and Member_Of name other Access Users,
/// one level down and one level up a hierarchy of users (a department and
/// the people in it, say). The application sets all three; they are
/// read-only over the network.
///
/// Global_Identifier names the user across devices: every Access User
/// standing for the same person, team or item carries the same nonzero
/// value, and 0 means none is assigned (Clause 12.33.5). It is the W row of
/// Table 12-38, so clients write it as they write an Access Credential's
/// (#1463).
pub struct AccessUserObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    global_identifier: u32,
    user_type: AccessUserType,
    credentials: Vec<BACnetDeviceObjectReference>,
    members: Vec<BACnetDeviceObjectReference>,
    member_of: Vec<BACnetDeviceObjectReference>,
    status_flags: StatusFlags,
    reliability: Reliability,
}

impl AccessUserObject {
    /// Create a new Access User object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_USER, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            global_identifier: 0,
            user_type: AccessUserType::ASSET,
            credentials: Vec::new(),
            members: Vec::new(),
            member_of: Vec::new(),
            status_flags: StatusFlags::empty(),
            reliability: Reliability::NO_FAULT_DETECTED,
        })
    }

    /// Set Global_Identifier; 0 means none is assigned (Clause 12.33.5).
    pub fn set_global_identifier(&mut self, value: u32) {
        self.global_identifier = value;
    }

    /// Set Credentials, the Access Credential objects the user holds
    /// (Clause 12.33.14). A reference with no device identifier names an
    /// object in this device.
    ///
    /// Each reference has to name an Access Credential object, and its
    /// device identifier, when given, a Device object; a list breaking either
    /// rule is refused with VALUE_OUT_OF_RANGE and the credentials set before
    /// are kept. The list is read-only over the network.
    pub fn set_credentials(
        &mut self,
        credentials: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
    ) -> Result<(), Error> {
        self.credentials = references_to(ObjectType::ACCESS_CREDENTIAL, credentials)?;
        Ok(())
    }

    /// Set Members, the Access Users one level below this one
    /// (Clause 12.33.12), with the checks [`Self::set_credentials`] makes,
    /// for Access User objects.
    pub fn set_members(
        &mut self,
        members: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
    ) -> Result<(), Error> {
        self.members = references_to(ObjectType::ACCESS_USER, members)?;
        Ok(())
    }

    /// Set Member_Of, the Access Users one level above this one
    /// (Clause 12.33.13), with the checks [`Self::set_members`] makes.
    pub fn set_member_of(
        &mut self,
        groups: impl IntoIterator<Item = impl Into<BACnetDeviceObjectReference>>,
    ) -> Result<(), Error> {
        self.member_of = references_to(ObjectType::ACCESS_USER, groups)?;
        Ok(())
    }
}

impl BACnetObject for AccessUserObject {
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
        // Clause 12.33 holds the OUT_OF_SERVICE flag FALSE.
        if let Some(result) =
            read_common_properties!(self, property, array_index, no_out_of_service)
        {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_USER.to_raw()))
            }
            p if p == PropertyIdentifier::USER_TYPE => {
                Ok(PropertyValue::Enumerated(self.user_type.to_raw()))
            }
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier.into()))
            }
            p if p == PropertyIdentifier::CREDENTIALS => {
                Ok(crate::device_reference::reference_list(&self.credentials))
            }
            p if p == PropertyIdentifier::MEMBERS => {
                Ok(crate::device_reference::reference_list(&self.members))
            }
            p if p == PropertyIdentifier::MEMBER_OF => {
                Ok(crate::device_reference::reference_list(&self.member_of))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
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
            p if p == PropertyIdentifier::USER_TYPE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.user_type = AccessUserType::from_raw(v);
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(crate::common::unhandled_write_error(
                self.property_metadata().as_ref(),
                property,
                _array_index,
            )),
        }
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        super::metadata_identity::for_access_user_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }
}

// ---------------------------------------------------------------------------
