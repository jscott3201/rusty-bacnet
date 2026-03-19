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
mod tests {
    use super::*;
    use bacnet_types::constructed::{
        BACnetCalendarEntry, BACnetDateRange, BACnetSpecialEvent, BACnetTimeValue, BACnetWeekNDay,
        SpecialEventPeriod,
    };
    use bacnet_types::primitives::{Date, Time};

    // --- Calendar ---

    #[test]
    fn calendar_read_present_value_default() {
        let cal = CalendarObject::new(1, "CAL-1").unwrap();
        let val = cal
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(false));
    }

    #[test]
    fn calendar_set_present_value() {
        let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
        cal.set_present_value(true);
        let val = cal
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Boolean(true));
    }

    #[test]
    fn calendar_write_present_value_denied() {
        let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
        let result = cal.write_property(
            PropertyIdentifier::PRESENT_VALUE,
            None,
            PropertyValue::Boolean(true),
            None,
        );
        assert!(result.is_err());
    }

    #[test]
    fn calendar_date_list_empty_by_default() {
        let cal = CalendarObject::new(1, "CAL-1").unwrap();
        let val = cal
            .read_property(PropertyIdentifier::DATE_LIST, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn calendar_date_list_add_and_read_entries() {
        let mut cal = CalendarObject::new(1, "CAL-1").unwrap();

        // Add a Date entry
        let d = Date {
            year: 124,
            month: 3,
            day: 15,
            day_of_week: 5,
        };
        cal.add_date_entry(BACnetCalendarEntry::Date(d));

        // Add a DateRange entry
        let dr = BACnetDateRange {
            start_date: Date {
                year: 124,
                month: 6,
                day: 1,
                day_of_week: 6,
            },
            end_date: Date {
                year: 124,
                month: 6,
                day: 30,
                day_of_week: 0,
            },
        };
        cal.add_date_entry(BACnetCalendarEntry::DateRange(dr.clone()));

        // Add a WeekNDay entry
        let wnd = BACnetWeekNDay {
            month: BACnetWeekNDay::ANY,
            week_of_month: BACnetWeekNDay::ANY,
            day_of_week: 1,
        };
        cal.add_date_entry(BACnetCalendarEntry::WeekNDay(wnd.clone()));

        let val = cal
            .read_property(PropertyIdentifier::DATE_LIST, None)
            .unwrap();

        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 3);
            assert_eq!(items[0], PropertyValue::Date(d));
            assert_eq!(items[1], PropertyValue::OctetString(dr.encode().to_vec()));
            assert_eq!(items[2], PropertyValue::OctetString(wnd.encode().to_vec()));
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn calendar_date_list_clear() {
        let mut cal = CalendarObject::new(1, "CAL-1").unwrap();
        let d = Date {
            year: 124,
            month: 1,
            day: 1,
            day_of_week: 1,
        };
        cal.add_date_entry(BACnetCalendarEntry::Date(d));
        // Confirm it was added
        let val = cal
            .read_property(PropertyIdentifier::DATE_LIST, None)
            .unwrap();
        if let PropertyValue::List(items) = &val {
            assert_eq!(items.len(), 1);
        } else {
            panic!("expected PropertyValue::List");
        }
        // Clear and verify empty
        cal.clear_date_list();
        let val = cal
            .read_property(PropertyIdentifier::DATE_LIST, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn calendar_property_list_contains_date_list() {
        let cal = CalendarObject::new(1, "CAL-1").unwrap();
        let props = cal.property_list();
        assert!(props.contains(&PropertyIdentifier::DATE_LIST));
    }

    // --- Schedule ---

    #[test]
    fn schedule_read_present_value_default() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let val = sched
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(72.0));
    }

    #[test]
    fn schedule_read_schedule_default() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let val = sched
            .read_property(PropertyIdentifier::SCHEDULE_DEFAULT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(72.0));
    }

    #[test]
    fn schedule_write_schedule_default() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        sched
            .write_property(
                PropertyIdentifier::SCHEDULE_DEFAULT,
                None,
                PropertyValue::Real(68.0),
                None,
            )
            .unwrap();
        let val = sched
            .read_property(PropertyIdentifier::SCHEDULE_DEFAULT, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(68.0));
    }

    #[test]
    fn schedule_set_present_value() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        sched.set_present_value(PropertyValue::Real(65.0));
        let val = sched
            .read_property(PropertyIdentifier::PRESENT_VALUE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Real(65.0));
    }

    // --- Schedule weekly_schedule ---

    fn make_time(hour: u8, minute: u8) -> Time {
        Time {
            hour,
            minute,
            second: 0,
            hundredths: 0,
        }
    }

    fn make_tv(hour: u8, minute: u8, raw_value: Vec<u8>) -> BACnetTimeValue {
        BACnetTimeValue {
            time: make_time(hour, minute),
            value: raw_value,
        }
    }

    #[test]
    fn schedule_weekly_schedule_empty_by_default() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let val = sched
            .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, None)
            .unwrap();
        if let PropertyValue::List(days) = val {
            assert_eq!(days.len(), 7);
            for day in &days {
                assert_eq!(*day, PropertyValue::List(vec![]));
            }
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn schedule_weekly_schedule_set_monday_read_no_index() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let entries = vec![
            make_tv(8, 0, vec![0x01]),  // 08:00
            make_tv(17, 0, vec![0x00]), // 17:00
        ];
        sched.set_weekly_schedule(0, entries.clone()); // Monday

        let val = sched
            .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, None)
            .unwrap();
        if let PropertyValue::List(days) = val {
            assert_eq!(days.len(), 7);
            // Monday (index 0) should have 2 entries
            if let PropertyValue::List(monday_entries) = &days[0] {
                assert_eq!(monday_entries.len(), 2);
                // First entry: [Time(08:00), OctetString([0x01])]
                if let PropertyValue::List(pair) = &monday_entries[0] {
                    assert_eq!(pair[0], PropertyValue::Time(make_time(8, 0)));
                    assert_eq!(pair[1], PropertyValue::OctetString(vec![0x01]));
                } else {
                    panic!("expected pair list");
                }
            } else {
                panic!("expected Monday list");
            }
            // Remaining days should be empty
            for day in days.iter().skip(1) {
                assert_eq!(*day, PropertyValue::List(vec![]));
            }
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn schedule_weekly_schedule_index_0_returns_count() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let val = sched
            .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(7));
    }

    #[test]
    fn schedule_weekly_schedule_index_1_returns_monday() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let entries = vec![make_tv(9, 30, vec![0xAB])];
        sched.set_weekly_schedule(0, entries); // Monday = day_index 0, array_index 1

        let val = sched
            .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(1))
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 1);
            if let PropertyValue::List(pair) = &items[0] {
                assert_eq!(pair[0], PropertyValue::Time(make_time(9, 30)));
                assert_eq!(pair[1], PropertyValue::OctetString(vec![0xAB]));
            } else {
                panic!("expected pair list");
            }
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn schedule_weekly_schedule_index_7_returns_sunday() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let entries = vec![make_tv(10, 0, vec![0xFF])];
        sched.set_weekly_schedule(6, entries); // Sunday = day_index 6, array_index 7

        let val = sched
            .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(7))
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 1);
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn schedule_weekly_schedule_invalid_index_8_returns_error() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let result = sched.read_property(PropertyIdentifier::WEEKLY_SCHEDULE, Some(8));
        assert!(result.is_err());
        if let Err(Error::Protocol { code, .. }) = result {
            assert_eq!(code, ErrorCode::INVALID_ARRAY_INDEX.to_raw() as u32);
        } else {
            panic!("expected Protocol error");
        }
    }

    #[test]
    fn schedule_weekly_schedule_out_of_bounds_day_index_ignored() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        // day_index 7 is out of bounds; should be silently ignored
        sched.set_weekly_schedule(7, vec![make_tv(8, 0, vec![0x01])]);
        // All days should still be empty
        let val = sched
            .read_property(PropertyIdentifier::WEEKLY_SCHEDULE, None)
            .unwrap();
        if let PropertyValue::List(days) = val {
            for day in &days {
                assert_eq!(*day, PropertyValue::List(vec![]));
            }
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    // --- Schedule effective_period ---

    #[test]
    fn schedule_effective_period_default_null() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let val = sched
            .read_property(PropertyIdentifier::EFFECTIVE_PERIOD, None)
            .unwrap();
        assert_eq!(val, PropertyValue::Null);
    }

    #[test]
    fn schedule_effective_period_set_and_read() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let period = BACnetDateRange {
            start_date: Date {
                year: 124,
                month: 1,
                day: 1,
                day_of_week: 1,
            },
            end_date: Date {
                year: 124,
                month: 12,
                day: 31,
                day_of_week: 2,
            },
        };
        sched.set_effective_period(period.clone());
        let val = sched
            .read_property(PropertyIdentifier::EFFECTIVE_PERIOD, None)
            .unwrap();
        assert_eq!(val, PropertyValue::OctetString(period.encode().to_vec()));
    }

    // --- Schedule exception_schedule ---

    #[test]
    fn schedule_exception_schedule_empty_by_default() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let val = sched
            .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn schedule_exception_schedule_count_via_index_zero() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let event = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: BACnetWeekNDay::ANY,
                    week_of_month: BACnetWeekNDay::ANY,
                    day_of_week: 7,
                },
            )),
            list_of_time_values: vec![make_tv(0, 0, vec![0x00])],
            event_priority: 16,
        };
        sched.add_exception(event);
        let val = sched
            .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, Some(0))
            .unwrap();
        assert_eq!(val, PropertyValue::Unsigned(1));
    }

    #[test]
    fn schedule_exception_schedule_add_and_read() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let event = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: BACnetWeekNDay::ANY,
                    week_of_month: BACnetWeekNDay::ANY,
                    day_of_week: 7, // Sunday
                },
            )),
            list_of_time_values: vec![make_tv(0, 0, vec![0x00])],
            event_priority: 16,
        };
        sched.add_exception(event);
        let val = sched
            .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, None)
            .unwrap();
        // Should be a List with one event entry
        if let PropertyValue::List(events) = &val {
            assert_eq!(events.len(), 1);
        } else {
            panic!("expected List, got {val:?}");
        }

        // Add a second exception
        let event2 = BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: BACnetWeekNDay::ANY,
                    week_of_month: BACnetWeekNDay::ANY,
                    day_of_week: 1, // Monday
                },
            )),
            list_of_time_values: vec![],
            event_priority: 14,
        };
        sched.add_exception(event2);
        let val = sched
            .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, None)
            .unwrap();
        if let PropertyValue::List(events) = &val {
            assert_eq!(events.len(), 2);
        } else {
            panic!("expected List, got {val:?}");
        }

        // array_index 0 returns count
        let count = sched
            .read_property(PropertyIdentifier::EXCEPTION_SCHEDULE, Some(0))
            .unwrap();
        assert_eq!(count, PropertyValue::Unsigned(2));
    }

    // --- Schedule list_of_object_property_references ---

    #[test]
    fn schedule_opr_list_empty_by_default() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let val = sched
            .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
            .unwrap();
        assert_eq!(val, PropertyValue::List(vec![]));
    }

    #[test]
    fn schedule_opr_list_add_and_read() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let r = BACnetObjectPropertyReference::new(oid, PropertyIdentifier::PRESENT_VALUE.to_raw());
        sched.add_object_property_reference(r.clone());

        let val = sched
            .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 1);
            if let PropertyValue::List(pair) = &items[0] {
                assert_eq!(pair[0], PropertyValue::ObjectIdentifier(oid));
                assert_eq!(
                    pair[1],
                    PropertyValue::Enumerated(PropertyIdentifier::PRESENT_VALUE.to_raw())
                );
            } else {
                panic!("expected pair list");
            }
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    #[test]
    fn schedule_opr_list_multiple_references() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let oid1 = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        let oid2 = ObjectIdentifier::new(ObjectType::BINARY_OUTPUT, 5).unwrap();
        sched.add_object_property_reference(BACnetObjectPropertyReference::new(
            oid1,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ));
        sched.add_object_property_reference(BACnetObjectPropertyReference::new(
            oid2,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ));

        let val = sched
            .read_property(PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES, None)
            .unwrap();
        if let PropertyValue::List(items) = val {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected PropertyValue::List");
        }
    }

    // --- Schedule property_list ---

    #[test]
    fn schedule_property_list_contains_new_properties() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let props = sched.property_list();
        assert!(props.contains(&PropertyIdentifier::WEEKLY_SCHEDULE));
        assert!(props.contains(&PropertyIdentifier::EXCEPTION_SCHEDULE));
        assert!(props.contains(&PropertyIdentifier::EFFECTIVE_PERIOD));
        assert!(props.contains(&PropertyIdentifier::LIST_OF_OBJECT_PROPERTY_REFERENCES));
    }

    // --- Schedule evaluate() ---

    #[test]
    fn evaluate_returns_default_when_no_entries() {
        let sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let value = sched.evaluate(0, 12, 0); // Monday noon
        assert_eq!(value, PropertyValue::Real(72.0));
    }

    #[test]
    fn evaluate_returns_weekly_value() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        // Monday: 08:00 → occupied, 17:00 → unoccupied
        sched.set_weekly_schedule(
            0,
            vec![make_tv(8, 0, vec![0x01]), make_tv(17, 0, vec![0x00])],
        );

        // Before first entry → default
        assert_eq!(sched.evaluate(0, 7, 59), PropertyValue::Real(72.0));
        // At 08:00 → occupied
        assert_eq!(
            sched.evaluate(0, 8, 0),
            PropertyValue::OctetString(vec![0x01])
        );
        // At 12:00 → still occupied (last entry before current time)
        assert_eq!(
            sched.evaluate(0, 12, 0),
            PropertyValue::OctetString(vec![0x01])
        );
        // At 17:00 → unoccupied
        assert_eq!(
            sched.evaluate(0, 17, 0),
            PropertyValue::OctetString(vec![0x00])
        );
        // At 23:59 → still unoccupied
        assert_eq!(
            sched.evaluate(0, 23, 59),
            PropertyValue::OctetString(vec![0x00])
        );
    }

    #[test]
    fn evaluate_different_day_returns_default() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        // Only Monday has entries
        sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

        // Tuesday should return default
        assert_eq!(sched.evaluate(1, 12, 0), PropertyValue::Real(72.0));
    }

    #[test]
    fn evaluate_exception_overrides_weekly() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        // Monday: 08:00 → 0x01
        sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

        // Exception: all day → 0xFF (higher priority)
        sched.add_exception(BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: BACnetWeekNDay::ANY,
                    week_of_month: BACnetWeekNDay::ANY,
                    day_of_week: BACnetWeekNDay::ANY,
                },
            )),
            list_of_time_values: vec![make_tv(0, 0, vec![0xFF])],
            event_priority: 10,
        });

        // Exception should win over weekly schedule
        assert_eq!(
            sched.evaluate(0, 12, 0),
            PropertyValue::OctetString(vec![0xFF])
        );
    }

    #[test]
    fn evaluate_out_of_service_returns_present_value() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);
        sched.set_present_value(PropertyValue::Real(55.0));
        sched.out_of_service = true;

        assert_eq!(sched.evaluate(0, 12, 0), PropertyValue::Real(55.0));
    }

    #[test]
    fn evaluate_exception_priority_lowest_number_wins() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        // Two exceptions, priority 15 (lower prio) and priority 5 (higher prio)
        sched.add_exception(BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: BACnetWeekNDay::ANY,
                    week_of_month: BACnetWeekNDay::ANY,
                    day_of_week: BACnetWeekNDay::ANY,
                },
            )),
            list_of_time_values: vec![make_tv(0, 0, vec![0xAA])],
            event_priority: 15,
        });
        sched.add_exception(BACnetSpecialEvent {
            period: SpecialEventPeriod::CalendarEntry(BACnetCalendarEntry::WeekNDay(
                BACnetWeekNDay {
                    month: BACnetWeekNDay::ANY,
                    week_of_month: BACnetWeekNDay::ANY,
                    day_of_week: BACnetWeekNDay::ANY,
                },
            )),
            list_of_time_values: vec![make_tv(0, 0, vec![0xBB])],
            event_priority: 5,
        });

        // Priority 5 (lower number = higher priority) should win
        assert_eq!(
            sched.evaluate(0, 12, 0),
            PropertyValue::OctetString(vec![0xBB])
        );
    }

    // --- Schedule tick_schedule ---

    #[test]
    fn tick_schedule_returns_none_when_no_refs() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);
        // No property references → None
        assert!(sched.tick_schedule(0, 12, 0).is_none());
    }

    #[test]
    fn tick_schedule_returns_none_when_value_unchanged() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        sched.add_object_property_reference(BACnetObjectPropertyReference::new(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ));
        // No weekly entries → evaluates to default (Real(72.0)) which matches present_value
        assert!(sched.tick_schedule(0, 12, 0).is_none());
    }

    #[test]
    fn tick_schedule_returns_value_and_refs_on_change() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let target_oid = ObjectIdentifier::new(ObjectType::ANALOG_OUTPUT, 5).unwrap();
        sched.add_object_property_reference(BACnetObjectPropertyReference::new(
            target_oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ));
        sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

        let result = sched.tick_schedule(0, 12, 0);
        assert!(result.is_some());
        let (value, refs) = result.unwrap();
        assert_eq!(value, PropertyValue::OctetString(vec![0x01]));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].0, target_oid);
        assert_eq!(refs[0].1, PropertyIdentifier::PRESENT_VALUE.to_raw());
    }

    #[test]
    fn tick_schedule_updates_present_value() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        sched.add_object_property_reference(BACnetObjectPropertyReference::new(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ));
        sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);

        let _ = sched.tick_schedule(0, 12, 0);
        assert_eq!(
            *sched.present_value(),
            PropertyValue::OctetString(vec![0x01])
        );

        // Second call with same time → no change
        assert!(sched.tick_schedule(0, 12, 0).is_none());
    }

    #[test]
    fn tick_schedule_returns_none_when_out_of_service() {
        let mut sched = ScheduleObject::new(1, "SCHED-1", PropertyValue::Real(72.0)).unwrap();
        let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
        sched.add_object_property_reference(BACnetObjectPropertyReference::new(
            oid,
            PropertyIdentifier::PRESENT_VALUE.to_raw(),
        ));
        sched.set_weekly_schedule(0, vec![make_tv(8, 0, vec![0x01])]);
        sched.out_of_service = true;

        assert!(sched.tick_schedule(0, 12, 0).is_none());
    }
}
