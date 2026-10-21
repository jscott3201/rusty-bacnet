use bacnet_encoding::primitives::*;
use bacnet_encoding::sink::{EncodeSink, SinkFull};
use bacnet_types::enums::ObjectType;
use bacnet_types::primitives::{Date, ObjectIdentifier, Time};
use bytes::BytesMut;

#[derive(Clone, Copy, Debug)]
pub enum Form {
    Raw,
    App,
    Context(u8),
}

#[derive(Clone, Copy, Debug)]
pub enum Value {
    Unsigned(u64),
    Signed(i32),
    Real(u32),
    Double(u64),
    Null,
    Boolean(bool),
    Enumerated(u32),
    Date([u8; 4]),
    Time([u8; 4]),
    Object(u32, u32),
}

#[derive(Clone, Copy, Debug)]
pub struct Encoding(pub Form, pub Value);

impl Encoding {
    pub fn encode(self, sink: &mut (impl EncodeSink + ?Sized)) -> Result<usize, SinkFull> {
        use Form::{App, Context as Ctx, Raw};
        use Value::*;
        match (self.0, self.1) {
            (Raw, Unsigned(v)) => encode_unsigned_to_sink(sink, v),
            (Raw, Signed(v)) => encode_signed_to_sink(sink, v),
            (Raw, Real(bits)) => encode_real_to_sink(sink, f32::from_bits(bits)),
            (Raw, Double(bits)) => encode_double_to_sink(sink, f64::from_bits(bits)),
            (App, Null) => encode_app_null_to_sink(sink),
            (App, Boolean(v)) => encode_app_boolean_to_sink(sink, v),
            (App, Unsigned(v)) => encode_app_unsigned_to_sink(sink, v),
            (App, Signed(v)) => encode_app_signed_to_sink(sink, v),
            (App, Real(bits)) => encode_app_real_to_sink(sink, f32::from_bits(bits)),
            (App, Double(bits)) => encode_app_double_to_sink(sink, f64::from_bits(bits)),
            (App, Enumerated(v)) => encode_app_enumerated_to_sink(sink, v),
            (App, Date(fields)) => encode_app_date_to_sink(sink, &date(fields)),
            (App, Time(fields)) => encode_app_time_to_sink(sink, &time(fields)),
            (App, Object(t, i)) => encode_app_object_id_to_sink(sink, &object(t, i)),
            (Ctx(tag), Unsigned(v)) => encode_ctx_unsigned_to_sink(sink, tag, v),
            (Ctx(tag), Signed(v)) => encode_ctx_signed_to_sink(sink, tag, v),
            (Ctx(tag), Real(bits)) => encode_ctx_real_to_sink(sink, tag, f32::from_bits(bits)),
            (Ctx(tag), Double(bits)) => encode_ctx_double_to_sink(sink, tag, f64::from_bits(bits)),
            (Ctx(tag), Enumerated(v)) => encode_ctx_enumerated_to_sink(sink, tag, v),
            (Ctx(tag), Boolean(v)) => encode_ctx_boolean_to_sink(sink, tag, v),
            (Ctx(tag), Object(t, i)) => encode_ctx_object_id_to_sink(sink, tag, &object(t, i)),
            (Ctx(tag), Date(fields)) => encode_ctx_date_to_sink(sink, tag, &date(fields)),
            other => panic!("fixture asks for an absent scalar entrypoint: {other:?}"),
        }
    }

