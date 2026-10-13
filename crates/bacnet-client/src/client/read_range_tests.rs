//! Strict and lenient ReadRange against a device that numbers the record
//! after its sequence wrap 0 (#1531).
use bacnet_encoding::constructed::encode_log_record;
use bacnet_services::read_range::{
    RangeSpec, ReadRangeAck, ReadRangeRequest, ReadRangeValidation, ReadRangeViolation,
};
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::error::Error;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

use super::fake_device::{client_with_device, DEVICE_MAC};

fn trend_log() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::TREND_LOG, 1).unwrap()
}

fn records(count: u8) -> Vec<BACnetLogRecord> {
    (0..count)
        .map(|second| BACnetLogRecord {
            date: Date {
                year: 126,
                month: 10,
                day: 5,
                day_of_week: 1,
            },
            time: Time {
                hour: 9,
                minute: 0,
                second,
                hundredths: 0,
            },
            log_datum: LogDatum::RealValue(f32::from(second)),
            status_flags: None,
        })
        .collect()
}

/// A well-formed ten-record page whose first sequence number is 0.
fn wrapped_page() -> Vec<u8> {
    let mut item_data = BytesMut::new();
    for record in records(10) {
        encode_log_record(&record, &mut item_data).unwrap();
    }
    let ack = ReadRangeAck {
        object_identifier: trend_log(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        result_flags: (false, false, true),
        item_count: 10,
        item_data: item_data.to_vec(),
        first_sequence_number: Some(0),
    };
    let mut buf = BytesMut::new();
    ack.encode(&mut buf);
    buf.to_vec()
}

fn request() -> ReadRangeRequest {
    ReadRangeRequest {
        object_identifier: trend_log(),
        property_identifier: PropertyIdentifier::LOG_BUFFER,
        property_array_index: None,
        range: Some(RangeSpec::BySequenceNumber {
            reference_seq: 1,
            count: 10,
        }),
    }
}

#[tokio::test]
async fn strict_refuses_a_zero_first_sequence_page_with_the_rule_it_broke() {
    let (mut client, device) = client_with_device(|_, _| Ok(wrapped_page())).await;
    let request = request();
    let error = client
        .read_range(
            &DEVICE_MAC,
            request.object_identifier,
            request.property_identifier,
            None,
            request.range.clone(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            Error::ReadRangeViolation(ReadRangeViolation::ZeroFirstSequenceNumber)
        ),
        "{error:?}"
    );
    let error = client
        .read_range_with(&DEVICE_MAC, &request, ReadRangeValidation::Strict)
        .await
        .unwrap_err();
    assert!(matches!(error, Error::ReadRangeViolation(_)));
    assert_eq!(device.count(ConfirmedServiceChoice::READ_RANGE), 2);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn lenient_keeps_the_page_and_names_the_rule_in_one_request() {
    let (mut client, device) = client_with_device(|_, _| Ok(wrapped_page())).await;
    let reply = client
        .read_range_with(&DEVICE_MAC, &request(), ReadRangeValidation::Lenient)
        .await
        .unwrap();
    assert_eq!(
        reply.violations,
        [ReadRangeViolation::ZeroFirstSequenceNumber]
    );
    assert_eq!(reply.ack.first_sequence_number, Some(0));
    assert_eq!(reply.ack.trend_log_records().unwrap(), records(10));
    assert_eq!(device.count(ConfirmedServiceChoice::READ_RANGE), 1);
    client.stop().await.unwrap();
}

#[tokio::test]
async fn lenient_still_refuses_an_answer_that_does_not_decode() {
    let (mut client, _device) = client_with_device(|_, _| Ok(vec![0x0C, 0x00])).await;
    let error = client
        .read_range_with(&DEVICE_MAC, &request(), ReadRangeValidation::Lenient)
        .await
        .unwrap_err();
    assert!(
        matches!(error, Error::Decoding { .. } | Error::BufferTooShort { .. }),
        "{error:?}"
    );
    client.stop().await.unwrap();
}
