//! Bounded retained encoding with full strict projection validation.
use super::*;
use crate::server::GetEventInformationBudget;
use bacnet_encoding::{primitives, tags};

#[derive(Debug)]
pub(crate) enum EventInformationFailure {
    Service(Error),
    Objects,
    Bytes,
}

impl From<Error> for EventInformationFailure {
    fn from(error: Error) -> Self {
        Self::Service(error)
    }
}

pub(crate) fn handle_get_event_information_configured(
    db: &ObjectDatabase,
    service_data: &[u8],
    buf: &mut BytesMut,
    budget: GetEventInformationBudget,
) -> Result<(), EventInformationFailure> {
    let request = GetEventInformationRequest::decode(service_data)?;
    // Admission uses the same database view as the scan, before any callbacks.
    if db.len() > budget.max_objects {
        return Err(EventInformationFailure::Objects);
    }
    let cursor = request
        .last_received_object_identifier
        .map(|oid| oid.encode());
    let mut identifiers = db.list_objects();
    identifiers.sort_unstable_by_key(ObjectIdentifier::encode);
    let mut page = Page::new(budget);
    for oid in identifiers {
        if cursor.is_some_and(|cursor| oid.encode() <= cursor) {
            continue;
        }
        let Some(object) = db.get(&oid) else {
            continue;
        };
        // Do not stop validating when the page is full, even for NORMAL objects.
        if let EventSummaryProjectionResult::Projected(projection) =
            EventSummaryProjection::read(object, db)?
        {
            if projection.is_selected() {
                page.push(projection.into())?;
            }
        }
    }
    page.finish(buf)
}

struct Page {
    budget: GetEventInformationBudget,
    encoded: BytesMut,
    prefix_len: usize,
    suffix_len: usize,
    count: usize,
    more: bool,
    closed: bool,
    #[cfg(test)]
    encodings: usize,
}

impl Page {
    fn new(budget: GetEventInformationBudget) -> Self {
        let mut encoded = BytesMut::new();
        tags::encode_opening_tag(&mut encoded, 0);
        let prefix_len = encoded.len();
        let suffix_len = Self::suffix(false).len();
        let closed = prefix_len + suffix_len > budget.max_service_ack_bytes;
        Self {
            budget,
            encoded,
            prefix_len,
            suffix_len,
            count: 0,
            more: false,
            closed,
            #[cfg(test)]
            encodings: 0,
        }
    }

    fn suffix(more: bool) -> BytesMut {
        let mut suffix = BytesMut::new();
        tags::encode_closing_tag(&mut suffix, 0);
        primitives::encode_ctx_boolean(&mut suffix, 1, more);
        suffix
    }

    fn push(&mut self, summary: EventSummary) -> Result<(), Error> {
        if self.closed || self.count == self.budget.max_returned_summaries {
            self.more = true;
            self.closed = true;
            return Ok(());
        }
        // Reuse the authoritative codec for one candidate, never a full-page
        // clone or prefix re-encoding. Only its small ACK wrappers are discarded.
        let mut candidate = BytesMut::new();
        #[cfg(test)]
        {
            self.encodings += 1;
        }
        GetEventInformationAck {
            list_of_event_summaries: vec![summary],
            more_events: false,
        }
        .encode(&mut candidate)?;
        let item = &candidate[self.prefix_len..candidate.len() - self.suffix_len];
        let remaining = self
            .budget
            .max_service_ack_bytes
            .saturating_sub(self.encoded.len() + self.suffix_len);
        if item.len() > remaining {
            self.more = true;
            self.closed = true;
        } else {
            self.encoded.extend_from_slice(item);
            self.count += 1;
            self.closed = self.encoded.len() + self.suffix_len == self.budget.max_service_ack_bytes;
        }
        Ok(())
    }

    fn finish(mut self, buf: &mut BytesMut) -> Result<(), EventInformationFailure> {
        if self.encoded.len() + self.suffix_len > self.budget.max_service_ack_bytes
            || (self.count == 0 && self.more)
        {
            return Err(EventInformationFailure::Bytes);
        }
        self.encoded.extend_from_slice(&Self::suffix(self.more));
        buf.extend_from_slice(&self.encoded);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(timestamp: BACnetTimeStamp, state: u32) -> EventSummary {
        EventSummary {
            object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap(),
            event_state: state,
            acknowledged_transitions: 7,
            event_timestamps: std::array::from_fn(|_| timestamp.clone()),
            notify_type: state,
            event_enable: 7,
            event_priorities: [0, 100, 255],
            notification_class: 42,
        }
    }

    #[test]
    fn get_event_information_incremental_codec_matches_wrappers_and_widths() {
        use bacnet_types::primitives::{Date, Time};
        let time = Time {
            hour: 1,
            minute: 2,
            second: 3,
            hundredths: 4,
        };
        for timestamp in [
            BACnetTimeStamp::SequenceNumber(0),
            BACnetTimeStamp::SequenceNumber(65535),
            BACnetTimeStamp::Time(time),
            BACnetTimeStamp::DateTime {
                date: Date {
                    year: 126,
                    month: 9,
                    day: 8,
                    day_of_week: 2,
                },
                time,
            },
        ] {
            for state in [0, 255, 256, 65536, u32::MAX] {
                for more in [false, true] {
                    let item = summary(timestamp.clone(), state);
                    let mut expected = BytesMut::new();
                    GetEventInformationAck {
                        list_of_event_summaries: vec![item.clone()],
                        more_events: more,
                    }
                    .encode(&mut expected)
                    .unwrap();
                    let mut page = Page::new(GetEventInformationBudget {
                        max_service_ack_bytes: expected.len(),
                        ..Default::default()
                    });
                    page.push(item.clone()).unwrap();
                    if more {
                        page.push(item.clone()).unwrap();
                    }
                    assert_eq!(page.encodings, 1);
                    let mut actual = BytesMut::new();
                    page.finish(&mut actual).unwrap();
                    assert_eq!(actual, expected);
                    let mut under = Page::new(GetEventInformationBudget {
                        max_service_ack_bytes: expected.len() - 1,
                        ..Default::default()
                    });
                    under.push(item.clone()).unwrap();
                    for _ in 0..100 {
                        under.push(item.clone()).unwrap();
                    }
                    assert_eq!(under.encodings, 1);
                    assert!(matches!(
                        under.finish(&mut BytesMut::new()),
                        Err(EventInformationFailure::Bytes)
                    ));
                }
            }
        }
    }

    #[test]
    fn get_event_information_overflow_closes_prefix_no_smaller_rescue() {
        let small = summary(BACnetTimeStamp::SequenceNumber(0), 0);
        let large = summary(BACnetTimeStamp::SequenceNumber(65535), u32::MAX);
        let mut one = BytesMut::new();
        GetEventInformationAck {
            list_of_event_summaries: vec![small.clone()],
            more_events: false,
        }
        .encode(&mut one)
        .unwrap();
        let mut page = Page::new(GetEventInformationBudget {
            max_service_ack_bytes: 2 * one.len() - 4,
            ..Default::default()
        });
        page.push(small.clone()).unwrap();
        page.push(large).unwrap();
        page.push(small).unwrap();
        assert_eq!((page.count, page.encodings, page.more), (1, 2, true));
        let mut out = BytesMut::new();
        page.finish(&mut out).unwrap();
        assert_eq!(out.len(), one.len());
        assert!(GetEventInformationAck::decode(&out).unwrap().more_events);
    }
}
