//! GetEventInformation with More_Events set (#2525): more objects in an
//! event state than one reply holds, so the first reply stops short, and
//! the follow-up request, carrying the last object identifier it listed,
//! gets the next page.
//!
//! The requester accepts replies of 128 octets, which two summaries of this
//! fixture fit and three don't. Both sides answer each request with the
//! same octets; and the pages are checked against the limit itself, not
//! only against each other: both run the same pagination, so a page cut
//! short would match.
use super::*;
use bacnet_services::alarm_event::{
    EventSummary, GetEventInformationAck, GetEventInformationRequest,
};

/// What the requester accepts: an Analog Value's summary, with its three
/// timestamps, is about 40 octets, so a page holds two.
const MAX_APDU: u16 = 128;

/// The ComplexACK's header ahead of the service parameters.
const HEADER: usize = 3;

/// Analog Values 1 to 5, alarming over 80 with no Time_Delay, through a
/// Notification Class with no destination.
fn five_alarms(identity: &bacnet_objects::DeviceIdentity) -> ObjectDatabase {
    let mut objects: Vec<Box<dyn BACnetObject>> = (1..=5)
        .map(|instance| Box::new(alarm_object(instance, 0)) as Box<dyn BACnetObject>)
        .collect();
    objects.push(Box::new(alarm_class(Vec::new())));
    objects.push(Box::new(NotificationClass::new(0, "NC-0").unwrap()));
    database_with(identity, objects)
}

fn page_request(after: Option<ObjectIdentifier>) -> BytesMut {
    let mut body = BytesMut::new();
    GetEventInformationRequest {
        last_received_object_identifier: after,
    }
    .encode(&mut body);
    body
}

/// The summaries of a reply, and whether it says more follow.
fn page(reply: &Apdu) -> (Vec<EventSummary>, bool, usize) {
    let Apdu::ComplexAck(ack) = reply else {
        panic!("not a ComplexACK: {reply:?}")
    };
    let decoded = GetEventInformationAck::decode(&ack.service_ack).unwrap();
    (
        decoded.list_of_event_summaries,
        decoded.more_events,
        ack.service_ack.len(),
    )
}

/// The octets `summaries` take as a reply that says more follow.
fn encoded_len(summaries: &[EventSummary]) -> usize {
    let mut encoded = BytesMut::new();
    GetEventInformationAck {
        list_of_event_summaries: summaries.to_vec(),
        more_events: true,
    }
    .encode(&mut encoded)
    .unwrap();
    encoded.len()
}

#[tokio::test(start_paused = true)]
async fn a_page_that_stops_short_says_more_and_the_next_request_continues() {
    let mut both = EventParity::start(five_alarms).await;
    for instance in 1..=5 {
        let object = oid(ObjectType::ANALOG_VALUE, instance);
        let at = 300 + 100 * u64::from(instance);
        let value = PropertyValue::Real(90.0);
        let what = format!("AV-{instance} over the limit");
        let sent = both
            .write_to(&what, at, (object, P::PRESENT_VALUE), value)
            .await;
        assert_eq!(sent, 0, "{what}: no destination");
    }
    let service = ConfirmedServiceChoice::GET_EVENT_INFORMATION;
    let room = usize::from(MAX_APDU) - HEADER;
    let mut after = None;
    let mut listed = Vec::new();
    let mut pages = Vec::new();
    let mut at = 1_000;
    loop {
        let body = page_request(after);
        let (reply, _) = both
            .request_sized("a page", at, (service, &body), MAX_APDU, None)
            .await;
        at += 100;
        let (summaries, more, length) = page(&reply);
        assert!(length <= room, "a page of {length} octets, room for {room}");
        if more {
            // The page holds as many whole summaries as fit: one more
            // would not.
            let mut next = summaries.clone();
            next.push(summaries[0].clone());
            assert!(encoded_len(&next) > room, "a page cut short: {summaries:?}");
        }
        pages.push(summaries.len());
        listed.extend(summaries.iter().map(|summary| summary.object_identifier));
        let [.., last] = summaries.as_slice() else {
            panic!("an empty page")
        };
        after = Some(last.object_identifier);
        if !more {
            break;
        }
    }
    let every: Vec<_> = (1..=5)
        .map(|instance| oid(ObjectType::ANALOG_VALUE, instance))
        .collect();
    assert_eq!(listed, every, "every object once, in order");
    assert_eq!(pages, [2, 2, 1], "two summaries fit, a third doesn't");
}
