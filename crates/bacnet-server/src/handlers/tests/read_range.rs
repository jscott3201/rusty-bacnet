use std::borrow::Cow;
use std::sync::Arc;

use super::*;
use bacnet_client::client::BACnetClient;
use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ComplexAck};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_objects::clock::{ClockFrame, ClockReader};
use bacnet_objects::event_log::EventLogObject;
use bacnet_objects::log_buffer::LogRecordIdentity;
use bacnet_objects::trend::{TrendLogMultipleObject, TrendLogObject};
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_transport::loopback::LoopbackTransport;
use bacnet_transport::port::TransportPort;
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::ConfirmedServiceChoice;
use bacnet_types::primitives::{Date, Time};
use tokio::time::{timeout, Duration};

const DATE: Date = Date {
    year: 126,
    month: 8,
    day: 31,
    day_of_week: 1,
};

fn time(hour: u8) -> Time {
    Time {
        hour,
        minute: 2,
        second: 3,
        hundredths: 4,
    }
}

fn identity(sequence_number: u32, hour: u8) -> LogRecordIdentity {
    LogRecordIdentity::new(sequence_number, DATE, time(hour)).unwrap()
}

struct ListObject {
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    items: Vec<PropertyValue>,
    identities: Option<Vec<LogRecordIdentity>>,
}

impl BACnetObject for ListObject {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.oid
    }

    fn object_name(&self) -> &str {
        "read-range-list"
    }

    fn read_property(
        &self,
        property: PropertyIdentifier,
        _array_index: Option<u32>,
    ) -> Result<PropertyValue, Error> {
        if property == self.property {
            Ok(PropertyValue::List(self.items.clone()))
        } else {
            Err(Error::Protocol {
                class: ErrorClass::PROPERTY.to_raw() as u32,
                code: ErrorCode::UNKNOWN_PROPERTY.to_raw() as u32,
            })
        }
    }

    fn write_property(
        &mut self,
        _property: PropertyIdentifier,
        _array_index: Option<u32>,
        _value: PropertyValue,
        _priority: Option<u8>,
    ) -> Result<(), Error> {
        Err(Error::Protocol {
            class: ErrorClass::PROPERTY.to_raw() as u32,
            code: ErrorCode::WRITE_ACCESS_DENIED.to_raw() as u32,
        })
    }

    fn property_list(&self) -> Cow<'static, [PropertyIdentifier]> {
        Cow::Owned(vec![self.property])
    }

    fn log_record_identities_internal(&self) -> Option<Vec<LogRecordIdentity>> {
        self.identities.clone()
    }
}

fn list_db(
    property: PropertyIdentifier,
    items: Vec<PropertyValue>,
    identities: Option<Vec<LogRecordIdentity>>,
) -> (ObjectDatabase, ObjectIdentifier) {
    let oid = ObjectIdentifier::new(ObjectType::TREND_LOG, 91).unwrap();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(ListObject {
        oid,
        property,
        items,
        identities,
    }))
    .unwrap();
    (db, oid)
}

fn call(
    db: &ObjectDatabase,
    oid: ObjectIdentifier,
    property: PropertyIdentifier,
    range: Option<RangeSpec>,
) -> Result<ReadRangeAck, Error> {
    let request = ReadRangeRequest {
        object_identifier: oid,
        property_identifier: property,
        property_array_index: None,
        range,
    };
    let mut request_bytes = BytesMut::new();
    request.encode(&mut request_bytes);
    let mut ack_bytes = BytesMut::new();
    handle_read_range(db, &request_bytes, &mut ack_bytes)?;
    ReadRangeAck::decode(&ack_bytes)
}

fn encoded_items(items: &[PropertyValue]) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    for item in items {
        encode_property_value(&mut encoded, item).unwrap();
    }
    encoded.to_vec()
}

fn unsigned_items(values: &[u64]) -> Vec<PropertyValue> {
    values
        .iter()
        .copied()
        .map(PropertyValue::Unsigned)
        .collect()
}

fn assert_ack(
    ack: &ReadRangeAck,
    expected: &[PropertyValue],
    flags: (bool, bool, bool),
    first_sequence_number: Option<u32>,
) {
    assert_eq!(ack.item_count, expected.len() as u32);
    assert_eq!(ack.item_data, encoded_items(expected));
    assert_eq!(ack.result_flags, flags);
    assert_eq!(ack.first_sequence_number, first_sequence_number);
}

