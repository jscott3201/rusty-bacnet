//! Service parameters named field by field (#2074), for the services a
//! capture shows most: ReadProperty, ReadPropertyMultiple and their ACKs,
//! WriteProperty, the COV and event notifications, GetEventInformation and
//! its ACK, and the two password-bearing requests, DeviceCommunicationControl
//! and ReinitializeDevice.
//!
//! The service's own decoder in `bacnet-services` (or `bacnet-encoding`)
//! reads the values; a light scan of the tag headers gives each field its
//! octets. A property value is written by the shared formatter with the
//! object type and property as context, so enumerations, bits and
//! constructed values come out named, and its tags stay under it.
//!
//! Anything else, a segment of a longer message, or parameters the
//! service's decoder refuses, falls back to the tag walk, with a leaf
//! saying why the decoder refused them. A password is never shown, on
//! either path.

use std::ops::Range;

use bacnet_encoding::constructed::decode_event_notification_tolerant;
use bacnet_encoding::tags::{MAX_CONTEXT_NESTING_DEPTH, TagClass, decode_tag};
use bacnet_services::alarm_event::{GetEventInformationAck, GetEventInformationRequest};
use bacnet_services::cov::COVNotificationRequest;
use bacnet_services::device_mgmt::{DeviceCommunicationControlRequest, ReinitializeDeviceRequest};
use bacnet_services::read_property::{ReadPropertyACK, ReadPropertyRequest};
use bacnet_services::rpm::{ReadPropertyMultipleACK, ReadPropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_types::constructed::BACnetPropertyValue;
use bacnet_types::enums::{
    ConfirmedServiceChoice as Confirmed, ObjectType, PropertyIdentifier,
    UnconfirmedServiceChoice as Unconfirmed,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue};

use super::{Field, tags};

#[path = "service_password.rs"]
mod password;

use crate::format::alarms::{EventSummaryData, cell};
use crate::format::notification::EventValues;
use crate::format::{Floats, StateTexts, ValueFormat, format_property_value, timestamp_text};
use crate::label::{kebab, object_label};
use password::{PASSWORD, hide_passwords};

/// Which parameters an APDU carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Params {
    /// A confirmed request's.
    Request(Confirmed),
    /// An unconfirmed request's.
    Unconfirmed(Unconfirmed),
    /// A Complex-ACK's.
    Ack(Confirmed),
}

/// Why the named decode gave way to the tag walk.
enum Miss {
    /// The service has no named decode here.
    Unnamed,
    /// Its decoder refused the parameters, or they aren't laid out as it
    /// read them.
    Refused(String),
}

impl From<bacnet_types::error::Error> for Miss {
    fn from(error: bacnet_types::error::Error) -> Self {
        Self::Refused(error.to_string())
    }
}

type Named = Result<Vec<Field>, Miss>;

/// The parameters `apdu[start..]` of `params`, named field by field when
/// the service has a named decode and its decoder reads them, otherwise
/// tag by tag. A segment (`segment` is its sequence number) always takes
/// the tag walk; a later segment of a password-bearing request, whose tag
/// boundaries can't be trusted, is one opaque leaf. Ranges are offsets in
/// `apdu` plus `base`.
pub(super) fn parameters(
    params: Option<Params>,
    segment: Option<u8>,
    apdu: &[u8],
    base: usize,
    start: usize,
) -> Vec<Field> {
    let doc = Doc {
        data: &apdu[start..],
        at: base + start,
    };
    let secret = params.and_then(Params::secret);
    if secret.is_some() && segment.is_some_and(|sequence| sequence != 0) {
        return vec![Field::leaf(
            format!(
                "Segment data: {} octets, hidden (may hold a password)",
                doc.data.len()
            ),
            doc.whole(),
        )];
    }
    let whole = params.filter(|_| segment.is_none());
    let refused = match whole.map(|params| named(params, &doc)) {
        Some(Ok(fields)) => return fields,
        Some(Err(Miss::Refused(why))) => Some(why),
        Some(Err(Miss::Unnamed)) | None => None,
    };
    let mut fields = tags(apdu, base, start, apdu.len(), 0);
    if let Some(secret) = secret {
        hide_passwords(&mut fields, &doc, secret);
    }
    if let Some(why) = refused {
        fields.push(Field::leaf(
            format!("Not read as named fields: {why}"),
            doc.whole(),
        ));
    }
    fields
}

