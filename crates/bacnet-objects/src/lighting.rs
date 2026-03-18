//! Lighting Output (type 54), Binary Lighting Output (type 55), and Channel
//! (type 53) objects per ASHRAE 135-2020 Clauses 12.55, 12.56, and 12.53.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::{
    self, read_common_properties, read_priority_array, write_priority_array,
    write_priority_array_direct,
};
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// LightingOutput (type 54)
// ---------------------------------------------------------------------------

/// BACnet Lighting Output object.
///
/// Commandable output with a 16-level priority array controlling a
/// floating-point present-value (0.0 to 100.0 percent).
pub struct LightingOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: f32,
    tracking_value: f32,
    /// Stored as opaque OctetString for now (BACnetLightingCommand encoding).
    lighting_command: Vec<u8>,
    lighting_command_default_priority: u32,
    /// LightingInProgress enumeration: 0=idle, 1=fade-active, 2=ramp-active, 3=not-controlled, etc.
    in_progress: u32,
    blink_warn_enable: bool,
    egress_time: u32,
    egress_active: bool,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    priority_array: [Option<f32>; 16],
    relinquish_default: f32,
}

impl LightingOutputObject {
    /// Create a new Lighting Output object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LIGHTING_OUTPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0.0,
            tracking_value: 0.0,
            lighting_command: Vec::new(),
            lighting_command_default_priority: 16,
            in_progress: 0, // idle
            blink_warn_enable: false,
            egress_time: 0,
            egress_active: false,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: 0,
            priority_array: [None; 16],
            relinquish_default: 0.0,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Recalculate present-value from the priority array.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }
}

