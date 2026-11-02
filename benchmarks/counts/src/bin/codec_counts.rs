//! The `benches/encoding.rs` closures, looped with `black_box` instead of
//! timed by Criterion, for callgrind instruction counts per iteration.
//!
//! `codec-counts <workload> <n>` runs 200 warm-up iterations, checks the
//! workload's output against literal wire bytes or decoded fields, runs `n`
//! counted iterations, checks again and prints the counted loop's allocator
//! totals. `codec-counts list` prints the workload names, which are the
//! Criterion benchmark names.

use std::hint::black_box;

use bacnet_counts::{AllocSnapshot, CountingAlloc, WARMUP, parse_n, report, usage_exit};
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_types::enums::{ConfirmedServiceChoice, ObjectType, PropertyIdentifier};
use bacnet_types::primitives::ObjectIdentifier;
use bytes::{Bytes, BytesMut};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const USAGE: &str = "codec-counts <workload> <n> | codec-counts list";

const WORKLOADS: [&str; 7] = [
    "encode_read_property_request",
    "decode_read_property_request",
    "encode_npdu_100b_payload",
    "decode_npdu_100b_payload",
    "encode_apdu_confirmed_request",
    "decode_apdu_confirmed_request",
    "full_stack_encode_rp",
];

/// The ReadProperty request's service data: AI 1 in context tag 0, Present
/// Value (85) in context tag 1.
const RP_REQUEST: [u8; 7] = [0x0C, 0x00, 0x00, 0x00, 0x01, 0x19, 0x55];
/// The confirmed-request APDU header the benchmarks encode: segmented
/// response accepted, no max-segments limit, max APDU 1476 (code 5), invoke
/// ID 1, ReadProperty (12).
const APDU_HEADER: [u8; 4] = [0x02, 0x05, 0x01, 0x0C];
/// A version-1 NPDU with no control bits set.
const NPDU_HEADER: [u8; 2] = [0x01, 0x00];

fn rp_request() -> ReadPropertyRequest {
    ReadPropertyRequest {
        object_identifier: ObjectIdentifier::new(ObjectType::ANALOG_INPUT, 1).expect("valid AI"),
        property_identifier: PropertyIdentifier::PRESENT_VALUE,
        property_array_index: None,
    }
}

fn confirmed_rp(service_request: Bytes) -> Apdu {
    Apdu::ConfirmedRequest(ConfirmedRequest {
        segmented: false,
        more_follows: false,
        segmented_response_accepted: true,
        max_segments: None,
        max_apdu_length: 1476,
        invoke_id: 1,
        sequence_number: None,
        proposed_window_size: None,
        service_choice: ConfirmedServiceChoice::READ_PROPERTY,
        service_request,
    })
}

fn npdu_100b() -> Npdu {
    Npdu {
        payload: Bytes::from(vec![0u8; 100]),
        ..Default::default()
    }
}

fn concat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

/// Warm-up, oracle, counted loop, oracle. `iteration` is the benchmark's
/// closure body and returns its output for the oracle; the counted loop
/// passes that output to `black_box`, as Criterion's `iter` does.
fn measure<T>(
    name: &str,
    n: u64,
    mut iteration: impl FnMut() -> T,
    oracle: impl Fn(&T),
) -> AllocSnapshot {
    for _ in 0..WARMUP {
        black_box(iteration());
    }
    oracle(&iteration());
    let start = AllocSnapshot::now();
    for _ in 0..n {
        black_box(iteration());
    }
    let counted = AllocSnapshot::now().since(start);
    oracle(&iteration());
    report(name, "none", n, counted);
    counted
}

fn run(name: &str, n: u64) {
    match name {
        "encode_read_property_request" => {
            let req = rp_request();
            measure(
                name,
                n,
                || {
                    let mut buf = BytesMut::with_capacity(64);
                    req.encode(&mut buf);
                    buf
                },
                |buf| assert_eq!(&buf[..], &RP_REQUEST),
            );
        }
        "decode_read_property_request" => {
            let mut buf = BytesMut::new();
            rp_request().encode(&mut buf);
            let data = buf.to_vec();
            let expected = rp_request();
            measure(
                name,
                n,
                || ReadPropertyRequest::decode(&data).expect("decode"),
                |req| {
                    assert_eq!(req.object_identifier, expected.object_identifier);
                    assert_eq!(req.property_identifier, expected.property_identifier);
                    assert_eq!(req.property_array_index, None);
                },
            );
        }
        "encode_npdu_100b_payload" => {
            let npdu = npdu_100b();
            let expected = concat(&[&NPDU_HEADER, &[0u8; 100]]);
            measure(
                name,
                n,
                || {
                    let mut buf = BytesMut::with_capacity(128);
                    encode_npdu(&mut buf, &npdu).expect("encode NPDU");
                    buf
                },
                |buf| assert_eq!(&buf[..], &expected[..]),
            );
        }
        "decode_npdu_100b_payload" => {
            let mut buf = BytesMut::new();
            encode_npdu(&mut buf, &npdu_100b()).expect("encode NPDU");
            let data = Bytes::from(buf.to_vec());
            measure(
                name,
                n,
                || decode_npdu(data.clone()).expect("decode NPDU"),
                |npdu| assert_eq!(npdu, &npdu_100b()),
            );
        }
        "encode_apdu_confirmed_request" => {
            let apdu = confirmed_rp(Bytes::from(vec![0u8; 20]));
            let expected = concat(&[&APDU_HEADER, &[0u8; 20]]);
            measure(
                name,
                n,
                || {
                    let mut buf = BytesMut::with_capacity(64);
                    encode_apdu(&mut buf, &apdu).expect("valid APDU encoding");
                    buf
                },
                |buf| assert_eq!(&buf[..], &expected[..]),
            );
        }
        "decode_apdu_confirmed_request" => {
            let mut buf = BytesMut::new();
            let apdu = confirmed_rp(Bytes::from(vec![0u8; 20]));
            encode_apdu(&mut buf, &apdu).expect("valid APDU encoding");
            let data = Bytes::from(buf.to_vec());
            measure(
                name,
                n,
                || decode_apdu(data.clone()).expect("decode APDU"),
                |decoded| assert_eq!(decoded, &apdu),
            );
        }
        "full_stack_encode_rp" => {
            let req = rp_request();
            let expected = concat(&[&NPDU_HEADER, &APDU_HEADER, &RP_REQUEST]);
            measure(
                name,
                n,
                || {
                    let mut svc_buf = BytesMut::with_capacity(32);
                    req.encode(&mut svc_buf);
                    let apdu = confirmed_rp(Bytes::from(svc_buf.to_vec()));
                    let mut apdu_buf = BytesMut::with_capacity(64);
                    encode_apdu(&mut apdu_buf, &apdu).expect("valid APDU encoding");
                    let npdu = Npdu {
                        payload: Bytes::from(apdu_buf.to_vec()),
                        ..Default::default()
                    };
                    let mut npdu_buf = BytesMut::with_capacity(128);
                    encode_npdu(&mut npdu_buf, &npdu).expect("encode NPDU");
                    npdu_buf
                },
                |buf| assert_eq!(&buf[..], &expected[..]),
            );
        }
        _ => usage_exit(USAGE),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let name = args.next().unwrap_or_default();
    if name == "list" {
        for workload in WORKLOADS {
            println!("{workload}");
        }
        return;
    }
    if !WORKLOADS.contains(&name.as_str()) {
        usage_exit(USAGE);
    }
    let n = parse_n(args.next(), USAGE);
    run(&name, n);
}