fn named(params: Params, doc: &Doc<'_>) -> Named {
    use Params::{Ack, Request, Unconfirmed as Un};
    let data = doc.data;
    match params {
        Request(Confirmed::READ_PROPERTY) => read_property(doc, ReadPropertyRequest::decode(data)?),
        Ack(Confirmed::READ_PROPERTY) => read_property_ack(doc, ReadPropertyACK::decode(data)?),
        Request(Confirmed::READ_PROPERTY_MULTIPLE) => {
            rpm_request(doc, ReadPropertyMultipleRequest::decode(data)?)
        }
        Ack(Confirmed::READ_PROPERTY_MULTIPLE) => {
            rpm_ack(doc, ReadPropertyMultipleACK::decode(data)?)
        }
        Request(Confirmed::WRITE_PROPERTY) => {
            write_property(doc, WritePropertyRequest::decode(data)?)
        }
        Request(Confirmed::CONFIRMED_COV_NOTIFICATION)
        | Un(Unconfirmed::UNCONFIRMED_COV_NOTIFICATION) => {
            cov_notification(doc, COVNotificationRequest::decode(data)?)
        }
        Request(Confirmed::CONFIRMED_EVENT_NOTIFICATION)
        | Un(Unconfirmed::UNCONFIRMED_EVENT_NOTIFICATION) => event_notification(doc),
        Request(Confirmed::GET_EVENT_INFORMATION) => {
            let request = GetEventInformationRequest::decode(data)?;
            let items = doc.items()?;
            Ok(opt(doc, &items, 0, |_| {
                request
                    .last_received_object_identifier
                    .map(|object| format!("Last received object: {}", object_label(object)))
            }))
        }
        Ack(Confirmed::GET_EVENT_INFORMATION) => {
            event_information(doc, &GetEventInformationAck::decode(data)?)
        }
        Request(Confirmed::DEVICE_COMMUNICATION_CONTROL) => {
            let request = DeviceCommunicationControlRequest::decode(data)?;
            let items = doc.items()?;
            let mut fields = opt(doc, &items, 0, |_| {
                request
                    .time_duration
                    .map(|minutes| format!("Time duration: {minutes} min"))
            });
            fields.push(doc.leaf(
                need(&items, 1)?,
                format!("Enable/disable: {}", kebab(request.enable_disable)),
            ));
            fields.extend(opt(doc, &items, 2, |_| Some(PASSWORD.into())));
            Ok(fields)
        }
        Request(Confirmed::REINITIALIZE_DEVICE) => {
            let request = ReinitializeDeviceRequest::decode(data)?;
            let items = doc.items()?;
            let mut fields = vec![doc.leaf(
                need(&items, 0)?,
                format!(
                    "Reinitialized state: {}",
                    kebab(request.reinitialized_state)
                ),
            )];
            fields.extend(opt(doc, &items, 1, |_| Some(PASSWORD.into())));
            Ok(fields)
        }
        _ => Err(Miss::Unnamed),
    }
}

/// The parameters being read, with where they sit in the APDU.
struct Doc<'a> {
    /// The parameters.
    data: &'a [u8],
    /// Where `data[0]` sits, as a field's range counts.
    at: usize,
}

/// One tagged item at a level of the parameters: a primitive, or an
/// opening tag through its closing tag. Offsets are in [`Doc::data`].
#[derive(Debug, Clone)]
struct Item {
    /// Its tag number.
    number: u8,
    /// Its octets, tags included.
    range: Range<usize>,
    /// Inside the opening and closing tags (or the primitive's content).
    content: Range<usize>,
}

