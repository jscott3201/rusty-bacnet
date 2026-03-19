//! Timer object (type 31) per ASHRAE 135-2020 Clause 12.
//!
//! The Timer object represents a countdown or count-up timer. Its present value
//! is an Enumerated representing the timer state: 0=idle, 1=running, 2=expired.

use bacnet_types::enums::{ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, PropertyValue, StatusFlags, Time};
use std::borrow::Cow;

use crate::common::{self, read_common_properties};
use crate::traits::BACnetObject;

/// Timer state enumeration values.
const TIMER_STATE_IDLE: u32 = 0;
const TIMER_STATE_RUNNING: u32 = 1;
const TIMER_STATE_EXPIRED: u32 = 2;

/// BACnet Timer object — countdown/count-up timer.
pub struct TimerObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: u32,
    timer_running: bool,
    initial_timeout: u64,
    update_time: (Date, Time),
    expiration_time: (Date, Time),
    status_flags: StatusFlags,
    /// Event_State: 0 = NORMAL.
    event_state: u32,
    out_of_service: bool,
    reliability: u32,
}

impl TimerObject {
    /// Create a new Timer object with default values (idle state).
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::TIMER, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: TIMER_STATE_IDLE,
            timer_running: false,
            initial_timeout: 0,
            update_time: (
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
            expiration_time: (
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

    /// Start the timer — transitions to running state.
    pub fn start(&mut self) {
        self.present_value = TIMER_STATE_RUNNING;
        self.timer_running = true;
    }

    /// Stop the timer — transitions to idle state.
    pub fn stop(&mut self) {
        self.present_value = TIMER_STATE_IDLE;
        self.timer_running = false;
    }

    /// Set the initial timeout in milliseconds.
    pub fn set_initial_timeout(&mut self, timeout_ms: u64) {
        self.initial_timeout = timeout_ms;
    }

    /// Set the update time as a (Date, Time) tuple.
    pub fn set_update_time(&mut self, date: Date, time: Time) {
        self.update_time = (date, time);
    }

    /// Set the expiration time as a (Date, Time) tuple.
    pub fn set_expiration_time(&mut self, date: Date, time: Time) {
        self.expiration_time = (date, time);
    }
}

impl BACnetObject for TimerObject {
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
                Ok(PropertyValue::Enumerated(ObjectType::TIMER.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::TIMER_STATE => {
                Ok(PropertyValue::Enumerated(self.present_value))
            }
            p if p == PropertyIdentifier::TIMER_RUNNING => {
                Ok(PropertyValue::Boolean(self.timer_running))
            }
            p if p == PropertyIdentifier::INITIAL_TIMEOUT => {
                Ok(PropertyValue::Unsigned(self.initial_timeout))
            }
            p if p == PropertyIdentifier::UPDATE_TIME => Ok(PropertyValue::List(vec![
                PropertyValue::Date(self.update_time.0),
                PropertyValue::Time(self.update_time.1),
            ])),
            p if p == PropertyIdentifier::EXPIRATION_TIME => Ok(PropertyValue::List(vec![
                PropertyValue::Date(self.expiration_time.0),
                PropertyValue::Time(self.expiration_time.1),
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
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                if let PropertyValue::Enumerated(v) = value {
                    if v > TIMER_STATE_EXPIRED {
                        return Err(common::value_out_of_range_error());
                    }
                    self.present_value = v;
                    self.timer_running = v == TIMER_STATE_RUNNING;
                    Ok(())
                } else {
                    Err(common::invalid_data_type_error())
                }
            }
            p if p == PropertyIdentifier::INITIAL_TIMEOUT => {
                if let PropertyValue::Unsigned(v) = value {
                    self.initial_timeout = v;
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
            PropertyIdentifier::TIMER_STATE,
            PropertyIdentifier::TIMER_RUNNING,
            PropertyIdentifier::INITIAL_TIMEOUT,
            PropertyIdentifier::UPDATE_TIME,
            PropertyIdentifier::EXPIRATION_TIME,
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
    fn timer_create_and_read_defaults() {
        let timer = TimerObject::new(1, "TMR-1").unwrap();
        assert_eq!(timer.object_name(), "TMR-1");
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(TIMER_STATE_IDLE)
        );
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::TIMER_RUNNING, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
    }

    #[test]
    fn timer_object_type() {
        let timer = TimerObject::new(1, "TMR-1").unwrap();
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::OBJECT_TYPE, None)
                .unwrap(),
            PropertyValue::Enumerated(ObjectType::TIMER.to_raw())
        );
    }

    #[test]
    fn timer_start_stop() {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        timer.start();
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(TIMER_STATE_RUNNING)
        );
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::TIMER_RUNNING, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );

        timer.stop();
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(TIMER_STATE_IDLE)
        );
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::TIMER_RUNNING, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
    }

    #[test]
    fn timer_write_present_value() {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        timer
            .write_property(
                PropertyIdentifier::PRESENT_VALUE,
                None,
                PropertyValue::Enumerated(TIMER_STATE_RUNNING),
                None,
            )
            .unwrap();
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::PRESENT_VALUE, None)
                .unwrap(),
            PropertyValue::Enumerated(TIMER_STATE_RUNNING)
        );
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::TIMER_RUNNING, None)
                .unwrap(),
            PropertyValue::Boolean(true)
        );
    }

    #[test]
    fn timer_write_present_value_out_of_range() {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        let result = timer.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Enumerated(99),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn timer_write_present_value_wrong_type() {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        let result = timer.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Unsigned(1),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn timer_read_initial_timeout() {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        timer.set_initial_timeout(5000);
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::INITIAL_TIMEOUT, None)
                .unwrap(),
            PropertyValue::Unsigned(5000)
        );
    }

    #[test]
    fn timer_write_initial_timeout() {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        timer
            .write_property(
                PropertyIdentifier::INITIAL_TIMEOUT,
                None,
                PropertyValue::Unsigned(10000),
                None,
            )
            .unwrap();
        assert_eq!(
            timer
                .read_property(PropertyIdentifier::INITIAL_TIMEOUT, None)
                .unwrap(),
            PropertyValue::Unsigned(10000)
        );
    }

    #[test]
    fn timer_read_update_time() {
        let timer = TimerObject::new(1, "TMR-1").unwrap();
        let val = timer
            .read_property(PropertyIdentifier::UPDATE_TIME, None)
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
    fn timer_read_timer_state_matches_pv() {
        let mut timer = TimerObject::new(1, "TMR-1").unwrap();
        timer.start();
        let pv = timer
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        let ts = timer
            .read_property(PropertyIdentifier::TIMER_STATE, None)
            .unwrap();
        assert_eq!(pv, ts);
    }

    #[test]
    fn timer_property_list() {
        let timer = TimerObject::new(1, "TMR-1").unwrap();
        let list = timer.property_list();
        assert!(list.contains(&PropertyIdentifier::PRESENT_VALUE));
        assert!(list.contains(&PropertyIdentifier::TIMER_STATE));
        assert!(list.contains(&PropertyIdentifier::TIMER_RUNNING));
        assert!(list.contains(&PropertyIdentifier::INITIAL_TIMEOUT));
        assert!(list.contains(&PropertyIdentifier::UPDATE_TIME));
        assert!(list.contains(&PropertyIdentifier::EXPIRATION_TIME));
    }
}
