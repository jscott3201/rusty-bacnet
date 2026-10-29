//! Constructed elements as text: the productions the element table names,
//! split with their codecs and written out in the TUI's style.
//!
//! Every element of the table has a renderer, here or in `recipients` and
//! `commands`; a value that doesn't decode as its elements to the last
//! octet is left to the generic view.

use bacnet_encoding::constructed::{
    ConstructedElement, decode_calendar_entry, decode_daily_schedule, decode_date_range,
    decode_device_object_property_reference, decode_device_object_reference, decode_name_value,
    decode_object_property_reference_at, decode_special_event, decode_value_source, split_elements,
};
use bacnet_encoding::primitives::decode_timestamp_choice;
use bacnet_types::constructed::{
    BACnetCalendarEntry, BACnetDeviceObjectReference, BACnetTimeValue, BACnetValueSource,
    SpecialEventPeriod,
};
use bacnet_types::enums::PropertyIdentifier;
use bacnet_types::error::Error;
use bacnet_types::primitives::{BACnetTimeStamp, ObjectIdentifier};

use super::primitive::{date_range, date_text, date_time, month_text, time_text, weekday};
use super::{ValueFormat, commands, event_parameter, recipients};
use crate::label::{kebab, object_label};

/// The object instance an unset reference names: the standard keeps
/// 4194303 for an object that isn't there.
const UNSET_INSTANCE: u32 = 4_194_303;

/// The text of each element of `data`, or `None` when the element has no
/// renderer, `data` is empty, or an element doesn't decode. `first` is the
/// place in the array (from 0) of the first element: 0 for a whole read,
/// the index less one for an indexed read.
pub(super) fn elements(
    format: &ValueFormat<'_>,
    element: ConstructedElement,
    data: &[u8],
    first: usize,
) -> Option<Vec<String>> {
    if data.is_empty() {
        return None;
    }
    let render = renderer(element)?;
    let mut texts = Vec::new();
    let ends = split_elements(data, |octets, offset| {
        render(format, first.saturating_add(texts.len()), octets, offset).map(|(text, end)| {
            texts.push(text);
            end
        })
    })?;
    debug_assert_eq!(ends.len(), texts.len());
    Some(texts)
}

