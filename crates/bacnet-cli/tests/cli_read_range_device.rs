//! `bacnet read-range` against a scripted device that breaks ReadRange
//! rules or stops answering (#1532): a single read shows the rules broken
//! unless `--strict`, and `--all` prints what it read and where to resume
//! before the error that stopped it.
#[allow(dead_code)]
mod support;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use bacnet_encoding::apdu::{Apdu, ComplexAck, decode_apdu, encode_apdu};
use bacnet_encoding::constructed::encode_log_record;
use bacnet_encoding::npdu::{Npdu, decode_npdu, encode_npdu};
use bacnet_services::read_property::ReadPropertyRequest;
use bacnet_services::read_range::{RangeSpec, ReadRangeAck, ReadRangeRequest};
use bacnet_types::constructed::{BACnetLogRecord, LogDatum};
use bacnet_types::enums::{ConfirmedServiceChoice, PropertyIdentifier};
use bacnet_types::primitives::{Date, Time};
use bytes::{Bytes, BytesMut};
use support::run;
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// Records numbered `first..first + count`, each holding its number.
fn page(
    request: &ReadRangeRequest,
    first: u64,
    count: u64,
    flags: (bool, bool, bool),
) -> ReadRangeAck {
    let mut item_data = BytesMut::new();
    for sequence in first..first + count {
        let record = BACnetLogRecord {
            date: Date {
                year: 126,
                month: 10,
                day: 5,
                day_of_week: 1,
            },
            time: Time {
                hour: 9,
                minute: 0,
                second: sequence as u8,
                hundredths: 0,
            },
            log_datum: LogDatum::UnsignedValue(sequence),
            status_flags: None,
        };
        encode_log_record(&record, &mut item_data).unwrap();
    }
    ReadRangeAck {
        object_identifier: request.object_identifier,
        property_identifier: request.property_identifier,
        property_array_index: None,
        result_flags: flags,
        item_count: count as u32,
        item_data: item_data.to_vec(),
        first_sequence_number: Some(first),
    }
}

/// A B/IP device on loopback that answers each ReadRange with what
/// `answer` gives, or not at all for `None`.
async fn device<F>(mut answer: F) -> (String, JoinHandle<()>)
where
    F: FnMut(&ReadRangeRequest) -> Option<ReadRangeAck> + Send + 'static,
{
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap().to_string();
    let task = tokio::spawn(async move {
        let mut frame = [0u8; 1500];
        loop {
            let Ok((length, peer)) = socket.recv_from(&mut frame).await else {
                return;
            };
            let Some((invoke_id, request)) = read_range_request(&frame[..length]) else {
                continue;
            };
            if let Some(ack) = answer(&request) {
                reply(&socket, peer, invoke_id, &ack).await;
            }
        }
    });
    (address, task)
}

fn read_range_request(frame: &[u8]) -> Option<(u8, ReadRangeRequest)> {
    let npdu = decode_npdu(Bytes::copy_from_slice(frame.get(4..)?)).ok()?;
    let Apdu::ConfirmedRequest(request) = decode_apdu(npdu.payload).ok()? else {
        return None;
    };
    if request.service_choice != ConfirmedServiceChoice::READ_RANGE {
        return None;
    }
    Some((
        request.invoke_id,
        ReadRangeRequest::decode(&request.service_request).ok()?,
    ))
}

async fn reply(socket: &UdpSocket, peer: SocketAddr, invoke_id: u8, ack: &ReadRangeAck) {
    let mut service_ack = BytesMut::new();
    ack.encode(&mut service_ack);
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ComplexAck(ComplexAck {
            segmented: false,
            more_follows: false,
            invoke_id,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: ConfirmedServiceChoice::READ_RANGE,
            service_ack: service_ack.freeze(),
        }),
    )
    .unwrap();
    let mut npdu = BytesMut::new();
    encode_npdu(
        &mut npdu,
        &Npdu {
            payload: apdu.freeze(),
            ..Npdu::default()
        },
    )
    .unwrap();
    let mut frame = vec![0x81, 0x0A];
    frame.extend_from_slice(&((4 + npdu.len()) as u16).to_be_bytes());
    frame.extend_from_slice(&npdu);
    socket.send_to(&frame, peer).await.unwrap();
}

