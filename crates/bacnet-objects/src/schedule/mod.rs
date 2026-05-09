//! Schedule (type 17) and Calendar (type 6) objects per ASHRAE 135-2020.

use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDateRange, BACnetObjectPropertyReference, BACnetSpecialEvent,
    BACnetTimeValue,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, StatusFlags};
use std::borrow::Cow;

use crate::common::read_property_list_property;
use crate::traits::BACnetObject;

// ---------------------------------------------------------------------------
// Calendar (type 6)
// ---------------------------------------------------------------------------

/// BACnet Calendar object.
///
/// Present_Value is Boolean — true when today matches one of the date_list
/// entries. The application is responsible for evaluating the date_list and
/// calling `set_present_value()`.
pub struct CalendarObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: bool,
    status_flags: StatusFlags,
    date_list: Vec<BACnetCalendarEntry>,
}

impl CalendarObject {
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::CALENDAR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: false,
            status_flags: StatusFlags::empty(),
            date_list: Vec::new(),
        })
    }

    /// Application sets this based on date-list evaluation.
    pub fn set_present_value(&mut self, value: bool) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Append a calendar entry to the date_list.
    pub fn add_date_entry(&mut self, entry: BACnetCalendarEntry) {
        self.date_list.push(entry);
    }

    /// Remove all entries from the date_list.
    pub fn clear_date_list(&mut self) {
        self.date_list.clear();
    }
}

impl BACnetObject for CalendarObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::CALENDAR.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => {
                Ok(PropertyValue::Boolean(self.present_value))
            }
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.status_flags.bits() << 4],
            }),
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
            p if p == PropertyIdentifier::OUT_OF_SERVICE => Ok(PropertyValue::Boolean(false)),
            p if p == PropertyIdentifier::DATE_LIST => Ok(PropertyValue::List(
                self.date_list
                    .iter()
                    .map(|entry| match entry {
                        BACnetCalendarEntry::Date(d) => PropertyValue::Date(*d),
                        BACnetCalendarEntry::DateRange(dr) => {
                            PropertyValue::OctetString(dr.encode().to_vec())
                        }
                        BACnetCalendarEntry::WeekNDay(wnd) => {
                            PropertyValue::OctetString(wnd.encode().to_vec())
                        }
                    })
                    .collect(),
            )),
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(s) = value {
                self.description = s;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
            });
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::DATE_LIST,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::OUT_OF_SERVICE,
        ];
        Cow::Borrowed(PROPS)
    }
}

// ---------------------------------------------------------------------------
// Schedule (type 17)
// ---------------------------------------------------------------------------

/// BACnet Schedule object.
///
/// Stores schedule configuration. The application is responsible for
/// evaluating the weekly/exception schedule and calling `set_present_value()`.
/// Present_Value data type matches schedule_default.
pub struct ScheduleObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    present_value: PropertyValue,
    schedule_default: PropertyValue,
    out_of_service: bool,
    reliability: u32,
    status_flags: StatusFlags,
    /// 7-day weekly schedule: index 0 = Monday, index 6 = Sunday.
    weekly_schedule: [Vec<BACnetTimeValue>; 7],
    exception_schedule: Vec<BACnetSpecialEvent>,
    effective_period: Option<BACnetDateRange>,
    list_of_object_property_references: Vec<BACnetObjectPropertyReference>,
    /// Priority for writing to referenced objects (1-16).
    priority_for_writing: u8,
}