#[test]
fn by_position_is_one_based_signed_and_reports_exact_endpoints() {
    let items = unsigned_items(&[10, 20, 30, 40, 50]);
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), None);
    let cases = [
        (1, 2, vec![10, 20], (true, false, false)),
        (3, 2, vec![30, 40], (false, false, false)),
        (5, 2, vec![50], (false, true, false)),
        (1, -2, vec![10], (true, false, false)),
        (3, -2, vec![20, 30], (false, false, false)),
        (5, -2, vec![40, 50], (false, true, false)),
        (1, -5, vec![10], (true, false, false)),
        (5, 5, vec![50], (false, true, false)),
    ];

    for (reference_index, count, expected, flags) in cases {
        let ack = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::ByPosition {
                reference_index,
                count,
            }),
        )
        .unwrap();
        assert_ack(&ack, &unsigned_items(&expected), flags, None);
    }

    for reference_index in [0, 6] {
        let ack = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::ByPosition {
                reference_index,
                count: 2,
            }),
        )
        .unwrap();
        assert_ack(&ack, &[], (false, false, false), None);
    }
}

#[test]
fn signed_selector_uses_resident_order_across_wrap() {
    let identities = [u32::MAX, 1, 2];
    let reference = identities.iter().position(|sequence| *sequence == 1);
    let positive = super::super::read_range::select_signed_range(identities.len(), reference, 2);
    assert_eq!(positive.range, 1..3);
    assert_eq!(positive.result_flags, (false, true, false));
    assert_eq!(&identities[positive.range], &[1, 2]);

    let negative = super::super::read_range::select_signed_range(identities.len(), reference, -2);
    assert_eq!(negative.range, 0..2);
    assert_eq!(negative.result_flags, (true, false, false));
    assert_eq!(&identities[negative.range], &[u32::MAX, 1]);

    for (reference, count) in [(None, 1), (Some(3), 1), (Some(0), 0)] {
        let empty = super::super::read_range::select_signed_range(3, reference, count);
        assert!(empty.range.is_empty());
        assert_eq!(empty.result_flags, (false, false, false));
    }
}

#[test]
fn empty_and_unbounded_lists_have_only_included_endpoint_flags() {
    let items = unsigned_items(&[10, 20]);
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items.clone(), None);
    let ack = call(&db, oid, PropertyIdentifier::LOG_BUFFER, None).unwrap();
    assert_ack(&ack, &items, (true, true, false), None);

    let (empty_db, empty_oid) = list_db(PropertyIdentifier::LOG_BUFFER, Vec::new(), None);
    let unbounded = call(&empty_db, empty_oid, PropertyIdentifier::LOG_BUFFER, None).unwrap();
    assert_ack(&unbounded, &[], (false, false, false), None);
    let positioned = call(
        &empty_db,
        empty_oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 1,
        }),
    )
    .unwrap();
    assert_ack(&positioned, &[], (false, false, false), None);
}

#[test]
fn by_sequence_uses_exact_wrapped_identity_without_sorting() {
    let items = unsigned_items(&[u32::MAX as u64, 1, 2]);
    let identities = vec![identity(u32::MAX, 1), identity(1, 2), identity(2, 3)];
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, items, Some(identities));

    let positive = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 1,
            count: 2,
        }),
    )
    .unwrap();
    assert_ack(
        &positive,
        &unsigned_items(&[1, 2]),
        (false, true, false),
        Some(1),
    );

    let negative = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 1,
            count: -2,
        }),
    )
    .unwrap();
    assert_ack(
        &negative,
        &unsigned_items(&[u32::MAX as u64, 1]),
        (true, false, false),
        Some(u32::MAX),
    );

    let absent = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 77,
            count: 2,
        }),
    )
    .unwrap();
    assert_ack(&absent, &[], (false, false, false), None);
}