impl Doc<'_> {
    fn whole(&self) -> Range<usize> {
        self.at..self.at + self.data.len()
    }

    fn abs(&self, range: &Range<usize>) -> Range<usize> {
        self.at + range.start..self.at + range.end
    }

    fn leaf(&self, item: &Item, label: impl Into<String>) -> Field {
        Field::leaf(label, self.abs(&item.range))
    }

    /// The top-level items.
    fn items(&self) -> Result<Vec<Item>, Miss> {
        self.items_in(&(0..self.data.len()))
    }

    /// The items in `span`, one level deep.
    fn items_in(&self, span: &Range<usize>) -> Result<Vec<Item>, Miss> {
        let data = &self.data[..span.end];
        let mut items = Vec::new();
        let mut pos = span.start;
        while pos < span.end {
            let (tag, content) = decode_tag(data, pos)?;
            if tag.is_closing {
                return Err(layout());
            }
            let (inner, after) = if tag.is_opening {
                close(data, content)?
            } else {
                let end = primitive_end(data, tag.class, tag.number, tag.length, content)?;
                (end, end)
            };
            items.push(Item {
                number: tag.number,
                range: pos..after,
                content: content..inner,
            });
            pos = after;
        }
        Ok(items)
    }

    /// The tags inside `item`, as the tag walk shows them.
    fn tags_of(&self, item: &Item) -> Vec<Field> {
        tags(self.data, self.at, item.content.start, item.content.end, 1)
    }

    /// A property value, written with its object type and property, its
    /// tags under it unless it is one primitive; and its text alone.
    fn value(
        &self,
        item: &Item,
        object_type: ObjectType,
        property: PropertyIdentifier,
        index: Option<u32>,
        octets: &[u8],
    ) -> (String, Field) {
        let texts = StateTexts::default();
        let format = ValueFormat::new(object_type, &texts, Floats::Display);
        let text = format.value_at(property, index, octets);
        let mut inner = self.tags_of(item);
        if let [one] = inner.as_slice()
            && one.children.is_empty()
        {
            inner.clear();
        }
        let mut field = Field::node(format!("Value: {text}"), self.abs(&item.range), inner);
        field.device_text = format
            .data_at(property, index, octets)
            .may_hold_device_text();
        (text, field)
    }
}

/// The parameters aren't laid out as their decoder read them.
fn layout() -> Miss {
    Miss::Refused("unexpected layout".into())
}

/// Where the primitive whose content starts at `content` ends. An
/// application Boolean holds its value in the header.
fn primitive_end(
    data: &[u8],
    class: TagClass,
    number: u8,
    length: u32,
    content: usize,
) -> Result<usize, Miss> {
    if class == TagClass::Application && number == 1 {
        return Ok(content);
    }
    content
        .checked_add(usize::try_from(length).map_err(|_| layout())?)
        .filter(|end| *end <= data.len())
        .ok_or_else(layout)
}

/// From just inside an opening tag: where its closing tag starts, and the
/// octet after it.
fn close(data: &[u8], mut pos: usize) -> Result<(usize, usize), Miss> {
    let mut depth = 0usize;
    while pos < data.len() {
        let (tag, content) = decode_tag(data, pos)?;
        if tag.is_closing {
            if depth == 0 {
                return Ok((pos, content));
            }
            depth -= 1;
            pos = content;
        } else if tag.is_opening {
            depth += 1;
            if depth > MAX_CONTEXT_NESTING_DEPTH {
                return Err(layout());
            }
            pos = content;
        } else {
            pos = primitive_end(data, tag.class, tag.number, tag.length, content)?;
        }
    }
    Err(layout())
}

/// The item tagged `number`.
fn find(items: &[Item], number: u8) -> Option<&Item> {
    items.iter().find(|item| item.number == number)
}

/// The item tagged `number`, which the decoder says is there.
fn need(items: &[Item], number: u8) -> Result<&Item, Miss> {
    find(items, number).ok_or_else(layout)
}

/// A leaf for the item tagged `number`, when it is there and `label`
/// names it.
fn opt(
    doc: &Doc<'_>,
    items: &[Item],
    number: u8,
    label: impl FnOnce(&Item) -> Option<String>,
) -> Vec<Field> {
    find(items, number)
        .and_then(|item| label(item).map(|label| doc.leaf(item, label)))
        .into_iter()
        .collect()
}

/// Items split into runs, each starting at an item tagged `first`; as
/// many runs as `count`, or a layout miss.
fn runs(items: &[Item], first: u8, count: usize) -> Result<Vec<&[Item]>, Miss> {
    if items.first().is_some_and(|item| item.number != first) {
        return Err(layout());
    }
    let runs: Vec<&[Item]> = items.chunk_by(|_, next| next.number != first).collect();
    if runs.len() == count {
        Ok(runs)
    } else {
        Err(layout())
    }
}