impl ScheduleObject {
    pub fn new(
        instance: u32,
        name: impl Into<String>,
        schedule_default: PropertyValue,
    ) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::SCHEDULE, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            present_value: schedule_default.clone(),
            schedule_default,
            out_of_service: false,
            reliability: 0,
            status_flags: StatusFlags::empty(),
            weekly_schedule: [vec![], vec![], vec![], vec![], vec![], vec![], vec![]],
            exception_schedule: Vec::new(),
            effective_period: None,
            list_of_object_property_references: Vec::new(),
            priority_for_writing: 16, // default: lowest priority
        })
    }

    /// Application sets this based on schedule evaluation.
    pub fn set_present_value(&mut self, value: PropertyValue) {
        self.present_value = value;
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Set time-value entries for a given day (0=Monday .. 6=Sunday).
    pub fn set_weekly_schedule(&mut self, day_index: usize, entries: Vec<BACnetTimeValue>) {
        if day_index < 7 {
            self.weekly_schedule[day_index] = entries;
        }
    }

    /// Append a special event to the exception schedule.
    pub fn add_exception(&mut self, event: BACnetSpecialEvent) {
        self.exception_schedule.push(event);
    }

    /// Set the effective period for this schedule.
    pub fn set_effective_period(&mut self, period: BACnetDateRange) {
        self.effective_period = Some(period);
    }

    /// Append an object property reference to the list.
    pub fn add_object_property_reference(&mut self, r: BACnetObjectPropertyReference) {
        self.list_of_object_property_references.push(r);
    }

    /// Read the current present_value.
    pub fn present_value(&self) -> &PropertyValue {
        &self.present_value
    }

    /// Evaluate the schedule for the given day and time.
    ///
    /// Returns the current effective value (from exception, weekly, or default).
    /// `day_of_week`: 0=Monday .. 6=Sunday.
    pub fn evaluate(&self, day_of_week: u8, hour: u8, minute: u8) -> PropertyValue {
        if self.out_of_service {
            return self.present_value.clone();
        }

        // 1. Check exception_schedule first (highest priority = lowest number)
        let mut best_exception: Option<(u8, &[u8])> = None;
        for event in &self.exception_schedule {
            if let Some(raw) = find_active_time_value(&event.list_of_time_values, hour, minute) {
                match best_exception {
                    None => best_exception = Some((event.event_priority, raw)),
                    Some((p, _)) if event.event_priority < p => {
                        best_exception = Some((event.event_priority, raw));
                    }
                    _ => {}
                }
            }
        }
        if let Some((_, raw)) = best_exception {
            return PropertyValue::OctetString(raw.to_vec());
        }

        // 2. Check weekly_schedule[day_of_week]
        if (day_of_week as usize) < 7 {
            if let Some(raw) =
                find_active_time_value(&self.weekly_schedule[day_of_week as usize], hour, minute)
            {
                return PropertyValue::OctetString(raw.to_vec());
            }
        }

        // 3. Fall back to schedule_default
        self.schedule_default.clone()
    }
}

/// Find the last time-value entry whose time is at or before (hour, minute).
///
/// Entries are expected to be in chronological order per the BACnet spec.
fn find_active_time_value(entries: &[BACnetTimeValue], hour: u8, minute: u8) -> Option<&[u8]> {
    let mut result = None;
    for tv in entries {
        let t = &tv.time;
        if t.hour < hour || (t.hour == hour && t.minute <= minute) {
            result = Some(tv.value.as_slice());
        }
    }
    result
}

