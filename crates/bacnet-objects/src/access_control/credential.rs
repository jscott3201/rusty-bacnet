use super::*;

// AccessCredentialObject (type 32)
// ---------------------------------------------------------------------------

/// BACnet Access Credential object (type 32).
///
/// Represents a credential (card, fob, biometric, etc.) used for access control.
/// Present value indicates active/inactive status (BinaryPV).
pub struct AccessCredentialObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // BinaryPV: 0=inactive, 1=active
    credential_status: u32,
    assigned_access_rights_count: u32,
    authentication_factors: Vec<Vec<u8>>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessCredentialObject {
    /// Create a new Access Credential object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_CREDENTIAL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // inactive
            credential_status: 0,
            assigned_access_rights_count: 0,
            authentication_factors: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ACCESS_CREDENTIAL.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::CREDENTIAL_STATUS => {
                Ok(PropertyValue::Enumerated(self.credential_status))
            }
            p if p == PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS => Ok(PropertyValue::Unsigned(
                self.assigned_access_rights_count as u64,
            )),
            p if p == PropertyIdentifier::AUTHENTICATION_FACTORS => Ok(PropertyValue::List(
                self.authentication_factors
                    .iter()
                    .map(|f| PropertyValue::OctetString(f.clone()))
                    .collect(),
            )),
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
        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Enumerated(v) = value {
                    self.present_value = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::CREDENTIAL_STATUS => {
                if let PropertyValue::Enumerated(v) = value {
                    self.credential_status = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            _ => Err(common::write_access_denied_error()),
        }
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::CREDENTIAL_STATUS,
            PropertyIdentifier::ASSIGNED_ACCESS_RIGHTS,
            PropertyIdentifier::AUTHENTICATION_FACTORS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
