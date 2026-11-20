//! Named EventNotification and GetEventInformation response fields.

use super::*;

pub(super) fn event_notification(doc: &Doc<'_>) -> Named {
    let request = decode_event_notification_tolerant(doc.data)?;
    let items = doc.items()?;
    let mut fields = vec![
        doc.leaf(
            need(items, 0)?,
            format!("Process ID: {}", request.process_identifier),
        ),
        doc.leaf(
            need(items, 1)?,
            format!(
                "Initiating device: {}",
                object_label(request.initiating_device_identifier)
            ),
        ),
        doc.leaf(
            need(items, 2)?,
            format!(
                "Event object: {}",
                object_label(request.event_object_identifier)
            ),
        ),
        doc.leaf(
            need(items, 3)?,
            format!("Time stamp: {}", timestamp_text(&request.timestamp)),
        ),
        doc.leaf(
            need(items, 4)?,
            format!("Notification class: {}", request.notification_class),
        ),
        doc.leaf(need(items, 5)?, format!("Priority: {}", request.priority)),
        doc.leaf(
            need(items, 6)?,
            format!("Event type: {}", kebab(request.event_type)),
        ),
    ];
    if let Some(item) = find(items, 7) {
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
        need(items, 8)?,
        format!("Notify type: {}", kebab(request.notify_type)),
    ));
    fields.extend(opt(doc, items, 9, |_| {
        Some(format!("Ack required: {}", request.ack_required))
    }));
    fields.extend(opt(doc, items, 10, |_| {
        Some(format!("From state: {}", kebab(request.from_state)))
    }));
    fields.push(doc.leaf(
        need(items, 11)?,
        format!("To state: {}", kebab(request.to_state)),
    ));
    if let (Some(item), Some(values)) = (find(items, 12), &request.event_values) {
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

pub(super) fn event_information(doc: &Doc<'_>, ack: &GetEventInformationAck) -> Named {
    let items = doc.items()?;
    let list = need(items, 0)?;
    let inner = doc.items_in(list)?;
    let summaries = &ack.list_of_event_summaries;
    let mut rows = Vec::new();
    for (run, summary) in runs(inner, 0, summaries.len())?.into_iter().zip(summaries) {
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
        doc.leaf(need(items, 1)?, format!("More events: {}", ack.more_events)),
    ])
}
