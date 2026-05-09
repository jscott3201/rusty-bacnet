use super::*;

// AccessRightsObject (type 34)
// ---------------------------------------------------------------------------

/// BACnet Access Rights object (type 34).
///
/// Defines a set of access rules (positive and negative) that can be
/// assigned to credentials and users.
pub struct AccessRightsObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    global_identifier: u64,
    positive_access_rules_count: u32,
    negative_access_rules_count: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessRightsObject {
    /// Create a new Access Rights object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_RIGHTS, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            global_identifier: 0,
            positive_access_rules_count: 0,
            negative_access_rules_count: 0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
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
        if let Some(result) = read_common_properties!(self, property, array_index) {
            return result;
        }
        match property {
            p if p == PropertyIdentifier::OBJECT_TYPE => Ok(PropertyValue::Enumerated(
                ObjectType::ACCESS_RIGHTS.to_raw(),
            )),
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier))
            }
            p if p == PropertyIdentifier::POSITIVE_ACCESS_RULES => Ok(PropertyValue::Unsigned(
                self.positive_access_rules_count as u64,
            )),
            p if p == PropertyIdentifier::NEGATIVE_ACCESS_RULES => Ok(PropertyValue::Unsigned(
                self.negative_access_rules_count as u64,
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
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                if let PropertyValue::Unsigned(v) = value {
                    self.global_identifier = v;
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
            PropertyIdentifier::GLOBAL_IDENTIFIER,
            PropertyIdentifier::POSITIVE_ACCESS_RULES,
            PropertyIdentifier::NEGATIVE_ACCESS_RULES,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