/// `read-range` of Trend Log 1 at `target` with `flags`.
async fn read_range(target: &str, format: &str, flags: &[&str]) -> std::process::Output {
    let mut args = vec![
        "--interface",
        "127.0.0.1",
        "--port",
        "0",
        "--timeout",
        "100",
        "--format",
        format,
        "read-range",
        target,
        "trend-log:1",
    ];
    args.extend_from_slice(flags);
    run(args).await
}

fn values(json: &serde_json::Value) -> Vec<u64> {
    json["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["datum"].as_str().unwrap().parse().unwrap())
        .collect()
}

/// A device that numbers its first record after the wrap 0: a single read
/// shows the page and the rule it broke; `--strict` refuses it.
#[tokio::test]
async fn a_single_read_shows_the_rules_broken_unless_strict() {
    let (target, task) = device(|request| Some(page(request, 0, 2, (true, true, false)))).await;
    let range = ["--sequence", "0", "--count", "5"];

    let output = read_range(&target, "json", &range).await;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(values(&json), [0, 1]);
    assert_eq!(
        json["violations"],
        serde_json::json!(["zero_first_sequence_number"])
    );
    assert_eq!(json["first_sequence_number"], serde_json::json!(0));

    let output = read_range(&target, "table", &range).await;
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(
        table
            .lines()
            .next()
            .unwrap()
            .ends_with("violations=zero_first_sequence_number"),
        "{table}"
    );

    let mut strict = range.to_vec();
    strict.push("--strict");
    let output = read_range(&target, "json", &strict).await;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ZeroFirstSequenceNumber"), "{stderr}");
    task.abort();
}

/// A device that stops answering after the first page: `--all` prints the
/// page it read and the flags that resume, then fails with the timeout.
#[tokio::test]
async fn all_prints_what_it_read_and_where_to_resume_before_an_error() {
    let mut answered = false;
    let (target, task) = device(move |request| {
        let first = matches!(
            request.range,
            Some(RangeSpec::BySequenceNumber {
                reference_seq: 1,
                ..
            })
        );
        (first && !std::mem::replace(&mut answered, true))
            .then(|| page(request, 1, 2, (true, false, true)))
    })
    .await;

    let output = read_range(
        &target,
        "json",
        &["--all", "--sequence", "1", "--count", "2"],
    )
    .await;
    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(values(&json), [1, 2]);
    assert_eq!(json["pages"], serde_json::json!(1));
    assert_eq!(json["next"], serde_json::json!("--sequence 3"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Abort { reason: AbortReason::TSM_TIMEOUT }"),
        "{stderr}"
    );
    task.abort();
}

#[tokio::test]
async fn the_minimum_interval_is_at_most_an_hour() {
    let output = run([
        "--min-interval-ms",
        "3600001",
        "read-range",
        "127.0.0.1:47808",
        "trend-log:1",
    ])
    .await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("3600000"), "{stderr}");
}

/// What a silent device was asked: each confirmed request's service, and
/// for a ReadProperty the property.
type Asked = Arc<Mutex<Vec<(ConfirmedServiceChoice, Option<PropertyIdentifier>)>>>;

/// A B/IP device on loopback that answers nothing and keeps what it is
/// asked.
async fn silent() -> (String, Asked, JoinHandle<()>) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap().to_string();
    let asked: Asked = Arc::default();
    let log = Arc::clone(&asked);
    let task = tokio::spawn(async move {
        let mut frame = [0u8; 1500];
        while let Ok((length, _)) = socket.recv_from(&mut frame).await {
            let Some(npdu) = frame
                .get(4..length)
                .and_then(|npdu| decode_npdu(Bytes::copy_from_slice(npdu)).ok())
            else {
                continue;
            };
            if let Ok(Apdu::ConfirmedRequest(request)) = decode_apdu(npdu.payload) {
                let property = (request.service_choice == ConfirmedServiceChoice::READ_PROPERTY)
                    .then(|| ReadPropertyRequest::decode(&request.service_request).ok())
                    .flatten()
                    .map(|read| read.property_identifier);
                log.lock().unwrap().push((request.service_choice, property));
            }
        }
    });
    (address, asked, task)
}

