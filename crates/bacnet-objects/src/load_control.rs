//! Load Control object (type 28) per ASHRAE 135-2020 Clause 12.
//!
//! The Load Control object provides a standard interface for demand-response
//! load shedding. It tracks requested, expected, and actual shed levels.

use bacnet_types::constructed::BACnetShedLevel;
use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// BACnet Load Control object — demand-response load shedding.
pub struct LoadControlObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    /// Present value: enumerated shed state (0=shed-inactive, 1=shed-request-pending,
    /// 2=shed-compliant, 3=shed-non-compliant).
    present_value: u32,
    requested_shed_level: BACnetShedLevel,
    expected_shed_level: BACnetShedLevel,
    actual_shed_level: BACnetShedLevel,
    shed_duration: u64,
    start_time: (Date, Time),
    status_flags: StatusFlags,
    /// Event_State: 0 = NORMAL.
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
}

impl LoadControlObject {
    /// Create a new Load Control object with default values.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::LOAD_CONTROL, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: 0,
            requested_shed_level: BACnetShedLevel::Percent(0),
            expected_shed_level: BACnetShedLevel::Percent(0),
            actual_shed_level: BACnetShedLevel::Percent(0),
            shed_duration: 0,
            start_time: (
                Date {
                    year: 0xFF,
                    month: 0xFF,
                    day: 0xFF,
                    day_of_week: 0xFF,
                },
                Time {
                    hour: 0xFF,
                    minute: 0xFF,
                    second: 0xFF,
                    hundredths: 0xFF,
                },
            ),
            status_flags: StatusFlags::empty(),
            event_state: 0, // NORMAL
            out_of_service: false,
            reliability: 0,
        })
    }

    /// Set the requested shed level.
    pub fn set_requested_shed_level(&mut self, level: BACnetShedLevel) {
        self.requested_shed_level = level;
    }

    /// Set the actual shed level.
    pub fn set_actual_shed_level(&mut self, level: BACnetShedLevel) {
        self.actual_shed_level = level;
    }

    /// Encode a BACnetShedLevel to a PropertyValue.
    fn shed_level_to_property(level: &BACnetShedLevel) -> PropertyValue {
        match level {
            BACnetShedLevel::Percent(v) => {
                PropertyValue::List(vec![PropertyValue::Unsigned(*v as u64)])
            }
            BACnetShedLevel::Level(v) => {
                PropertyValue::List(vec![PropertyValue::Unsigned(*v as u64)])
            }
            BACnetShedLevel::Amount(v) => PropertyValue::List(vec![PropertyValue::Real(*v)]),
        }
    }
}

