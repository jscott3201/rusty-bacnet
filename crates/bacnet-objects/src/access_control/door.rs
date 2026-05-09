use super::*;

// AccessDoorObject (type 30)
// ---------------------------------------------------------------------------

/// BACnet Access Door object (type 30).
///
/// Represents a physical door or barrier in an access control system.
/// Present value indicates the door command status (DoorStatus enumeration).
pub struct AccessDoorObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,    // DoorStatus: 0=closed, 1=opened, 2=unknown
    door_status: u32,      // DoorStatus enumeration
    lock_status: u32,      // LockStatus enumeration
    secured_status: u32,   // DoorSecuredStatus enumeration
    door_alarm_state: u32, // DoorAlarmState enumeration
    door_members: Vec<ObjectIdentifier>,
    status_flags: StatusFlags,
    /// Event_State: 0 = NORMAL.
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
    /// 16-level priority array for commandable Present_Value.
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
}

impl AccessDoorObject {
    /// Create a new Access Door object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::ACCESS_DOOR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // closed
            door_status: 0,   // closed
            lock_status: 0,
            secured_status: 0,
            door_alarm_state: 0,
            door_members: Vec::new(),
            status_flags: StatusFlags::empty(),
            event_state: 0, // NORMAL
            out_of_service: false,
            reliability: 0,
            priority_array: Default::default(),
            relinquish_default: 0, // closed
        })
    }
}

impl BACnetObject for AccessDoorObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::ACCESS_DOOR.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::DOOR_STATUS => {
                Ok(PropertyValue::Enumerated(self.door_status))
            }
            p if p == PropertyIdentifier::LOCK_STATUS => {
                Ok(PropertyValue::Enumerated(self.lock_status))
            }
            p if p == PropertyIdentifier::SECURED_STATUS => {
                Ok(PropertyValue::Enumerated(self.secured_status))
            }
            p if p == PropertyIdentifier::DOOR_ALARM_STATE => {
                Ok(PropertyValue::Enumerated(self.door_alarm_state))
            }
            p if p == PropertyIdentifier::DOOR_MEMBERS => Ok(PropertyValue::List(
                self.door_members
                    .iter()
                    .map(|oid| PropertyValue::ObjectIdentifier(*oid))
                    .collect(),
            )),
            p if p == PropertyIdentifier::EVENT_STATE => {
                Ok(PropertyValue::Enumerated(self.event_state))
            }
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                common::read_priority_array!(self, array_index, PropertyValue::Enumerated)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Enumerated(self.relinquish_default))
            }
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
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
                let slot = priority.unwrap_or(16).clamp(1, 16) as usize - 1;
                if let PropertyValue::Null = value {
                    // Relinquish command at this priority
                    self.priority_array[slot] = None;
                } else if let PropertyValue::Enumerated(v) = value {
                    self.priority_array[slot] = Some(v);
                } else if self.out_of_service {
                    // When OOS, accept direct writes without priority
                    if let PropertyValue::Enumerated(v) = value {
                        self.present_value = v;
                        return Ok(());
                    }
                    return Err(common::invalid_data_type_error());
                } else {
                    return Err(common::invalid_data_type_error());
                }
                // Recalculate PV from priority array
                self.present_value = self
                    .priority_array
                    .iter()
                    .flatten()
                    .next()
                    .copied()
                    .unwrap_or(self.relinquish_default);
                Ok(())
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
            PropertyIdentifier::DOOR_STATUS,
            PropertyIdentifier::LOCK_STATUS,
            PropertyIdentifier::SECURED_STATUS,
            PropertyIdentifier::DOOR_ALARM_STATE,
            PropertyIdentifier::DOOR_MEMBERS,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
