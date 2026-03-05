use bytes::BytesMut;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

use bacnet_encoding::apdu::{decode_apdu, encode_apdu, Apdu, ConfirmedRequest};
use bacnet_encoding::npdu::{decode_npdu, encode_npdu, Npdu};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::Bytes;

fn bench_encode_read_property_request(c: &mut Criterion) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let req = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };

    c.bench_function("encode_read_property_request", |b| {
        b.iter(|| {
            let mut buf = BytesMut::with_capacity(64);
            req.encode(&mut buf);
            black_box(buf);
        })
    });
}

fn bench_decode_read_property_request(c: &mut Criterion) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let req = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };
    let mut buf = BytesMut::new();
    req.encode(&mut buf);
    let data = buf.to_vec();

    c.bench_function("decode_read_property_request", |b| {
        b.iter(|| {
            black_box(ReadPropertyRequest::decode(&data).unwrap());
        })
    });
}

fn bench_encode_npdu(c: &mut Criterion) {
    let npdu = Npdu {
        payload: Bytes::from(vec![0u8; 100]),
        ..Default::default()
    };

    c.bench_function("encode_npdu_100b_payload", |b| {
        b.iter(|| {
            let mut buf = BytesMut::with_capacity(128);
            encode_npdu(&mut buf, &npdu).unwrap();
            black_box(buf);
        })
    });
}

fn bench_decode_npdu(c: &mut Criterion) {
    let npdu = Npdu {
        payload: Bytes::from(vec![0u8; 100]),
        ..Default::default()
    };
    let mut buf = BytesMut::new();
    encode_npdu(&mut buf, &npdu).unwrap();
    let data = Bytes::from(buf.to_vec());

    c.bench_function("decode_npdu_100b_payload", |b| {
        b.iter(|| {
            black_box(decode_npdu(data.clone()).unwrap());
        })
    });
}

fn bench_encode_apdu_confirmed(c: &mut Criterion) {
    let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::from(vec![0u8; 20]),
    });

    c.bench_function("encode_apdu_confirmed_request", |b| {
        b.iter(|| {
            let mut buf = BytesMut::with_capacity(64);
            encode_apdu(&mut buf, &apdu);
            black_box(buf);
        })
    });
}

fn bench_decode_apdu_confirmed(c: &mut Criterion) {
    let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request: Bytes::from(vec![0u8; 20]),
    });
    let mut buf = BytesMut::new();
    encode_apdu(&mut buf, &apdu);
    let data = Bytes::from(buf.to_vec());

    c.bench_function("decode_apdu_confirmed_request", |b| {
        b.iter(|| {
            black_box(decode_apdu(data.clone()).unwrap());
        })
    });
}

fn bench_full_stack_encode(c: &mut Criterion) {
    let oid = ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).unwrap();
    let req = ReadPropertyRequest {
        object_identifier: oid,
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    };

    c.bench_function("full_stack_encode_rp", |b| {
        b.iter(|| {
            let mut svc_buf = BytesMut::with_capacity(32);
            req.encode(&mut svc_buf);

            let apdu = Apdu::ConfirmedRequest(ConfirmedRequest {
                segmented: false,
                more_follows: false,
                segmented_response_accepted: true,
                max_segments: None,
                max_apdu_length: 1476,
                invoke_id: 1,
                sequence_number: None,
                proposed_window_size: None,
                service_choice: ConfirmedServiceChoice::READ_PROPERTY,
                service_request: Bytes::from(svc_buf.to_vec()),
            });
            let mut apdu_buf = BytesMut::with_capacity(64);
            encode_apdu(&mut apdu_buf, &apdu);

            let npdu = Npdu {
                payload: Bytes::from(apdu_buf.to_vec()),
                ..Default::default()
            };
            let mut npdu_buf = BytesMut::with_capacity(128);
            encode_npdu(&mut npdu_buf, &npdu).unwrap();
            black_box(npdu_buf);
        })
    });
}

criterion_group!(
    benches,
    bench_encode_read_property_request,
    bench_decode_read_property_request,
    bench_encode_npdu,
    bench_decode_npdu,
    bench_encode_apdu_confirmed,
    bench_decode_apdu_confirmed,
    bench_full_stack_encode,
);
criterion_main!(benches);