    pub fn hosted(self, sink: &mut BytesMut) {
        use Form::{App, Context as Ctx, Raw};
        use Value::*;
        match (self.0, self.1) {
            (Raw, Unsigned(v)) => encode_unsigned(sink, v),
            (Raw, Signed(v)) => encode_signed(sink, v),
            (Raw, Real(bits)) => encode_real(sink, f32::from_bits(bits)),
            (Raw, Double(bits)) => encode_double(sink, f64::from_bits(bits)),
            (App, Null) => encode_app_null(sink),
            (App, Boolean(v)) => encode_app_boolean(sink, v),
            (App, Unsigned(v)) => encode_app_unsigned(sink, v),
            (App, Signed(v)) => encode_app_signed(sink, v),
            (App, Real(bits)) => encode_app_real(sink, f32::from_bits(bits)),
            (App, Double(bits)) => encode_app_double(sink, f64::from_bits(bits)),
            (App, Enumerated(v)) => encode_app_enumerated(sink, v),
            (App, Date(fields)) => encode_app_date(sink, &date(fields)),
            (App, Time(fields)) => encode_app_time(sink, &time(fields)),
            (App, Object(t, i)) => encode_app_object_id(sink, &object(t, i)),
            (Ctx(tag), Unsigned(v)) => encode_ctx_unsigned(sink, tag, v),
            (Ctx(tag), Signed(v)) => encode_ctx_signed(sink, tag, v),
            (Ctx(tag), Real(bits)) => encode_ctx_real(sink, tag, f32::from_bits(bits)),
            (Ctx(tag), Double(bits)) => encode_ctx_double(sink, tag, f64::from_bits(bits)),
            (Ctx(tag), Enumerated(v)) => encode_ctx_enumerated(sink, tag, v),
            (Ctx(tag), Boolean(v)) => encode_ctx_boolean(sink, tag, v),
            (Ctx(tag), Object(t, i)) => encode_ctx_object_id(sink, tag, &object(t, i)),
            (Ctx(tag), Date(fields)) => encode_ctx_date(sink, tag, &date(fields)),
            other => panic!("fixture asks for an absent scalar entrypoint: {other:?}"),
        }
    }
}

fn date(f: [u8; 4]) -> Date {
    Date {
        year: f[0],
        month: f[1],
        day: f[2],
        day_of_week: f[3],
    }
}
fn time(f: [u8; 4]) -> Time {
    Time {
        hour: f[0],
        minute: f[1],
        second: f[2],
        hundredths: f[3],
    }
}
fn object(t: u32, i: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::from_raw(t), i).unwrap()
}

#[derive(Debug)]
pub struct Case {
    pub encoding: Encoding,
    pub expected: Vec<u8>,
}

fn push(cases: &mut Vec<Case>, form: Form, value: Value, header: &[u8], payload: &[u8]) {
    cases.push(Case {
        encoding: Encoding(form, value),
        expected: [header, payload].concat(),
    });
}

// Literal headers indexed by the independently tabulated payload width, not
// by an encoder or its length helper. Context tags are 0, 14, 15 and 254.
const CONTEXT: [[&[u8]; 4]; 8] = [
    [&[0x09], &[0xE9], &[0xF9, 0x0F], &[0xF9, 0xFE]],
    [&[0x0A], &[0xEA], &[0xFA, 0x0F], &[0xFA, 0xFE]],
    [&[0x0B], &[0xEB], &[0xFB, 0x0F], &[0xFB, 0xFE]],
    [&[0x0C], &[0xEC], &[0xFC, 0x0F], &[0xFC, 0xFE]],
    [&[0x0D, 5], &[0xED, 5], &[0xFD, 0x0F, 5], &[0xFD, 0xFE, 5]],
    [&[0x0D, 6], &[0xED, 6], &[0xFD, 0x0F, 6], &[0xFD, 0xFE, 6]],
    [&[0x0D, 7], &[0xED, 7], &[0xFD, 0x0F, 7], &[0xFD, 0xFE, 7]],
    [&[0x0D, 8], &[0xED, 8], &[0xFD, 0x0F, 8], &[0xFD, 0xFE, 8]],
];
const APP_UNSIGNED: [&[u8]; 8] = [
    &[0x21],
    &[0x22],
    &[0x23],
    &[0x24],
    &[0x25, 5],
    &[0x25, 6],
    &[0x25, 7],
    &[0x25, 8],
];
const APP_SIGNED: [&[u8]; 4] = [&[0x31], &[0x32], &[0x33], &[0x34]];
const APP_ENUMERATED: [&[u8]; 4] = [&[0x91], &[0x92], &[0x93], &[0x94]];

fn contexts(cases: &mut Vec<Case>, value: Value, payload: &[u8]) {
    for (tag, header) in [0, 14, 15, 254].into_iter().zip(CONTEXT[payload.len() - 1]) {
        push(cases, Form::Context(tag), value, header, payload);
    }
}
fn numeric(cases: &mut Vec<Case>, value: Value, payload: &[u8], header: &[u8], raw: bool) {
    if raw {
        push(cases, Form::Raw, value, &[], payload);
    }
    push(cases, Form::App, value, header, payload);
    contexts(cases, value, payload);
}