impl BACnetObject for LightingOutputObject {
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
                ObjectType::LIGHTING_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Real(self.present_value))
            }
            p if p == PropertyIdentifier::TRACKING_VALUE => {
                Ok(PropertyValue::Real(self.tracking_value))
            }
            p if p == PropertyIdentifier::LIGHTING_COMMAND => {
                Ok(PropertyValue::OctetString(self.lighting_command.clone()))
            }
            p if p == PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY => Ok(
                PropertyValue::Unsigned(self.lighting_command_default_priority as u64),
            ),
            p if p == PropertyIdentifier::IN_PROGRESS => {
                Ok(PropertyValue::Enumerated(self.in_progress))
            }
            p if p == PropertyIdentifier::BLINK_WARN_ENABLE => {
                Ok(PropertyValue::Boolean(self.blink_warn_enable))
            }
            p if p == PropertyIdentifier::EGRESS_TIME => {
                Ok(PropertyValue::Unsigned(self.egress_time as u64))
            }
            p if p == PropertyIdentifier::EGRESS_ACTIVE => {
                Ok(PropertyValue::Boolean(self.egress_active))
            }
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                read_priority_array!(self, array_index, PropertyValue::Real)
            }
            p if p == PropertyIdentifier::RELINQUISH_DEFAULT => {
                Ok(PropertyValue::Real(self.relinquish_default))
            }
            p if p == PropertyIdentifier::DEFAULT_FADE_TIME => Ok(PropertyValue::Unsigned(0)),
            _ => Err(common::unknown_property_error()),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        // Direct writes to PRIORITY_ARRAY[index]
        write_priority_array_direct!(self, property, array_index, value, |v| {
            match v {
                PropertyValue::Real(f) => {
                    if !(0.0..=100.0).contains(&f) {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(f)
                    }
                }
                _ => Err(common::invalid_data_type_error()),
            }
        });

        // PRESENT_VALUE — commandable via priority array
        if property == PropertyIdentifier::PRESENT_VALUE {
            return write_priority_array!(self, value, priority, |v| {
                match v {
                    PropertyValue::Real(f) => {
                        if !(0.0..=100.0).contains(&f) {
                            Err(common::value_out_of_range_error())
                        } else {
                            Ok(f)
                        }
                    }
                    _ => Err(common::invalid_data_type_error()),
                }
            });
        }

        // LIGHTING_COMMAND — stored as opaque bytes
        if property == PropertyIdentifier::LIGHTING_COMMAND {
            if let PropertyValue::OctetString(data) = value {
                self.lighting_command = data;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // LIGHTING_COMMAND_DEFAULT_PRIORITY
        if property == PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY {
            if let PropertyValue::Unsigned(v) = value {
                if !(1..=16).contains(&v) {
                    return Err(common::value_out_of_range_error());
                }
                self.lighting_command_default_priority = v as u32;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // BLINK_WARN_ENABLE
        if property == PropertyIdentifier::BLINK_WARN_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.blink_warn_enable = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // EGRESS_TIME
        if property == PropertyIdentifier::EGRESS_TIME {
            if let PropertyValue::Unsigned(v) = value {
                self.egress_time = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::TRACKING_VALUE,
            PropertyIdentifier::LIGHTING_COMMAND,
            PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY,
            PropertyIdentifier::IN_PROGRESS,
            PropertyIdentifier::BLINK_WARN_ENABLE,
            PropertyIdentifier::EGRESS_TIME,
            PropertyIdentifier::EGRESS_ACTIVE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// BinaryLightingOutput (type 55)
// ---------------------------------------------------------------------------

/// BACnet Binary Lighting Output object.
///
/// Commandable output with a 16-level priority array controlling an
/// Enumerated present-value: 0=off, 1=on, 2=warn, 3=warn-off, 4=fade-on.
pub struct BinaryLightingOutputObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    blink_warn_enable: bool,
    egress_time: u32,
    egress_active: bool,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
    priority_array: [Option<u32>; 16],
    relinquish_default: u32,
}

impl BinaryLightingOutputObject {
    /// Valid BinaryLightingPV values: off=0, on=1, warn=2, warn-off=3, fade-on=4.
    const MAX_PV: u32 = 4;

    /// Create a new Binary Lighting Output object.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::BINARY_LIGHTING_OUTPUT, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0, // off
            blink_warn_enable: false,
            egress_time: 0,
            egress_active: false,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: 0,
            priority_array: [None; 16],
            relinquish_default: 0,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Recalculate present-value from the priority array.
    fn recalculate_present_value(&mut self) {
        self.present_value =
            common::recalculate_from_priority_array(&self.priority_array, self.relinquish_default);
    }
}

impl BACnetObject for BinaryLightingOutputObject {
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
                ObjectType::BINARY_LIGHTING_OUTPUT.to_raw(),
            )),
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::BLINK_WARN_ENABLE => {
                Ok(PropertyValue::Boolean(self.blink_warn_enable))
            }
            p if p == PropertyIdentifier::EGRESS_TIME => {
                Ok(PropertyValue::Unsigned(self.egress_time as u64))
            }
            p if p == PropertyIdentifier::EGRESS_ACTIVE => {
                Ok(PropertyValue::Boolean(self.egress_active))
            }
            p if p == PropertyIdentifier::PRIORITY_ARRAY => {
                read_priority_array!(self, array_index, PropertyValue::Enumerated)
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
        array_index: Option<u32>,
        value: PropertyValue,
        priority: Option<u8>,
    ) -> Result<(), Error> {
        // Direct writes to PRIORITY_ARRAY[index]
        write_priority_array_direct!(self, property, array_index, value, |v| {
            if let PropertyValue::Enumerated(e) = v {
                if e > Self::MAX_PV {
                    Err(common::value_out_of_range_error())
                } else {
                    Ok(e)
                }
            } else {
                Err(common::invalid_data_type_error())
            }
        });

        // PRESENT_VALUE — commandable via priority array
        if property == PropertyIdentifier::PRESENT_VALUE {
            return write_priority_array!(self, value, priority, |v| {
                if let PropertyValue::Enumerated(e) = v {
                    if e > Self::MAX_PV {
                        Err(common::value_out_of_range_error())
                    } else {
                        Ok(e)
                    }
                } else {
                    Err(common::invalid_data_type_error())
                }
            });
        }

        // BLINK_WARN_ENABLE
        if property == PropertyIdentifier::BLINK_WARN_ENABLE {
            if let PropertyValue::Boolean(v) = value {
                self.blink_warn_enable = v;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // EGRESS_TIME
        if property == PropertyIdentifier::EGRESS_TIME {
            if let PropertyValue::Unsigned(v) = value {
                self.egress_time = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::BLINK_WARN_ENABLE,
            PropertyIdentifier::EGRESS_TIME,
            PropertyIdentifier::EGRESS_ACTIVE,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::OUT_OF_SERVICE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::PRIORITY_ARRAY,
            PropertyIdentifier::RELINQUISH_DEFAULT,
        ];
        Cow::Borrowed(PROPS)
    }

    fn supports_cov(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Channel (type 53)
// ---------------------------------------------------------------------------

/// BACnet Channel object.
///
/// A channel aggregates multiple objects for group control. The present-value
/// represents the current channel value, and writes propagate to members.
pub struct ChannelObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present value — the current channel value (Unsigned).
    present_value: u32,
    /// Last priority used for the most recent write (Unsigned).
    last_priority: u32,
    /// Write status: 0=idle, 1=inProgress, 2=successful, 3=failed.
    write_status: u32,
    /// Channel number (Unsigned).
    channel_number: u32,
    /// Count of object-property references in this channel's member list.
    list_of_object_property_references_count: u32,
    out_of_service: bool,
    status_flags: StatusFlags,
    /// Reliability: 0 = NO_FAULT_DETECTED.
    reliability: u32,
}

impl ChannelObject {
    /// Create a new Channel object.
    pub fn new(instance: u32, name: impl Into<String>, channel_number: u32) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::CHANNEL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            last_priority: 16,
            write_status: 0, // idle
            channel_number,
            list_of_object_property_references_count: 0,
            out_of_service: false,
            status_flags: StatusFlags::empty(),
            reliability: 0,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }
}

impl BACnetObject for ChannelObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::CHANNEL.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Unsigned(self.present_value as u64))
            }
            p if p == PropertyIdentifier::LAST_PRIORITY => {
                Ok(PropertyValue::Unsigned(self.last_priority as u64))
            }
            p if p == PropertyIdentifier::WRITE_STATUS => {
                Ok(PropertyValue::Enumerated(self.write_status))
            }
            p if p == PropertyIdentifier::CHANNEL_NUMBER => {
                Ok(PropertyValue::Unsigned(self.channel_number as u64))
            }
            p if p == PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => Ok(
                PropertyValue::Unsigned(self.list_of_object_property_references_count as u64),
            ),
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
        // PRESENT_VALUE — write the channel value and update last_priority
        if property == PropertyIdentifier::PRESENT_VALUE {
            if let PropertyValue::Unsigned(v) = value {
                self.present_value = common::u64_to_u32(v)?;
                self.last_priority = priority.unwrap_or(16) as u32;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        // CHANNEL_NUMBER
        if property == PropertyIdentifier::CHANNEL_NUMBER {
            if let PropertyValue::Unsigned(v) = value {
                self.channel_number = common::u64_to_u32(v)?;
                return Ok(());
            }
            return Err(common::invalid_data_type_error());
        }

        if let Some(result) =
            common::write_out_of_service(&mut self.out_of_service, property, &value)
        {
            return result;
        }
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        Err(common::write_access_denied_error())
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::LAST_PRIORITY,
            PropertyIdentifier::WRITE_STATUS,
            PropertyIdentifier::CHANNEL_NUMBER,
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- LightingOutputObject ---

    #[test]
    fn lighting_output_create() {
        let obj = LightingOutputObject::new(1, "LO-1").unwrap();
        assert_eq!(obj.object_name(), "LO-1");
        assert_eq!(
            obj.object_identifier().object_type(),
            ObjectType::LIGHTING_OUTPUT
        );
        assert_eq!(obj.object_identifier().instance_number(), 1);
    }

    #[test]
    fn lighting_output_read_present_value() {
        let obj = LightingOutputObject::new(1, "LO-1").unwrap();
        let pv = obj.read_property(PropertyIdentifier::PRESENT_VALUE, None);
        assert_eq!(pv.unwrap(), PropertyValue::Real(0.0));
    }

    #[test]
    fn lighting_output_read_object_type() {
        let obj = LightingOutputObject::new(1, "LO-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::LIGHTING_OUTPUT.to_raw())
        );
    }

    #[test]
    fn lighting_output_write_pv_commandable() {
        let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
        // Write at priority 8
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(75.0),
            Some(8),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Real(75.0));

        // Write at priority 1 (higher) overrides
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(1),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Real(50.0));

        // Relinquish priority 1 — falls back to priority 8 value
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(1),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Real(75.0));
    }

    #[test]
    fn lighting_output_pv_out_of_range() {
        let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
        let result = obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(101.0),
            Some(16),
        );
        assert!(result.is_err());

        let result = obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(-1.0),
            Some(16),
        );
        assert!(result.is_err());
    }

    #[test]
    fn lighting_output_priority_array_read() {
        let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Real(50.0),
            Some(8),
        )
        .unwrap();

        // Read array size (index 0)
        let size = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
            .unwrap();
        assert_eq!(size, PropertyValue::Unsigned(16));

        // Read slot 8
        let slot = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(8))
            .unwrap();
        assert_eq!(slot, PropertyValue::Real(50.0));

        // Read empty slot 1
        let slot = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
            .unwrap();
        assert_eq!(slot, PropertyValue::Null);
    }

    #[test]
    fn lighting_output_priority_array_direct_write() {
        let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(5),
            PropertyValue::Real(33.0),
            None,
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Real(33.0));
    }

    #[test]
    fn lighting_output_relinquish_default() {
        let obj = LightingOutputObject::new(1, "LO-1").unwrap();
        let rd = obj
            .read_property(PropertyIdentifier::RELINQUISH_DEFAULT, None)
            .unwrap();
        assert_eq!(rd, PropertyValue::Real(0.0));
    }

    #[test]
    fn lighting_output_lighting_properties() {
        let mut obj = LightingOutputObject::new(1, "LO-1").unwrap();

        // TRACKING_VALUE
        let tv = obj
            .read_property(PropertyIdentifier::TRACKING_VALUE, None)
            .unwrap();
        assert_eq!(tv, PropertyValue::Real(0.0));

        // LIGHTING_COMMAND
        let lc = obj
            .read_property(PropertyIdentifier::LIGHTING_COMMAND, None)
            .unwrap();
        assert_eq!(lc, PropertyValue::OctetString(vec![]));

        // Write LIGHTING_COMMAND
        obj.write_property(
            PropertyIdentifier::LIGHTING_COMMAND,
            None,
            PropertyValue::OctetString(vec![0x01, 0x02]),
            None,
        )
        .unwrap();
        let lc = obj
            .read_property(PropertyIdentifier::LIGHTING_COMMAND, None)
            .unwrap();
        assert_eq!(lc, PropertyValue::OctetString(vec![0x01, 0x02]));

        // LIGHTING_COMMAND_DEFAULT_PRIORITY
        let lcdp = obj
            .read_property(PropertyIdentifier::LIGHTING_COMMAND_DEFAULT_PRIORITY, None)
            .unwrap();
        assert_eq!(lcdp, PropertyValue::Unsigned(16));

        // IN_PROGRESS
        let ip = obj
            .read_property(PropertyIdentifier::IN_PROGRESS, None)
            .unwrap();
        assert_eq!(ip, PropertyValue::Enumerated(0));

        // BLINK_WARN_ENABLE
        let bwe = obj
            .read_property(PropertyIdentifier::BLINK_WARN_ENABLE, None)
            .unwrap();
        assert_eq!(bwe, PropertyValue::Boolean(false));

        // EGRESS_TIME
        let et = obj
            .read_property(PropertyIdentifier::EGRESS_TIME, None)
            .unwrap();
        assert_eq!(et, PropertyValue::Unsigned(0));

        // EGRESS_ACTIVE
        let ea = obj
            .read_property(PropertyIdentifier::EGRESS_ACTIVE, None)
            .unwrap();
        assert_eq!(ea, PropertyValue::Boolean(false));
    }

    #[test]
    fn lighting_output_property_list() {
        let obj = LightingOutputObject::new(1, "LO-1").unwrap();
        let props = obj.property_list();
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::TRACKING_VALUE));
        assert!(props.contains(&PropertyIdentifier::LIGHTING_COMMAND));
        assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
        assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
    }

    // --- BinaryLightingOutputObject ---

    #[test]
    fn binary_lighting_output_create() {
        let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        assert_eq!(obj.object_name(), "BLO-1");
        assert_eq!(
            obj.object_identifier().object_type(),
            ObjectType::BINARY_LIGHTING_OUTPUT
        );
        assert_eq!(obj.object_identifier().instance_number(), 1);
    }

    #[test]
    fn binary_lighting_output_read_present_value() {
        let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        let pv = obj.read_property(PropertyIdentifier::PRESENT_VALUE, None);
        assert_eq!(pv.unwrap(), PropertyValue::Enumerated(0)); // off
    }

    #[test]
    fn binary_lighting_output_read_object_type() {
        let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(
            ot,
            PropertyValue::Enumerated(ObjectType::BINARY_LIGHTING_OUTPUT.to_raw())
        );
    }

    #[test]
    fn binary_lighting_output_write_pv_commandable() {
        let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        // Write on (1) at priority 8
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(8),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Enumerated(1));

        // Write warn (2) at priority 1 overrides
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(2),
            Some(1),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Enumerated(2));

        // Relinquish priority 1 — falls back to priority 8 (on)
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Null,
            Some(1),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Enumerated(1));
    }

    #[test]
    fn binary_lighting_output_pv_out_of_range() {
        let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        let result = obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(5), // > MAX_PV
            Some(16),
        );
        assert!(result.is_err());
    }

    #[test]
    fn binary_lighting_output_all_valid_pv_values() {
        let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        for val in 0..=4 {
            obj.write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(val),
                Some(16),
            )
            .unwrap();
            let pv = obj
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap();
            assert_eq!(pv, PropertyValue::Enumerated(val));
        }
    }

    #[test]
    fn binary_lighting_output_priority_array() {
        let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(1),
            Some(5),
        )
        .unwrap();

        // Read array size
        let size = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(0))
            .unwrap();
        assert_eq!(size, PropertyValue::Unsigned(16));

        // Read slot 5
        let slot = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(5))
            .unwrap();
        assert_eq!(slot, PropertyValue::Enumerated(1));

        // Read empty slot 1
        let slot = obj
            .read_property(PropertyIdentifier::PRIORITY_ARRAY, Some(1))
            .unwrap();
        assert_eq!(slot, PropertyValue::Null);
    }

    #[test]
    fn binary_lighting_output_priority_array_direct_write() {
        let mut obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        obj.write_property(
            PropertyIdentifier::PRIORITY_ARRAY,
            Some(3),
            PropertyValue::Enumerated(4), // fade-on
            None,
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Enumerated(4));
    }

    #[test]
    fn binary_lighting_output_property_list() {
        let obj = BinaryLightingOutputObject::new(1, "BLO-1").unwrap();
        let props = obj.property_list();
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::BLINK_WARN_ENABLE));
        assert!(props.contains(&PropertyIdentifier::EGRESS_TIME));
        assert!(props.contains(&PropertyIdentifier::PRIORITY_ARRAY));
        assert!(props.contains(&PropertyIdentifier::RELINQUISH_DEFAULT));
    }

    // --- ChannelObject ---

    #[test]
    fn channel_create() {
        let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        assert_eq!(obj.object_name(), "CH-1");
        assert_eq!(obj.object_identifier().object_type(), ObjectType::CHANNEL);
        assert_eq!(obj.object_identifier().instance_number(), 1);
    }

    #[test]
    fn channel_read_present_value() {
        let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        let pv = obj.read_property(PropertyIdentifier::PRESENT_VALUE, None);
        assert_eq!(pv.unwrap(), PropertyValue::Unsigned(0));
    }

    #[test]
    fn channel_read_object_type() {
        let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        let ot = obj
            .read_property(PropertyIdentifier::OBJECT_TYPE, None)
            .unwrap();
        assert_eq!(ot, PropertyValue::Enumerated(ObjectType::CHANNEL.to_raw()));
    }

    #[test]
    fn channel_write_present_value() {
        let mut obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(42),
            Some(8),
        )
        .unwrap();
        let pv = obj
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(pv, PropertyValue::Unsigned(42));

        // Verify last_priority was updated
        let lp = obj
            .read_property(PropertyIdentifier::LAST_PRIORITY, None)
            .unwrap();
        assert_eq!(lp, PropertyValue::Unsigned(8));
    }

    #[test]
    fn channel_read_channel_number() {
        let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        let cn = obj
            .read_property(PropertyIdentifier::CHANNEL_NUMBER, None)
            .unwrap();
        assert_eq!(cn, PropertyValue::Unsigned(5));
    }

    #[test]
    fn channel_write_channel_number() {
        let mut obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        obj.write_property(
            PropertyIdentifier::CHANNEL_NUMBER,
            None,
            PropertyValue::Unsigned(10),
            None,
        )
        .unwrap();
        let cn = obj
            .read_property(PropertyIdentifier::CHANNEL_NUMBER, None)
            .unwrap();
        assert_eq!(cn, PropertyValue::Unsigned(10));
    }

    #[test]
    fn channel_read_write_status() {
        let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        let ws = obj
            .read_property(PropertyIdentifier::WRITE_STATUS, None)
            .unwrap();
        assert_eq!(ws, PropertyValue::Enumerated(0)); // idle
    }

    #[test]
    fn channel_read_last_priority_default() {
        let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        let lp = obj
            .read_property(PropertyIdentifier::LAST_PRIORITY, None)
            .unwrap();
        assert_eq!(lp, PropertyValue::Unsigned(16)); // default priority
    }

    #[test]
    fn channel_property_list() {
        let obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        let props = obj.property_list();
        assert!(props.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(props.contains(&PropertyIdentifier::LAST_PRIORITY));
        assert!(props.contains(&PropertyIdentifier::WRITE_STATUS));
        assert!(props.contains(&PropertyIdentifier::CHANNEL_NUMBER));
        assert!(props.contains(&PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES));
    }

    #[test]
    fn channel_write_pv_default_priority() {
        let mut obj = ChannelObject::new(1, "CH-1", 5).unwrap();
        // Write without explicit priority — defaults to 16
        obj.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(99),
            None,
        )
        .unwrap();
        let lp = obj
            .read_property(PropertyIdentifier::LAST_PRIORITY, None)
            .unwrap();
        assert_eq!(lp, PropertyValue::Unsigned(16));
    }
}