/// The span of a run of items.
fn span(doc: &Doc<'_>, run: &[Item]) -> Range<usize> {
    let start = run.first().map_or(0, |item| item.range.start);
    let end = run.last().map_or(start, |item| item.range.end);
    doc.abs(&(start..end))
}

/// A property, with its array index when it has one:
/// `priority-array[3]`.
fn property_text(property: PropertyIdentifier, index: Option<u32>) -> String {
    match index {
        Some(index) => format!("{}[{index}]", kebab(property)),
        None => kebab(property),
    }
}

/// The object, property and index leaves every property service starts
/// with, tagged 0, 1 and 2.
fn object_property(
    doc: &Doc<'_>,
    items: &[Item],
    object: ObjectIdentifier,
    property: PropertyIdentifier,
    index: Option<u32>,
) -> Result<Vec<Field>, Miss> {
    let mut fields = vec![
        doc.leaf(need(items, 0)?, format!("Object: {}", object_label(object))),
        doc.leaf(need(items, 1)?, format!("Property: {}", kebab(property))),
    ];
    fields.extend(opt(doc, items, 2, |_| {
        index.map(|index| format!("Array index: {index}"))
    }));
    Ok(fields)
}

fn read_property(doc: &Doc<'_>, request: ReadPropertyRequest) -> Named {
    let items = doc.items()?;
    object_property(
        doc,
        &items,
        request.object_identifier,
        request.property_identifier,
        request.property_array_index,
    )
}

fn read_property_ack(doc: &Doc<'_>, ack: ReadPropertyACK) -> Named {
    let items = doc.items()?;
    let mut fields = object_property(
        doc,
        &items,
        ack.object_identifier,
        ack.property_identifier,
        ack.property_array_index,
    )?;
    fields.push(
        doc.value(
            need(&items, 3)?,
            ack.object_identifier.object_type(),
            ack.property_identifier,
            ack.property_array_index,
            &ack.property_value,
        )
        .1,
    );
    Ok(fields)
}

fn write_property(doc: &Doc<'_>, request: WritePropertyRequest) -> Named {
    let items = doc.items()?;
    let mut fields = object_property(
        doc,
        &items,
        request.object_identifier,
        request.property_identifier,
        request.property_array_index,
    )?;
    fields.push(
        doc.value(
            need(&items, 3)?,
            request.object_identifier.object_type(),
            request.property_identifier,
            request.property_array_index,
            &request.property_value,
        )
        .1,
    );
    fields.extend(opt(doc, &items, 4, |_| {
        request
            .priority
            .map(|priority| format!("Priority: {priority}"))
    }));
    Ok(fields)
}

fn rpm_request(doc: &Doc<'_>, request: ReadPropertyMultipleRequest) -> Named {
    let items = doc.items()?;
    let specs = &request.list_of_read_access_specs;
    let mut fields = Vec::new();
    for (run, spec) in runs(&items, 0, specs.len())?.into_iter().zip(specs) {
        let object = object_label(spec.object_identifier);
        let list = need(run, 1)?;
        let refs = doc.items_in(&list.content)?;
        let wanted = &spec.list_of_property_references;
        let properties = runs(&refs, 0, wanted.len())?
            .into_iter()
            .zip(wanted)
            .map(|(run, reference)| {
                Field::leaf(
                    format!(
                        "Property: {}",
                        property_text(
                            reference.property_identifier,
                            reference.property_array_index
                        )
                    ),
                    span(doc, run),
                )
            })
            .collect();
        fields.push(Field::node(
            format!("Read {object}"),
            span(doc, run),
            vec![
                doc.leaf(need(run, 0)?, format!("Object: {object}")),
                Field::node("Properties", doc.abs(&list.range), properties),
            ],
        ));
    }
    Ok(fields)
}