#[test]
fn by_sequence_rejects_unnumbered_properties_and_misalignment() {
    for (property, identities) in [
        (
            PropertyIdentifier::PROPERTY_LIST,
            Some(vec![identity(1, 1), identity(2, 2)]),
        ),
        (PropertyIdentifier::LOG_BUFFER, None),
        (PropertyIdentifier::LOG_BUFFER, Some(vec![identity(1, 1)])),
    ] {
        let (db, oid) = list_db(property, unsigned_items(&[10, 20]), identities);
        let error = call(
            &db,
            oid,
            property,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: 1,
                count: 1,
            }),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY.to_raw() as u32
                    && code == ErrorCode::LIST_ITEM_NOT_NUMBERED.to_raw() as u32
        ));
    }
}

fn record(value: u64) -> BACnetLogRecord {
    BACnetLogRecord {
        date: DATE,
        time: time(value as u8),
        log_datum: LogDatum::UnsignedValue(value),
        status_flags: None,
    }
}

fn projected_record(value: u64) -> PropertyValue {
    PropertyValue::List(vec![
        PropertyValue::Date(DATE),
        PropertyValue::Time(time(value as u8)),
        PropertyValue::Unsigned(value),
    ])
}

#[derive(Clone, Copy)]
enum LogFamily {
    Event,
    Trend,
    TrendMultiple,
}

fn fifo_log(family: LogFamily) -> Box<dyn BACnetObject> {
    match family {
        LogFamily::Event => {
            let mut object = EventLogObject::new(1, "EL-1", 3).unwrap();
            for value in 1..=4 {
                object.add_record(record(value));
            }
            Box::new(object)
        }
        LogFamily::Trend => {
            let mut object = TrendLogObject::new(1, "TL-1", 3).unwrap();
            for value in 1..=4 {
                object.add_record(record(value));
            }
            Box::new(object)
        }
        LogFamily::TrendMultiple => {
            let mut object = TrendLogMultipleObject::new(1, "TLM-1", 3).unwrap();
            for value in 1..=4 {
                object.add_record(record(value));
            }
            Box::new(object)
        }
    }
}

#[test]
fn every_log_family_continues_by_surviving_fifo_identity() {
    for family in [LogFamily::Event, LogFamily::Trend, LogFamily::TrendMultiple] {
        let object = fifo_log(family);
        let oid = object.object_identifier();
        let mut db = ObjectDatabase::new();
        db.add(object).unwrap();
        let expected = [projected_record(2), projected_record(3)];

        let positive = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: 2,
                count: 2,
            }),
        )
        .unwrap();
        assert_ack(&positive, &expected, (true, false, false), Some(2));

        let negative = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: 3,
                count: -2,
            }),
        )
        .unwrap();
        assert_ack(&negative, &expected, (true, false, false), Some(2));

        let evicted = call(
            &db,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: 1,
                count: 1,
            }),
        )
        .unwrap();
        assert_ack(&evicted, &[], (false, false, false), None);
    }
}

struct FixedClock;

impl ClockReader for FixedClock {
    fn read_clock(&self) -> Option<ClockFrame> {
        Some(ClockFrame {
            local_date: DATE,
            local_time: time(12),
            utc_offset: 0,
            daylight_savings_status: false,
        })
    }
}

#[test]
fn purge_status_record_replaces_old_sequence_continuation() {
    let mut object = EventLogObject::new(7, "EL-7", 3).unwrap();
    object.bind_clock_internal(Some(Arc::new(FixedClock)));
    object.add_record(record(1));
    object.add_record(record(2));
    object
        .write_property(
            PropertyIdentifier::RECORD_COUNT,
            None,
            PropertyValue::Unsigned(0),
            None,
        )
        .unwrap();
    let status_sequence = object.log_record_identities_internal().unwrap()[0].sequence_number();
    assert_eq!(status_sequence, 3);
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object)).unwrap();

    let purged = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: 2,
            count: 1,
        }),
    )
    .unwrap();
    assert_ack(&purged, &[], (false, false, false), None);

    let status = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::BySequenceNumber {
            reference_seq: status_sequence,
            count: 1,
        }),
    )
    .unwrap();
    assert_eq!(status.item_count, 1);
    assert_eq!(status.result_flags, (true, true, false));
    assert_eq!(status.first_sequence_number, Some(status_sequence));
}