/// Decodes the element at an offset, the `index`th (from 0) of the value,
/// and writes it out.
type Render = fn(&ValueFormat<'_>, usize, &[u8], usize) -> Result<(String, usize), Error>;

/// The renderer of an element; `None` only for an element added to the
/// table after this match (the enum is non-exhaustive), which a test
/// catches.
pub(super) fn renderer(element: ConstructedElement) -> Option<Render> {
    use ConstructedElement as E;
    Some(match element {
        E::TimeStamp => timestamp,
        E::DailySchedule => daily_schedule,
        E::SpecialEvent => special_event,
        E::CalendarEntry => calendar_entry,
        E::DateRange => |_, _, octets, offset| {
            decode_date_range(octets, offset)
                .map(|(range, end)| (date_range(&range.start_date, &range.end_date), end))
        },
        E::DeviceObjectPropertyReference => |_, _, octets, offset| {
            decode_device_object_property_reference(octets, offset).map(|(r, end)| {
                let text = reference(
                    r.object_identifier,
                    r.property_identifier,
                    r.property_array_index,
                );
                (on_device(text, r.device_identifier), end)
            })
        },
        E::ObjectPropertyReference => |_, _, octets, offset| {
            decode_object_property_reference_at(octets, offset).map(|(r, end)| {
                let text = reference(
                    r.object_identifier,
                    r.property_identifier,
                    r.property_array_index,
                );
                (text, end)
            })
        },
        E::DeviceObjectReference => |_, _, octets, offset| {
            decode_device_object_reference(octets, offset)
                .map(|(r, end)| (object_reference(&r), end))
        },
        E::ValueSource => |_, _, octets, offset| {
            decode_value_source(octets, offset).map(|(source, end)| {
                let text = match source {
                    BACnetValueSource::None => "none".to_string(),
                    BACnetValueSource::Object(r) => object_reference(&r),
                    BACnetValueSource::Address(address) => recipients::address_text(&address),
                };
                (text, end)
            })
        },
        E::NameValue => |format, _, octets, offset| {
            decode_name_value(octets, offset).map(|(tag, end)| {
                let text = match &tag.value {
                    Some(value) => format!(
                        "{} = {}",
                        tag.name,
                        format.element(PropertyIdentifier::TAGS, value)
                    ),
                    None => tag.name.clone(),
                };
                (text, end)
            })
        },
        E::Destination => recipients::destination,
        E::Recipient => recipients::recipient,
        E::CovSubscription => recipients::cov_subscription,
        E::AddressBinding => recipients::address_binding,
        E::EventParameter => event_parameter::event_parameter,
        E::ActionList => commands::action_list,
        E::ReadAccessSpecification => commands::read_access_specification,
        E::ReadAccessResult => commands::read_access_result,
        E::PropertyAccessResult => commands::property_access_result,
        E::StageLimitValue => commands::stage_limit_value,
        E::Scale => commands::scale,
        E::Prescale => commands::prescale,
        E::ShedLevel => commands::shed_level,
        E::PortPermission => commands::port_permission,
        E::AuthenticationFactor => commands::authentication_factor,
        E::AuthenticationFactorFormat => commands::authentication_factor_format,
        E::AuthenticationPolicy => commands::authentication_policy,
        E::AccessRule => commands::access_rule,
        _ => return None,
    })
}

/// An element's kind, as the CLI's JSON names it (`docs/CLI.md`, "Value
/// output"). Written out, so renaming an encoding variant can't change the
/// JSON; `element` only for one added to the table after this match, which
/// a test catches.
pub(super) fn element_name(element: ConstructedElement) -> &'static str {
    use ConstructedElement as E;
    match element {
        E::NameValue => "name-value",
        E::Destination => "destination",
        E::PortPermission => "port-permission",
        E::ReadAccessSpecification => "read-access-specification",
        E::ReadAccessResult => "read-access-result",
        E::ActionList => "action-list",
        E::DeviceObjectReference => "device-object-reference",
        E::AuthenticationFactorFormat => "authentication-factor-format",
        E::AuthenticationPolicy => "authentication-policy",
        E::StageLimitValue => "stage-limit-value",
        E::AccessRule => "access-rule",
        E::DeviceObjectPropertyReference => "device-object-property-reference",
        E::ObjectPropertyReference => "object-property-reference",
        E::PropertyAccessResult => "property-access-result",
        E::Recipient => "recipient",
        E::DailySchedule => "daily-schedule",
        E::SpecialEvent => "special-event",
        E::CalendarEntry => "calendar-entry",
        E::DateRange => "date-range",
        E::TimeStamp => "time-stamp",
        E::CovSubscription => "cov-subscription",
        E::ValueSource => "value-source",
        E::Scale => "scale",
        E::Prescale => "prescale",
        E::AddressBinding => "address-binding",
        E::ShedLevel => "shed-level",
        E::AuthenticationFactor => "authentication-factor",
        E::EventParameter => "event-parameter",
        _ => "element",
    }
}

/// What parts the elements of a collection: `; ` for the elements whose
/// own text holds commas, `, ` for the rest.
pub(super) fn separator(element: ConstructedElement) -> &'static str {
    use ConstructedElement as E;
    match element {
        E::DailySchedule
        | E::Destination
        | E::CovSubscription
        | E::ActionList
        | E::ReadAccessSpecification
        | E::ReadAccessResult
        | E::PropertyAccessResult
        | E::StageLimitValue
        | E::AuthenticationPolicy
        | E::AccessRule => "; ",
        _ => ", ",
    }
}

fn timestamp(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_timestamp_choice(octets, offset).map(|(stamp, end)| (timestamp_text(&stamp), end))
}