fn rpm_ack(doc: &Doc<'_>, ack: ReadPropertyMultipleACK) -> Named {
    let items = doc.items()?;
    let results = &ack.list_of_read_access_results;
    let mut fields = Vec::new();
    for (run, result) in runs(&items, 0, results.len())?.into_iter().zip(results) {
        let object = result.object_identifier;
        let list = need(run, 1)?;
        let inner = doc.items_in(&list.content)?;
        let elements = &result.list_of_results;
        let mut rows = Vec::new();
        for (run, element) in runs(&inner, 2, elements.len())?.into_iter().zip(elements) {
            let property = element.property_identifier;
            let index = element.property_array_index;
            let name = property_text(property, index);
            let mut kids = vec![doc.leaf(need(run, 2)?, format!("Property: {}", kebab(property)))];
            kids.extend(opt(doc, run, 3, |_| {
                index.map(|index| format!("Array index: {index}"))
            }));
            let (label, device_text) = match (&element.property_value, element.error) {
                (Some(value), _) => {
                    let (text, field) =
                        doc.value(need(run, 4)?, object.object_type(), property, index, value);
                    let label = format!("{name}: {text}");
                    let device_text = field.device_text;
                    kids.push(field);
                    (label, device_text)
                }
                (None, Some((class, code))) => {
                    let error = format!("{}:{}", kebab(class), kebab(code));
                    kids.push(doc.leaf(need(run, 5)?, format!("Error: {error}")));
                    (format!("{name}: error {error}"), false)
                }
                (None, None) => return Err(layout()),
            };
            let mut row = Field::node(label, span(doc, run), kids);
            row.device_text = device_text;
            rows.push(row);
        }
        let object = object_label(object);
        fields.push(Field::node(
            format!("Results for {object}"),
            span(doc, run),
            vec![
                doc.leaf(need(run, 0)?, format!("Object: {object}")),
                Field::node("Properties", doc.abs(&list.range), rows),
            ],
        ));
    }
    Ok(fields)
}

/// The reported property values of a notification, one row each, their
/// values written as properties of `object_type`.
fn property_values(
    doc: &Doc<'_>,
    list: &Item,
    object_type: ObjectType,
    values: &[BACnetPropertyValue],
) -> Result<Field, Miss> {
    let inner = doc.items_in(&list.content)?;
    let mut rows = Vec::new();
    for (run, value) in runs(&inner, 0, values.len())?.into_iter().zip(values) {
        let property = value.property_identifier;
        let index = value.property_array_index;
        let mut kids = vec![doc.leaf(need(run, 0)?, format!("Property: {}", kebab(property)))];
        kids.extend(opt(doc, run, 1, |_| {
            index.map(|index| format!("Array index: {index}"))
        }));
        let (text, field) = doc.value(need(run, 2)?, object_type, property, index, &value.value);
        let label = format!("{}: {text}", property_text(property, index));
        let device_text = field.device_text;
        kids.push(field);
        kids.extend(opt(doc, run, 3, |_| {
            value
                .priority
                .map(|priority| format!("Priority: {priority}"))
        }));
        let mut row = Field::node(label, span(doc, run), kids);
        row.device_text = device_text;
        rows.push(row);
    }
    Ok(Field::node("Values", doc.abs(&list.range), rows))
}

fn cov_notification(doc: &Doc<'_>, request: COVNotificationRequest) -> Named {
    let items = doc.items()?;
    let monitored = request.monitored_object_identifier;
    Ok(vec![
        doc.leaf(
            need(&items, 0)?,
            format!(
                "Subscriber process ID: {}",
                request.subscriber_process_identifier
            ),
        ),
        doc.leaf(
            need(&items, 1)?,
            format!(
                "Initiating device: {}",
                object_label(request.initiating_device_identifier)
            ),
        ),
        doc.leaf(
            need(&items, 2)?,
            format!("Monitored object: {}", object_label(monitored)),
        ),
        doc.leaf(
            need(&items, 3)?,
            match request.time_remaining {
                0 => "Time remaining: no expiry".to_string(),
                seconds => format!("Time remaining: {seconds} s"),
            },
        ),
        property_values(
            doc,
            need(&items, 4)?,
            monitored.object_type(),
            &request.list_of_values,
        )?,
    ])
}

