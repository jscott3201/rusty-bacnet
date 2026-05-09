use super::*;

// CredentialDataInputObject (type 37)
// ---------------------------------------------------------------------------

/// BACnet Credential Data Input object (type 37).
///
/// Represents a credential reader device (card reader, biometric scanner, etc.).
/// Present value indicates the authentication status.
pub struct CredentialDataInputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,              // AuthenticationStatus: 0=notReady, 1=waiting
    update_time: ([u8; 4], [u8; 4]), // (Date, Time) as raw bytes
    supported_formats: Vec<u64>,
    supported_format_classes: Vec<u64>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl CredentialDataInputObject {
    /// Create a new Credential Data Input object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::CREDENTIAL_DATA_INPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // notReady
            update_time: ([0xFF, 0xFF, 0xFF, 0xFF], [0xFF, 0xFF, 0xFF, 0xFF]),
            supported_formats: Vec::new(),
            supported_format_classes: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for CredentialDataInputObject {
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
                ObjectType::CREDENTIAL_DATA_INPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::UPDATE_TIME => {
                let (d, t) = &self.update_time;
                Ok(PropertyValue::List(vec![
                    PropertyValue::Date(Date {
                        year: d[0],
                        month: d[1],
                        day: d[2],
                        day_of_week: d[3],
                    }),
                    PropertyValue::Time(Time {
                        hour: t[0],
                        minute: t[1],
                        second: t[2],
                        hundredths: t[3],
                    }),
                ]))
            }
            p if p == PropertyIdentifier::SUPPORTED_FORMATS => Ok(PropertyValue::List(
                self.supported_formats
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v))
                    .collect(),
            )),
            p if p == PropertyIdentifier::SUPPORTED_FORMAT_CLASSES => Ok(PropertyValue::List(
                self.supported_format_classes
                    .iter()
                    .map(|v| PropertyValue::Unsigned(*v))
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
        // CredentialDataInput is primarily read-only (driven by hardware)
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::UPDATE_TIME,
            PropertyIdentifier::SUPPORTED_FORMATS,
            PropertyIdentifier::SUPPORTED_FORMAT_CLASSES,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