/// A timestamp as text: a time, `seq 12`, or a date and time.
pub(crate) fn timestamp_text(stamp: &BACnetTimeStamp) -> String {
    match stamp {
        BACnetTimeStamp::Time(time) => time_text(time),
        BACnetTimeStamp::SequenceNumber(n) => format!("seq {n}"),
        BACnetTimeStamp::DateTime { date, time } => date_time(date, time),
    }
}

/// Time and value pairs: `07:00 = active, 18:00 = inactive`.
fn time_values(format: &ValueFormat<'_>, values: &[BACnetTimeValue]) -> String {
    if values.is_empty() {
        return "none".into();
    }
    values
        .iter()
        .map(|tv| {
            format!(
                "{} = {}",
                time_text(&tv.time),
                format.element(PropertyIdentifier::PRESENT_VALUE, &tv.value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// One day of a Weekly_Schedule, named for its place in the week, which
/// starts on Monday (Clause 12.24).
fn daily_schedule(
    format: &ValueFormat<'_>,
    index: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_daily_schedule(octets, offset).map(|(values, end)| {
        let label = u8::try_from(index + 1)
            .ok()
            .and_then(weekday)
            .unwrap_or("day");
        (format!("{label} {}", time_values(format, &values)), end)
    })
}

fn calendar_text(entry: &BACnetCalendarEntry) -> String {
    match entry {
        BACnetCalendarEntry::Date(date) => date_text(date),
        BACnetCalendarEntry::DateRange(range) => date_range(&range.start_date, &range.end_date),
        BACnetCalendarEntry::WeekNDay(week) => {
            let month = match week.month {
                0xFF => "any month".to_string(),
                month => format!("month {}", month_text(month)),
            };
            format!(
                "{}, {}, {month}",
                weekday(week.day_of_week).unwrap_or("any day"),
                week_text(week.week_of_month)
            )
        }
    }
}

/// A week-of-month field in words: weeks 1 to 5 by their days, and 6 to 9
/// counted back from the month's end in blocks of seven days.
fn week_text(week: u8) -> String {
    match week {
        0xFF => "any week".into(),
        1..=5 => {
            let first = (week - 1) * 7 + 1;
            let last = if week == 5 { 31 } else { first + 6 };
            format!("days {first}-{last}")
        }
        6 => "the final 7 days".into(),
        7..=9 => format!("the 7 days before the final {}", (week - 6) * 7),
        week => format!("week {week}"),
    }
}

fn calendar_entry(
    _: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_calendar_entry(octets, offset).map(|(entry, end)| (calendar_text(&entry), end))
}

fn special_event(
    format: &ValueFormat<'_>,
    _: usize,
    octets: &[u8],
    offset: usize,
) -> Result<(String, usize), Error> {
    decode_special_event(octets, offset).map(|(event, end)| {
        let period = match &event.period {
            SpecialEventPeriod::CalendarEntry(entry) => calendar_text(entry),
            SpecialEventPeriod::CalendarReference(calendar) => object_label(*calendar),
        };
        let text = format!(
            "{period}: {} (priority {})",
            time_values(format, &event.list_of_time_values),
            event.event_priority
        );
        (text, end)
    })
}

/// `analog-input 2 present-value`, with `[3]` for an array index; `none`
/// for the unset reference.
pub(super) fn reference(object: ObjectIdentifier, property: u32, index: Option<u32>) -> String {
    if object.instance_number() == UNSET_INSTANCE {
        return "none".into();
    }
    let mut text = format!(
        "{} {}",
        object_label(object),
        kebab(PropertyIdentifier::from_raw(property))
    );
    if let Some(index) = index {
        text.push_str(&format!("[{index}]"));
    }
    text
}

pub(super) fn on_device(text: String, device: Option<ObjectIdentifier>) -> String {
    match device {
        Some(device) if text != "none" => format!("{text} on device {}", device.instance_number()),
        _ => text,
    }
}

pub(super) fn object_reference(reference: &BACnetDeviceObjectReference) -> String {
    let object = if reference.object_identifier.instance_number() == UNSET_INSTANCE {
        "none".to_string()
    } else {
        object_label(reference.object_identifier)
    };
    on_device(object, reference.device_identifier)
}