impl BACnetObject for ScheduleObject {
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
        match property {
            p if p == PropertyIdentifier::OBJECT_IDENTIFIER => {
                Ok(PropertyValue::ObjectIdentifier(self.oid))
            }
            p if p == PropertyIdentifier::OBJECT_NAME => {
                Ok(PropertyValue::CharacterString(self.name.clone()))
            }
            p if p == PropertyIdentifier::DESCRIPTION => {
                Ok(PropertyValue::CharacterString(self.description.clone()))
            }
            p if p == PropertyIdentifier::OBJECT_TYPE => {
                Ok(PropertyValue::Enumerated(ObjectType::SCHEDULE.to_raw()))
            }
            p if p == PropertyIdentifier::PRESENT_VALUE => Ok(self.present_value.clone()),
            p if p == PropertyIdentifier::SCHEDULE_DEFAULT => Ok(self.schedule_default.clone()),
            p if p == PropertyIdentifier::STATUS_FLAGS => Ok(PropertyValue::BitString {
                unused_bits: 4,
                data: vec![self.status_flags.bits() << 4],
            }),
            p if p == PropertyIdentifier::EVENT_STATE => Ok(PropertyValue::Enumerated(0)),
            p if p == PropertyIdentifier::RELIABILITY => {
                Ok(PropertyValue::Enumerated(self.reliability))
            }
            p if p == PropertyIdentifier::OUT_OF_SERVICE => {
                Ok(PropertyValue::Boolean(self.out_of_service))
            }
            p if p == PropertyIdentifier::WEEKLY_SCHEDULE => match array_index {
                None => {
                    let days: Vec<PropertyValue> = self
                        .weekly_schedule
                        .iter()
                        .map(|day| {
                            PropertyValue::List(
                                day.iter()
                                    .map(|tv| {
                                        PropertyValue::List(vec![
                                            PropertyValue::Time(tv.time),
                                            PropertyValue::OctetString(tv.value.clone()),
                                        ])
                                    })
                                    .collect(),
                            )
                        })
                        .collect();
                    Ok(PropertyValue::List(days))
                }
                Some(0) => Ok(PropertyValue::Unsigned(7)),
                Some(idx) if (1..=7).contains(&idx) => {
                    let day = &self.weekly_schedule[(idx - 1) as usize];
                    Ok(PropertyValue::List(
                        day.iter()
                            .map(|tv| {
                                PropertyValue::List(vec![
                                    PropertyValue::Time(tv.time),
                                    PropertyValue::OctetString(tv.value.clone()),
                                ])
                            })
                            .collect(),
                    ))
                }
                _ => Err(Error::Protocol {
                    class: ErrorClass::PROPERTY.to_raw() as u32,
                    code: ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32,
                }),
            },
            p if p == PropertyIdentifier::EXCEPTION_SCHEDULE => match array_index {
                None => {
                    let events: Vec<PropertyValue> = self
                        .exception_schedule
                        .iter()
                        .map(|ev| {
                            let tvs: Vec<PropertyValue> = ev
                                .list_of_time_values
                                .iter()
                                .map(|tv| {
                                    PropertyValue::List(vec![
                                        PropertyValue::Time(tv.time),
                                        PropertyValue::OctetString(tv.value.clone()),
                                    ])
                                })
                                .collect();
                            PropertyValue::List(vec![
                                PropertyValue::Unsigned(ev.event_priority as u64),
                                PropertyValue::List(tvs),
                            ])
                        })
                        .collect();
                    Ok(PropertyValue::List(events))
                }
                Some(0) => Ok(PropertyValue::Unsigned(self.exception_schedule.len() as u64)),
                Some(i) => {
                    let idx = (i as usize).checked_sub(1).ok_or(Error::Protocol {
                        class: ErrorClass::PROPERTY.to_raw() as u32,
                        code: ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32,
                    })?;
                    let ev = self.exception_schedule.get(idx).ok_or(Error::Protocol {
                        class: ErrorClass::PROPERTY.to_raw() as u32,
                        code: ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32,
                    })?;
                    let tvs: Vec<PropertyValue> = ev
                        .list_of_time_values
                        .iter()
                        .map(|tv| {
                            PropertyValue::List(vec![
                                PropertyValue::Time(tv.time),
                                PropertyValue::OctetString(tv.value.clone()),
                            ])
                        })
                        .collect();
                    Ok(PropertyValue::List(vec![
                        PropertyValue::Unsigned(ev.event_priority as u64),
                        PropertyValue::List(tvs),
                    ]))
                }
            },
            p if p == PropertyIdentifier::EFFECTIVE_PERIOD => match &self.effective_period {
                Some(dr) => Ok(PropertyValue::OctetString(dr.encode().to_vec())),
                None => Ok(PropertyValue::Null),
            },
            p if p == PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES => {
                Ok(PropertyValue::List(
                    self.list_of_object_property_references
                        .iter()
                        .map(|r| {
                            PropertyValue::List(vec![
                                PropertyValue::ObjectIdentifier(r.object_identifier),
                                PropertyValue::Enumerated(r.property_identifier),
                            ])
                        })
                        .collect(),
                ))
            }
            p if p == PropertyIdentifier::PRIORITY_FOR_WRITING => {
                Ok(PropertyValue::Unsigned(self.priority_for_writing as u64))
            }
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if property == PropertyIdentifier::SCHEDULE_DEFAULT {
            self.schedule_default = value;
            return Ok(());
        }
        if property == PropertyIdentifier::RELIABILITY {
            if let PropertyValue::Enumerated(v) = value {
                self.reliability = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::OUT_OF_SERVICE {
            if let PropertyValue::Boolean(v) = value {
                self.out_of_service = v;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        if property == PropertyIdentifier::DESCRIPTION {
            if let PropertyValue::CharacterString(s) = value {
                self.description = s;
                return Ok(());
            }
            return Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::INVALID_DATA_TYPE.to_raw() as u32,
            });
        }
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        static PROPS: &[PropertyIdentifier] = &[
            PropertyIdentifier::OBJECT_IDENTIFIER,
            PropertyIdentifier::OBJECT_NAME,
            PropertyIdentifier::DESCRIPTION,
            PropertyIdentifier::OBJECT_TYPE,
            PropertyIdentifier::PRESENT_VALUE,
            PropertyIdentifier::SCHEDULE_DEFAULT,
            PropertyIdentifier::WEEKLY_SCHEDULE,
            PropertyIdentifier::EXCEPTION_SCHEDULE,
            PropertyIdentifier::EFFECTIVE_PERIOD,
            PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES,
            PropertyIdentifier::STATUS_FLAGS,
            PropertyIdentifier::EVENT_STATE,
            PropertyIdentifier::RELIABILITY,
            PropertyIdentifier::OUT_OF_SERVICE,
        ];
        Cow::Borrowed(PROPS)
    }

    fn tick_schedule(
        &mut self,
        day_of_week: u8,
        hour: u8,
        minute: u8,
    ) -> Option<(PropertyValue, Vec<(ObjectIdentifier, u32)>)> {
        if self.out_of_service || self.list_of_object_property_references.is_empty() {
            return None;
        }

        let new_value = self.evaluate(day_of_week, hour, minute);
        if new_value == self.present_value {
            return None;
        }

        self.present_value = new_value.clone();

        let refs = self
            .list_of_object_property_references
            .iter()
            .map(|r| (r.object_identifier, r.property_identifier))
            .collect();

        Some((new_value, refs))
    }
}

#[cfg(test)]
mod tests;