#[test]
fn by_time_keeps_explicit_typed_denial() {
    let (db, oid) = list_db(PropertyIdentifier::LOG_BUFFER, unsigned_items(&[1]), None);
    let error = call(
        &db,
        oid,
        PropertyIdentifier::LOG_BUFFER,
        Some(RangeSpec::ByTime {
            reference_time: (DATE, time(1)),
            count: 1,
        }),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        Error::Protocol { class, code }
            if class == ErrorClass::SERVICES.to_raw() as u32
                && code == ErrorCode::SERVICE_REQUEST_DENIED.to_raw() as u32
    ));
}

#[test]
fn item_encoding_error_keeps_response_byte_for_byte_unchanged() {
    let items = unsigned_items(&[1, 2]);
    let request = ReadRangeRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::ByPosition {
            reference_index: 1,
            count: 2,
        }),
    };
    let selection = super::super::read_range::select_signed_range(items.len(), Some(0), 2);
    let mut response = BytesMut::from(&b"existing-response"[..]);
    let before = response.clone();
    let mut encoded = 0;
    let error = super::super::read_range::append_read_range_ack_with(
        &request,
        &items,
        &selection,
        None,
        &mut response,
        |buf, item| {
            encoded += 1;
            if encoded == 2 {
                Err(Error::Encoding("synthetic item encoding failure".into()))
            } else {
                encode_property_value(buf, item)
            }
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        Error::Encoding(message) if message == "synthetic item encoding failure"
    ));
    assert_eq!(response, before);
}

#[tokio::test]
async fn client_decodes_server_ack_and_continues_by_returned_sequence() {
    let client_mac = vec![0x31];
    let server_mac = vec![0x32];
    let (client_transport, mut server_transport) =
        LoopbackTransport::pair(client_mac.clone(), server_mac.clone());
    let mut server_rx = server_transport.start().await.unwrap();

    let object = fifo_log(LogFamily::Trend);
    let oid = object.object_identifier();
    let mut db = ObjectDatabase::new();
    db.add(object).unwrap();

    let server_task = tokio::spawn(async move {
        for _ in 0..2 {
            let received = timeout(Duration::from_secs(2), server_rx.recv())
                .await
                .expect("server timed out waiting for ReadRange")
                .expect("server transport closed");
            assert_eq!(&received.source_mac[..], &client_mac);
            let npdu = decode_npdu(received.npdu).unwrap();
            let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).unwrap() else {
                panic!("expected confirmed ReadRange request");
            };
            assert_eq!(request.service_choice, ConfirmedServiceChoice::READ_RANGE);

            let mut service_ack = BytesMut::new();
            handle_read_range(&db, &request.service_request, &mut service_ack).unwrap();
            let response = Apdu::ComplexAck(ComplexAck {
                segmented: false,
                more_follows: false,
                invoke_id: request.invoke_id,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: request.service_choice,
                service_ack: service_ack.freeze(),
            });
            let mut encoded_apdu = BytesMut::new();
            encode_apdu(&mut encoded_apdu, &response).unwrap();
            let mut encoded_npdu = BytesMut::new();
            encode_npdu(
                &mut encoded_npdu,
                &Npdu {
                    payload: encoded_apdu.freeze(),
                    ..Npdu::default()
                },
            )
            .unwrap();
            server_transport
                .send_unicast(&encoded_npdu, &client_mac)
                .await
                .unwrap();
        }
        server_transport.stop().await.unwrap();
    });

    let mut client = BACnetClient::generic_builder()
        .transport(client_transport)
        .apdu_timeout_ms(2_000)
        .build()
        .await
        .unwrap();
    let first = client
        .read_range(
            &server_mac,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: 4,
                count: -3,
            }),
        )
        .await
        .unwrap();
    assert_ack(
        &first,
        &[
            projected_record(2),
            projected_record(3),
            projected_record(4),
        ],
        (true, true, false),
        Some(2),
    );

    let returned_sequence = first.first_sequence_number.unwrap();
    let continuation = client
        .read_range(
            &server_mac,
            oid,
            PropertyIdentifier::LOG_BUFFER,
            None,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: returned_sequence,
                count: 2,
            }),
        )
        .await
        .unwrap();
    assert_ack(
        &continuation,
        &[projected_record(2), projected_record(3)],
        (true, false, false),
        Some(returned_sequence),
    );

    client.stop().await.unwrap();
    server_task.await.unwrap();
}
