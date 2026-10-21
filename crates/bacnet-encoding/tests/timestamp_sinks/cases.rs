use bacnet_encoding::primitives::{encode_timestamp_choice_to_sink, encode_timestamp_to_sink};
use bacnet_encoding::sink::{EncodeSink, SinkFull};
use bacnet_types::primitives::{BACnetTimeStamp, Date, Time};

pub struct Case {
    pub name: String,
    pub timestamp: BACnetTimeStamp,
    pub outer: Option<u8>,
    pub expected: Vec<u8>,
}

impl Case {
    pub fn encode<S: EncodeSink + ?Sized>(&self, sink: &mut S) -> Result<usize, SinkFull> {
        match self.outer {
            None => encode_timestamp_choice_to_sink(sink, &self.timestamp),
            Some(tag) => encode_timestamp_to_sink(sink, tag, &self.timestamp),
        }
    }
}

fn time(fields: [u8; 4]) -> Time {
    Time {
        hour: fields[0],
        minute: fields[1],
        second: fields[2],
        hundredths: fields[3],
    }
}

fn datetime(date: [u8; 4], clock: [u8; 4]) -> BACnetTimeStamp {
    BACnetTimeStamp::DateTime {
        date: Date {
            year: date[0],
            month: date[1],
            day: date[2],
            day_of_week: date[3],
        },
        time: time(clock),
    }
}

// Independently tabulated headers and payloads, not encoder/decoder output.
fn wire_choices() -> Vec<(&'static str, BACnetTimeStamp, &'static [u8])> {
    vec![
        (
            "time-zero",
            BACnetTimeStamp::Time(time([0; 4])),
            &[0x0C, 0, 0, 0, 0],
        ),
        (
            "time-end",
            BACnetTimeStamp::Time(time([23, 59, 59, 99])),
            &[0x0C, 0x17, 0x3B, 0x3B, 0x63],
        ),
        (
            "time-wildcard",
            BACnetTimeStamp::Time(time([255; 4])),
            &[0x0C, 0xFF, 0xFF, 0xFF, 0xFF],
        ),
        (
            "sequence-zero",
            BACnetTimeStamp::SequenceNumber(0),
            &[0x19, 0x00],
        ),
        (
            "sequence-255",
            BACnetTimeStamp::SequenceNumber(255),
            &[0x19, 0xFF],
        ),
        (
            "sequence-256",
            BACnetTimeStamp::SequenceNumber(256),
            &[0x1A, 0x01, 0x00],
        ),
        (
            "sequence-max",
            BACnetTimeStamp::SequenceNumber(65535),
            &[0x1A, 0xFF, 0xFF],
        ),
        (
            "datetime",
            datetime([124, 2, 29, 4], [23, 59, 59, 99]),
            &[
                0x2E, 0xA4, 0x7C, 0x02, 0x1D, 0x04, 0xB4, 0x17, 0x3B, 0x3B, 0x63, 0x2F,
            ],
        ),
        (
            "datetime-wildcard",
            datetime([255; 4], [255; 4]),
            &[
                0x2E, 0xA4, 0xFF, 0xFF, 0xFF, 0xFF, 0xB4, 0xFF, 0xFF, 0xFF, 0xFF, 0x2F,
            ],
        ),
    ]
}

// Public-field and tag-255 compatibility cases are not validity assertions.
fn unchecked_choices() -> Vec<(&'static str, BACnetTimeStamp, &'static [u8])> {
    vec![
        (
            "unchecked-time",
            BACnetTimeStamp::Time(time([254, 60, 200, 250])),
            &[0x0C, 0xFE, 0x3C, 0xC8, 0xFA],
        ),
        (
            "unchecked-datetime",
            datetime([128, 0, 32, 0], [254, 60, 200, 250]),
            &[
                0x2E, 0xA4, 0x80, 0x00, 0x20, 0x00, 0xB4, 0xFE, 0x3C, 0xC8, 0xFA, 0x2F,
            ],
        ),
    ]
}

// Explicit marker literals also exercise inline/extended and nested tag 2.
const OUTER: &[(u8, &[u8], &[u8])] = &[
    (0, &[0x0E], &[0x0F]),
    (2, &[0x2E], &[0x2F]),
    (14, &[0xEE], &[0xEF]),
    (15, &[0xFE, 0x0F], &[0xFF, 0x0F]),
    (254, &[0xFE, 0xFE], &[0xFF, 0xFE]),
];

fn expand(choices: Vec<(&str, BACnetTimeStamp, &[u8])>) -> Vec<Case> {
    let mut cases = Vec::new();
    for (name, timestamp, bytes) in choices {
        cases.push(Case {
            name: format!("{name}/bare"),
            timestamp: timestamp.clone(),
            outer: None,
            expected: bytes.to_vec(),
        });
        for &(tag, opening, closing) in OUTER {
            cases.push(Case {
                name: format!("{name}/outer-{tag}"),
                timestamp: timestamp.clone(),
                outer: Some(tag),
                expected: [opening, bytes, closing].concat(),
            });
        }
    }
    cases
}

pub fn wire_cases() -> Vec<Case> {
    expand(wire_choices())
}

pub fn compatibility_cases() -> Vec<Case> {
    let mut cases = expand(unchecked_choices());
    for (name, timestamp, bytes) in wire_choices().into_iter().chain(unchecked_choices()) {
        cases.push(Case {
            name: format!("{name}/compat-outer-255"),
            timestamp,
            outer: Some(255),
            expected: [&[0xFE, 0xFF], bytes, &[0xFF, 0xFF]].concat(),
        });
    }
    cases
}

pub fn all_cases() -> Vec<Case> {
    wire_cases()
        .into_iter()
        .chain(compatibility_cases())
        .collect()
}
