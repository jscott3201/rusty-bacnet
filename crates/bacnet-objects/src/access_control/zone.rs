use super::*;

// AccessZoneObject (type 36)
// ---------------------------------------------------------------------------

/// BACnet Access Zone object (type 36).
///
/// Represents a physical zone or area controlled by access points.
/// Present value indicates the occupancy state (AccessZoneOccupancyState).
pub struct AccessZoneObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32, // AccessZoneOccupancyState enumeration
    global_identifier: u64,
    occupancy_count: u64,
    access_doors: Vec<ObjectIdentifier>,
    entry_points: Vec<ObjectIdentifier>,
    exit_points: Vec<ObjectIdentifier>,
    status_flags: StatusFlags,
    out_of_service: bool,
    reliability: u32,
}

impl AccessZoneObject {
    /// Create a new Access Zone object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_ZONE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            global_identifier: 0,
            occupancy_count: 0,
            access_doors: Vec::new(),
            entry_points: Vec::new(),
            exit_points: Vec::new(),
            status_flags: StatusFlags::empty(),
            out_of_service: false,
            reliability: 0,
        })
    }
}

impl BACnetObject for AccessZoneObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_ZONE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::GLOBAL_IDENTIFIER => {
                Ok(PropertyValue::Unsigned(self.global_identifier))
            }
            p if p == PropertyIdentifier::OCCUPANCY_COUNT => {
                Ok(PropertyValue::Unsigned(self.occupancy_count))
            }
            p if p == PropertyIdentifier::ACCESS_DOORS => Ok(PropertyValue::List(
                self.access_doors
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::ENTRY_POINTS => Ok(PropertyValue::List(
                self.entry_points
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::EXIT_POINTS => Ok(PropertyValue::List(
                self.exit_points
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
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
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::GLOBAL_IDENTIFIER,
            PropertyIdentifier::OCCUPANCY_COUNT,
            PropertyIdentifier::ACCESS_DOORS,
            PropertyIdentifier::ENTRY_POINTS,
            PropertyIdentifier::EXIT_POINTS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
