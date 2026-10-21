//! Calendar (type 6), Clause 12.9.

use std::borrow::Cow;
use std::sync::Arc;

use bacnet_types::calendar::SpecificDate;
use bacnet_types::constructed::BACnetCalendarEntry;
use bacnet_types::enums::{ErrorClass, ErrorCode, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{calendar_metadata, date_list};
use crate::clock::ClockReader;
use crate::common::{self, read_property_list_property};
use crate::traits::BACnetObject;

/// BACnet Calendar object.
///
/// Present_Value is TRUE when the device's local date matches an entry in
/// Date_List (Clause 12.9). The object reads that date from the database's
/// clock each time Present_Value is read, so the value follows the date with
/// no tick; without a clock, or while the clock has no valid local date, it
/// is FALSE. [`is_active_on`](Self::is_active_on) answers for any other day.
///
/// Date_List is a BACnetLIST of `BACnetCalendarEntry`, each entry under its
/// Clause 21 CHOICE tag on the wire. It is network-writable (WriteProperty,
/// AddListElement and RemoveListElement). A write whose entries hold a value
/// outside its Clause 21 range is refused with VALUE_OUT_OF_RANGE and leaves
/// the list unchanged.
///
/// The object serves only properties its table (Clause 12.9, Table 12-11)
/// defines. That table has no Status_Flags, Event_State, Out_Of_Service or
/// Reliability, so reads and writes of those return UNKNOWN_PROPERTY.
pub struct CalendarObject {
    oid: ObjectIdentifier,
    name: String,
    description: String,
    date_list: Vec<BACnetCalendarEntry>,
    /// The database's wall clock, the source of the local date.
    clock: Option<Arc<dyn ClockReader>>,
}

impl CalendarObject {
    /// Create a new Calendar object with an empty date list.
    pub fn new(instance: u32, name: impl Into<String>) -> Result<Self, Error> {
        let oid = ObjectIdentifier::new(ObjectType::CALENDAR, instance)?;
        Ok(Self {
            oid,
            name: name.into(),
            description: String::new(),
            date_list: Vec::new(),
            clock: None,
        })
    }

    /// Set the description string.
    pub fn set_description(&mut self, desc: impl Into<String>) {
        self.description = desc.into();
    }

    /// Append a calendar entry to the date_list.
    ///
    /// Refuses, as a network write would, an entry with a value outside its
    /// Clause 21 range (PROPERTY / VALUE_OUT_OF_RANGE) and an entry past the
    /// 1,024-entry cap (RESOURCES / NO_SPACE_TO_WRITE_PROPERTY).
    pub fn add_date_entry(&mut self, entry: BACnetCalendarEntry) -> Result<(), Error> {
        if !entry.is_valid() {
            return Err(common::value_out_of_range_error());
        }
        if self.date_list.len() >= date_list::MAX_DATE_LIST_ENTRIES {
            return Err(date_list::no_space_error());
        }
        self.date_list.push(entry);
        Ok(())
    }

    /// Remove all entries from the date_list.
    pub fn clear_date_list(&mut self) {
        self.date_list.clear();
    }

    /// The current date_list entries, as configured or last written.
    pub fn date_list(&self) -> &[BACnetCalendarEntry] {
        &self.date_list
    }

    /// Whether `day` matches a Date_List entry: the Present_Value the
    /// calendar has on that day.
    pub fn is_active_on(&self, day: SpecificDate) -> bool {
        self.date_list.iter().any(|entry| entry.matches(day))
    }

    /// Present_Value now: whether the clock's local date matches an entry.
    /// FALSE without a clock or a valid local date.
    pub fn present_value(&self) -> bool {
        self.clock
            .as_ref()
            .and_then(|clock| clock.read_clock())
            .and_then(|frame| SpecificDate::from_date(&frame.local_date))
            .is_some_and(|today| self.is_active_on(today))
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
                Ok(PropertyValue::Boolean(self.present_value()))
            }
            p if p == PropertyIdentifier::DATE_LIST => Ok(date_list::read(&self.date_list)),
            p if p == PropertyIdentifier::PROPERTY_LIST => {
                read_property_list_property(&self.property_list(), array_index)
            }
            _ => Err(Error::Protocol {
                class: ErrorClass::PROPERTY,
                code: ErrorCode::UNKNOWN_PROPERTY,
            }),
        }
    }

    fn write_property(
        &mut self,
        property: PropertyIdentifier,
        array_index: Option<u32>,
        value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        if let Some(result) = common::write_description(&mut self.description, property, &value) {
            return result;
        }
        if property == PropertyIdentifier::DATE_LIST {
            // A BACnetLIST takes no index; the services gate this first.
            if array_index.is_some() {
                return Err(common::property_is_not_an_array_error());
            }
            self.date_list = date_list::decode_write(value)?;
            return Ok(());
        }
        if property == PropertyIdentifier::PRESENT_VALUE {
            return Err(common::write_access_denied_error());
        }
        Err(common::unhandled_write_error(
            self.property_metadata().as_ref(),
            property,
            array_index,
        ))
    }

    fn property_metadata(&self) -> Cow<'_, [crate::property_metadata::PropertyMetadata]> {
        calendar_metadata::for_object(self)
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        crate::property_metadata::property_list_from_metadata(self.property_metadata().as_ref())
    }

    fn bind_clock_internal(&mut self, clock: Option<Arc<dyn ClockReader>>) {
        self.clock = clock;
    }

    fn calendar_state_internal(&self, day: SpecificDate) -> Option<bool> {
        Some(self.is_active_on(day))
    }
}