impl BACnetObject for LoadControlObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::LOAD_CONTROL.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::REQUESTED_SHED_LEVEL => {
                Ok(Self::shed_level_to_property(&self.requested_shed_level))
            }
            p if p == PropertyIdentifier::EXPECTED_SHED_LEVEL => {
                Ok(Self::shed_level_to_property(&self.expected_shed_level))
            }
            p if p == PropertyIdentifier::ACTUAL_SHED_LEVEL => {
                Ok(Self::shed_level_to_property(&self.actual_shed_level))
            }
            p if p == PropertyIdentifier::SHED_DURATION => {
                Ok(PropertyValue::Unsigned(self.shed_duration))
            }
            p if p == PropertyIdentifier::START_TIME => Ok(PropertyValue::List(vec![
                PropertyValue::Date(self.start_time.0),
                PropertyValue::Time(self.start_time.1),
            ])),
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
            p if p == PropertyIdentifier::SHED_DURATION => {
                if let PropertyValue::Unsigned(v) = value {
                    self.shed_duration = v;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::REQUESTED_SHED_LEVEL => {
                // Accept List with a single Unsigned (percent/level) or Real (amount)
                if let PropertyValue::List(ref items) = value {
                    if items.len() == 1 {
                        match &items[0] {
                            PropertyValue::Unsigned(v) => {
                                self.requested_shed_level =
                                    BACnetShedLevel::Percent(common::u64_to_u32(*v)?);
                                Ok(())
                            }
                            PropertyValue::Real(v) => {
                                common::reject_non_finite(*v)?;
                                self.requested_shed_level = BACnetShedLevel::Amount(*v);
                                Ok(())
                            }
                            _ => Err(common::invalid_data_type_error()),
                        }
                    } else {
                        Err(common::invalid_data_type_error())
                    }
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
            PropertyIdentifier::REQUESTED_SHED_LEVEL,
            PropertyIdentifier::EXPECTED_SHED_LEVEL,
            PropertyIdentifier::ACTUAL_SHED_LEVEL,
            PropertyIdentifier::SHED_DURATION,
            PropertyIdentifier::START_TIME,
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

    #[test]
    fn load_control_create_and_read_defaults() {
        let lc = LoadControlObject::new(1, "LC-1").unwrap();
        assert_eq!(lc.object_name(), "LC-1");
        assert_eq!(
            lc.read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(0)
        );
    }

    #[test]
    fn load_control_object_type() {
        let lc = LoadControlObject::new(1, "LC-1").unwrap();
        assert_eq!(
            lc.read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::LOAD_CONTROL.to_raw())
        );
    }

    #[test]
    fn load_control_read_shed_levels() {
        let lc = LoadControlObject::new(1, "LC-1").unwrap();
        // Default is Percent(0)
        assert_eq!(
            lc.read_property(PropertyIdentifier::REQUESTED_SHED_LEVEL, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(0)])
        );
        assert_eq!(
            lc.read_property(PropertyIdentifier::EXPECTED_SHED_LEVEL, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(0)])
        );
        assert_eq!(
            lc.read_property(PropertyIdentifier::ACTUAL_SHED_LEVEL, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(0)])
        );
    }

    #[test]
    fn load_control_set_requested_shed_level_amount() {
        let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
        lc.set_requested_shed_level(BACnetShedLevel::Amount(42.5));
        assert_eq!(
            lc.read_property(PropertyIdentifier::REQUESTED_SHED_LEVEL, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Real(42.5)])
        );
    }

    #[test]
    fn load_control_write_shed_duration() {
        let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
        lc.write_property(
            PropertyIdentifier::SHED_DURATION,
            None,
            PropertyValue::Unsigned(3600),
            None,
        )
        .unwrap();
        assert_eq!(
            lc.read_property(PropertyIdentifier::SHED_DURATION, None)
                .unwrap(),
            PropertyValue::Unsigned(3600)
        );
    }

    #[test]
    fn load_control_write_requested_shed_level() {
        let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
        lc.write_property(
            PropertyIdentifier::REQUESTED_SHED_LEVEL,
            None,
            PropertyValue::List(vec![PropertyValue::Unsigned(50)]),
            None,
        )
        .unwrap();
        assert_eq!(
            lc.read_property(PropertyIdentifier::REQUESTED_SHED_LEVEL, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Unsigned(50)])
        );
    }

    #[test]
    fn load_control_write_requested_shed_level_amount() {
        let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
        lc.write_property(
            PropertyIdentifier::REQUESTED_SHED_LEVEL,
            None,
            PropertyValue::List(vec![PropertyValue::Real(25.5)]),
            None,
        )
        .unwrap();
        assert_eq!(
            lc.read_property(PropertyIdentifier::REQUESTED_SHED_LEVEL, None)
                .unwrap(),
            PropertyValue::List(vec![PropertyValue::Real(25.5)])
        );
    }

    #[test]
    fn load_control_write_requested_shed_level_wrong_type() {
        let mut lc = LoadControlObject::new(1, "LC-1").unwrap();
        let result = lc.write_property(
            PropertyIdentifier::REQUESTED_SHED_LEVEL,
            None,
            PropertyValue::Unsigned(50),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn load_control_read_start_time() {
        let lc = LoadControlObject::new(1, "LC-1").unwrap();
        let val = lc
            .read_property(PropertyIdentifier::START_TIME, None)
            .unwrap();
        let unspec_date = Date {
            year: 0xFF,
            month: 0xFF,
            day: 0xFF,
            day_of_week: 0xFF,
        };
        let unspec_time = Time {
            hour: 0xFF,
            minute: 0xFF,
            second: 0xFF,
            hundredths: 0xFF,
        };
        assert_eq!(
            val,
            PropertyValue::List(vec![
                PropertyValue::Date(unspec_date),
                PropertyValue::Time(unspec_time),
            ])
        );
    }

    #[test]
    fn load_control_property_list() {
        let lc = LoadControlObject::new(1, "LC-1").unwrap();
        let list = lc.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::REQUESTED_SHED_LEVEL));
        assert!(list.contains(&PropertyIdentifier::EXPECTED_SHED_LEVEL));
        assert!(list.contains(&PropertyIdentifier::ACTUAL_SHED_LEVEL));
        assert!(list.contains(&PropertyIdentifier::SHED_DURATION));
        assert!(list.contains(&PropertyIdentifier::START_TIME));
    }
}