fn event_notification(doc: &Doc<'_>) -> Named {
    let request = decode_event_notification_tolerant(doc.data)?;
    let items = doc.items()?;
    let mut fields = vec![
        doc.leaf(
            need(&items, 0)?,
            format!("Process ID: {}", request.process_identifier),
        ),
        doc.leaf(
            need(&items, 1)?,
            format!(
                "Initiating device: {}",
                object_label(request.initiating_device_identifier)
            ),
        ),
        doc.leaf(
            need(&items, 2)?,
            format!(
                "Event object: {}",
                object_label(request.event_object_identifier)
            ),
        ),
        doc.leaf(
            need(&items, 3)?,
            format!("Time stamp: {}", timestamp_text(&request.timestamp)),
        ),
        doc.leaf(
            need(&items, 4)?,
            format!("Notification class: {}", request.notification_class),
        ),
        doc.leaf(need(&items, 5)?, format!("Priority: {}", request.priority)),
        doc.leaf(
            need(&items, 6)?,
            format!("Event type: {}", kebab(request.event_type)),
        ),
    ];
    if let Some(item) = find(&items, 7) {
        let mut field = match &request.message_text {
            Some(text) => {
                let quoted = format_property_value(&PropertyValue::CharacterString(text.clone()));
                doc.leaf(item, format!("Message text: {quoted}"))
            }
            None => doc.leaf(item, "Message text: in a character set not decoded"),
        };
        field.device_text = request.message_text.is_some();
        fields.push(field);
    }
    fields.push(doc.leaf(
        need(&items, 8)?,
        format!("Notify type: {}", kebab(request.notify_type)),
    ));
    fields.extend(opt(doc, &items, 9, |_| {
        Some(format!("Ack required: {}", request.ack_required))
    }));
    fields.extend(opt(doc, &items, 10, |_| {
        Some(format!("From state: {}", kebab(request.from_state)))
    }));
    fields.push(doc.leaf(
        need(&items, 11)?,
        format!("To state: {}", kebab(request.to_state)),
    ));
    if let (Some(item), Some(values)) = (find(&items, 12), &request.event_values) {
        let values = EventValues::with_floats(values, Floats::Display);
        let mut field = Field::node(
            format!("Event values: {}", values.text()),
            doc.abs(&item.range),
            doc.tags_of(item),
        );
        field.device_text = values.data.may_hold_device_text();
        fields.push(field);
    }
    Ok(fields)
}

fn event_information(doc: &Doc<'_>, ack: &GetEventInformationAck) -> Named {
    let items = doc.items()?;
    let list = need(&items, 0)?;
    let inner = doc.items_in(&list.content)?;
    let summaries = &ack.list_of_event_summaries;
    let mut rows = Vec::new();
    for (run, summary) in runs(&inner, 0, summaries.len())?.into_iter().zip(summaries) {
        let data = EventSummaryData::new(summary);
        let object = cell(&data.object);
        let state = cell(&data.event_state);
        let acked = cell(&data.acknowledged_transitions);
        let enabled = cell(&data.event_enable);
        let notify = cell(&data.notify_type);
        let offnormal = cell(&data.event_timestamps.to_offnormal);
        let fault = cell(&data.event_timestamps.to_fault);
        let normal = cell(&data.event_timestamps.to_normal);
        let p_offnormal = cell(&data.event_priorities.to_offnormal);
        let p_fault = cell(&data.event_priorities.to_fault);
        let p_normal = cell(&data.event_priorities.to_normal);
        let kids = vec![
            doc.leaf(need(run, 0)?, format!("Object: {object}")),
            doc.leaf(need(run, 1)?, format!("Event state: {state}")),
            doc.leaf(need(run, 2)?, format!("Acked transitions: {acked}")),
            doc.leaf(
                need(run, 3)?,
                format!(
                    "Event time stamps: to-offnormal {offnormal}, to-fault {fault}, \
                     to-normal {normal}"
                ),
            ),
            doc.leaf(need(run, 4)?, format!("Notify type: {notify}")),
            doc.leaf(need(run, 5)?, format!("Event enable: {enabled}")),
            doc.leaf(
                need(run, 6)?,
                format!(
                    "Event priorities: to-offnormal {p_offnormal}, to-fault {p_fault}, \
                     to-normal {p_normal}"
                ),
            ),
        ];
        rows.push(Field::node(
            format!("{object}: {state}"),
            span(doc, run),
            kids,
        ));
    }
    let count = summaries.len();
    let noun = if count == 1 { "summary" } else { "summaries" };
    Ok(vec![
        Field::node(
            format!("Event summaries: {count} {noun}"),
            doc.abs(&list.range),
            rows,
        ),
        doc.leaf(
            need(&items, 1)?,
            format!("More events: {}", ack.more_events),
        ),
    ])
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "service_fallback_tests.rs"]
mod fallback_tests;
