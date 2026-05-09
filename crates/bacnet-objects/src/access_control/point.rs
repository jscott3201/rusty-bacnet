use super::*;

// AccessPointObject (type 33)
// ---------------------------------------------------------------------------

/// BACnet Access Point object (type 33).
///
/// Represents an access point (reader/controller at a door) in an access control system.
/// Present value indicates the most recent access event.
pub struct AccessPointObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // AccessEvent enumeration
    access_event: u32,
    access_event_tag: u64,
    access_event_time: ([u8; 4], [u8; 4]), // (Date, Time) as raw bytes
    access_doors: Vec<ObjectIdentifier>,
    event_state: u32,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessPointObject {
    /// Create a new Access Point object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_POINT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            access_event: 0,
            access_event_tag: 0,
            access_event_time: ([0xFF, 0xFF, 0xFF, 0xFF], [0xFF, 0xFF, 0xFF, 0xFF]),
            access_doors: Vec::new(),
            event_state: 0,
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for AccessPointObject {
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
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_POINT.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT => {
                Ok(PropertyValue::Enumerated(self.access_event))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT_TAG => {
                Ok(PropertyValue::Unsigned(self.access_event_tag))
            }
            p if p == PropertyIdentifier::ACCESS_EVENT_TIME => {
                let (d, t) = &self.access_event_time;
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
            p if p == PropertyIdentifier::ACCESS_DOORS => Ok(PropertyValue::List(
                self.access_doors
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
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
            PropertyIdentifier::ACCESS_EVENT,
            PropertyIdentifier::ACCESS_EVENT_TAG,
            PropertyIdentifier::ACCESS_EVENT_TIME,
            PropertyIdentifier::ACCESS_DOORS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