/// A device that doesn't answer costs one timeout: the Trend Log's
/// Log_DeviceObjectProperty is read only once the device has answered, so
/// it is never asked for here (#2091).
#[tokio::test]
async fn a_silent_device_is_never_asked_for_the_reference() {
    for flags in [&[][..], &["--all"][..]] {
        let (target, asked, task) = silent().await;
        let output = read_range(&target, "json", flags).await;
        assert!(!output.status.success(), "{flags:?}");
        let asked = asked.lock().unwrap().clone();
        assert!(!asked.is_empty(), "{flags:?}");
        assert!(
            asked
                .iter()
                .all(|(_, property)| *property
                    != Some(PropertyIdentifier::LOG_DEVICE_OBJECT_PROPERTY)),
            "{flags:?}: {asked:?}"
        );
        if flags.is_empty() {
            assert!(
                asked
                    .iter()
                    .all(|(service, _)| *service == ConfirmedServiceChoice::READ_RANGE),
                "{asked:?}"
            );
        }
        task.abort();
    }
}

/// A logged string that holds an override, an escape sequence and a
/// newline prints cleaned in the table and exactly in JSON (#2144).
#[tokio::test]
async fn a_logged_string_prints_cleaned_in_the_table() {
    let text = "Door\u{202E} open\u{1b}[2J\nnow";
    let (target, task) = device(move |request| {
        let mut value = BytesMut::new();
        bacnet_encoding::primitives::encode_app_character_string(&mut value, text).unwrap();
        let record = BACnetLogRecord {
            date: Date {
                year: 126,
                month: 10,
                day: 5,
                day_of_week: 1,
            },
            time: Time {
                hour: 9,
                minute: 0,
                second: 1,
                hundredths: 0,
            },
            log_datum: LogDatum::AnyValue(value.to_vec()),
            status_flags: None,
        };
        let mut item_data = BytesMut::new();
        encode_log_record(&record, &mut item_data).unwrap();
        let mut ack = page(request, 1, 1, (true, true, false));
        ack.item_data = item_data.to_vec();
        Some(ack)
    })
    .await;
    let range = ["--sequence", "1", "--count", "1"];
    let output = read_range(&target, "table", &range).await;
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(table.contains("\"Door open [2J now\""), "{table:?}");
    let output = read_range(&target, "json", &range).await;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["records"][0]["datum"], format!("\"{text}\""), "{json}");
    task.abort();
}

/// A list that isn't a log prints one item a line; a string item holding
/// an override, an escape sequence and a newline prints cleaned, and
/// exactly in JSON (#2144).
#[tokio::test]
async fn a_list_item_string_prints_cleaned() {
    let text = "Door\u{202E} open\u{1b}[2J\nnow";
    let (target, task) = device(move |request| {
        let mut item_data = BytesMut::new();
        bacnet_encoding::primitives::encode_app_character_string(&mut item_data, text).unwrap();
        Some(ReadRangeAck {
            object_identifier: request.object_identifier,
            property_identifier: request.property_identifier,
            property_array_index: None,
            result_flags: (true, true, false),
            item_count: 1,
            item_data: item_data.to_vec(),
            first_sequence_number: None,
        })
    })
    .await;
    let read = |format: &'static str| {
        run([
            "--interface",
            "127.0.0.1",
            "--port",
            "0",
            "--timeout",
            "100",
            "--format",
            format,
            "read-range",
            &target,
            "msv:1",
            "state-text",
        ])
    };
    let output = read("table").await;
    let table = String::from_utf8(output.stdout).unwrap();
    assert!(table.contains("  [1] \"Door open [2J now\""), "{table:?}");
    let output = read("json").await;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["items"][0], format!("\"{text}\""), "{json}");
    task.abort();
}