// Independent payload octets cover both sides of every supported width change.
// ASHRAE 135-2020 20.2.1.3, 20.2.2-.7 and 20.2.11-.15 ground these
// wire-layout cases; round trips and the compatibility probe are separate.
pub const UNSIGNED: &[(u64, &[u8])] = &[
    (0, &[0]),
    (1, &[1]),
    (0xFF, &[0xFF]),
    (0x100, &[1, 0]),
    (0xFFFF, &[0xFF, 0xFF]),
    (0x10000, &[1, 0, 0]),
    (0xFFFFFF, &[0xFF, 0xFF, 0xFF]),
    (0x1000000, &[1, 0, 0, 0]),
    (0xFFFFFFFF, &[0xFF, 0xFF, 0xFF, 0xFF]),
    (0x100000000, &[1, 0, 0, 0, 0]),
    (0xFFFFFFFFFF, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
    (0x10000000000, &[1, 0, 0, 0, 0, 0]),
    (0xFFFFFFFFFFFF, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
    (0x1000000000000, &[1, 0, 0, 0, 0, 0, 0]),
    (
        0xFFFFFFFFFFFFFF,
        &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    ),
    (0x100000000000000, &[1, 0, 0, 0, 0, 0, 0, 0]),
    (u64::MAX, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
    (0x0102030405060708, &[1, 2, 3, 4, 5, 6, 7, 8]),
];
pub const SIGNED: &[(i32, &[u8])] = &[
    (i32::MIN, &[0x80, 0, 0, 0]),
    (-8_388_609, &[0xFF, 0x7F, 0xFF, 0xFF]),
    (-8_388_608, &[0x80, 0, 0]),
    (-32_769, &[0xFF, 0x7F, 0xFF]),
    (-32_768, &[0x80, 0]),
    (-129, &[0xFF, 0x7F]),
    (-128, &[0x80]),
    (-1, &[0xFF]),
    (0, &[0]),
    (1, &[1]),
    (127, &[0x7F]),
    (128, &[0x00, 0x80]),
    (32_767, &[0x7F, 0xFF]),
    (32_768, &[0, 0x80, 0]),
    (8_388_607, &[0x7F, 0xFF, 0xFF]),
    (8_388_608, &[0, 0x80, 0, 0]),
    (i32::MAX, &[0x7F, 0xFF, 0xFF, 0xFF]),
    (0x123456, &[0x12, 0x34, 0x56]),
    (-0x123456, &[0xED, 0xCB, 0xAA]),
];

pub fn wire_cases() -> Vec<Case> {
    use Value::*;
    let mut cases = Vec::new();
    for &(value, payload) in UNSIGNED {
        numeric(
            &mut cases,
            Unsigned(value),
            payload,
            APP_UNSIGNED[payload.len() - 1],
            true,
        );
    }
    for &(value, payload) in SIGNED {
        numeric(
            &mut cases,
            Signed(value),
            payload,
            APP_SIGNED[payload.len() - 1],
            true,
        );
    }
    for &(value, payload) in &[
        (0u32, &[0][..]),
        (1, &[1]),
        (0xFF, &[0xFF]),
        (0x100, &[1, 0]),
        (0xFFFF, &[0xFF, 0xFF]),
        (0x10000, &[1, 0, 0]),
        (0xFFFFFF, &[0xFF, 0xFF, 0xFF]),
        (0x1000000, &[1, 0, 0, 0]),
        (u32::MAX, &[0xFF, 0xFF, 0xFF, 0xFF]),
    ] {
        numeric(
            &mut cases,
            Enumerated(value),
            payload,
            APP_ENUMERATED[payload.len() - 1],
            false,
        );
    }
    for &(bits, payload) in &[
        (0, &[0, 0, 0, 0][..]),
        (0x80000000, &[0x80, 0, 0, 0]),
        (1, &[0, 0, 0, 1]),
        (0x3F800000, &[0x3F, 0x80, 0, 0]),
        (0xBF800000, &[0xBF, 0x80, 0, 0]),
        (0x7F7FFFFF, &[0x7F, 0x7F, 0xFF, 0xFF]),
    ] {
        numeric(&mut cases, Real(bits), payload, &[0x44], true);
    }
    for &(bits, payload) in &[
        (0, &[0, 0, 0, 0, 0, 0, 0, 0][..]),
        (0x8000000000000000, &[0x80, 0, 0, 0, 0, 0, 0, 0]),
        (1, &[0, 0, 0, 0, 0, 0, 0, 1]),
        (0x3FF0000000000000, &[0x3F, 0xF0, 0, 0, 0, 0, 0, 0]),
        (0xBFF0000000000000, &[0xBF, 0xF0, 0, 0, 0, 0, 0, 0]),
        (
            0x7FEFFFFFFFFFFFFF,
            &[0x7F, 0xEF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        ),
    ] {
        numeric(&mut cases, Double(bits), payload, &[0x55, 8], true);
    }
    push(&mut cases, Form::App, Null, &[0x00], &[]);
    push(&mut cases, Form::App, Boolean(false), &[0x10], &[]);
    push(&mut cases, Form::App, Boolean(true), &[0x11], &[]);
    contexts(&mut cases, Boolean(false), &[0]);
    contexts(&mut cases, Boolean(true), &[1]);
    for fields in [[0, 1, 1, 1], [126, 10, 18, 7], [254, 14, 34, 7], [255; 4]] {
        numeric(&mut cases, Date(fields), &fields, &[0xA4], false);
    }
    for fields in [[0; 4], [23, 59, 59, 99], [255; 4]] {
        push(&mut cases, Form::App, Time(fields), &[0xB4], &fields);
    }
    for &(kind, instance, payload) in &[
        (0, 0, &[0, 0, 0, 0][..]),
        (0, 0x3FFFFF, &[0, 0x3F, 0xFF, 0xFF]),
        (1, 0, &[0, 0x40, 0, 0]),
        (8, 1234, &[0x02, 0, 0x04, 0xD2]),
        (1023, 0, &[0xFF, 0xC0, 0, 0]),
        (1023, 0x3FFFFF, &[0xFF, 0xFF, 0xFF, 0xFF]),
    ] {
        numeric(&mut cases, Object(kind, instance), payload, &[0xC4], false);
    }
    cases
}

// These inputs are deliberately separate from standards-valid wire examples:
// NaN/infinity payload preservation, unchecked Date/Time fields and tag 255.
pub fn compatibility_cases() -> Vec<Case> {
    use Value::*;
    let mut cases = Vec::new();
    for &(bits, payload) in &[
        (0x7F800000, &[0x7F, 0x80, 0, 0][..]),
        (0xFF800000, &[0xFF, 0x80, 0, 0]),
        (0x7FC12345, &[0x7F, 0xC1, 0x23, 0x45]),
        (0x7F812345, &[0x7F, 0x81, 0x23, 0x45]),
        (0xFFCABCDE, &[0xFF, 0xCA, 0xBC, 0xDE]),
    ] {
        numeric(&mut cases, Real(bits), payload, &[0x44], true);
    }
    for &(bits, payload) in &[
        (0x7FF0000000000000, &[0x7F, 0xF0, 0, 0, 0, 0, 0, 0][..]),
        (0xFFF0000000000000, &[0xFF, 0xF0, 0, 0, 0, 0, 0, 0]),
        (
            0x7FF8123456789ABC,
            &[0x7F, 0xF8, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC],
        ),
        (
            0x7FF0123456789ABC,
            &[0x7F, 0xF0, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC],
        ),
        (
            0xFFF8ABCDEF012345,
            &[0xFF, 0xF8, 0xAB, 0xCD, 0xEF, 0x01, 0x23, 0x45],
        ),
    ] {
        numeric(&mut cases, Double(bits), payload, &[0x55, 8], true);
    }
    numeric(
        &mut cases,
        Date([1, 0, 254, 8]),
        &[1, 0, 254, 8],
        &[0xA4],
        false,
    );
    push(
        &mut cases,
        Form::App,
        Time([254, 60, 60, 100]),
        &[0xB4],
        &[254, 60, 60, 100],
    );
    for (value, header, payload) in [
        (Unsigned(u64::MAX), &[0xFD, 0xFF, 8][..], &[0xFF; 8][..]),
        (Signed(i32::MIN), &[0xFC, 0xFF], &[0x80, 0, 0, 0]),
        (Real(0x80000000), &[0xFC, 0xFF], &[0x80, 0, 0, 0]),
        (
            Double(0x8000000000000000),
            &[0xFD, 0xFF, 8],
            &[0x80, 0, 0, 0, 0, 0, 0, 0],
        ),
        (Enumerated(0x100), &[0xFA, 0xFF], &[1, 0]),
        (Boolean(true), &[0xF9, 0xFF], &[1]),
        (Object(1023, 0x3FFFFF), &[0xFC, 0xFF], &[0xFF; 4]),
        (Date([255; 4]), &[0xFC, 0xFF], &[0xFF; 4]),
    ] {
        push(&mut cases, Form::Context(255), value, header, payload);
    }
    cases
}

pub fn all_cases() -> Vec<Case> {
    let mut cases = wire_cases();
    cases.extend(compatibility_cases());
    cases
}
