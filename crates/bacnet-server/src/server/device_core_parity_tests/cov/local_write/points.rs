//! The rest of the local-write matrix (#2446): an Output, the Inputs other
//! than the Analog Input, the Values in each access mode, and a client's
//! simulated Reliability on an Integer Value, written over WriteProperty
//! to both hosts. Each row is a local change made on both sides, compared
//! as `local` compares it, unless it says it goes over the wire.
//!
//! An object whose event detection is on starts by switching it off, so
//! the server's event pass, which the `Reporter` has yet to run (#2303),
//! has nothing to do; BO-1 starts with it off.
use super::*;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::write_property::WritePropertyRequest;

/// One row: what it is, the object, the change, how many notifications it
/// calls for and the result.
type Row = (&'static str, ObjectIdentifier, Change, usize, Outcome);

/// How an input's state is written: an Enumerated or an Unsigned.
type Kind = fn(u32) -> PropertyValue;

const OK: Outcome = Ok(());

fn denied() -> Outcome {
    Err((ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED))
}

fn range() -> Outcome {
    Err((ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE))
}

fn unsupported() -> Outcome {
    Err((
        ErrorClass::OBJECT,
        ErrorCode::OPTIONAL_FUNCTIONALITY_NOT_SUPPORTED,
    ))
}

fn input(value: PropertyValue) -> Change {
    Change::Input(value)
}

fn at(value: PropertyValue, priority: u8) -> Change {
    Change::Command(value, Some(priority))
}

fn fault(reliability: Reliability) -> Change {
    Change::Reliability(reliability)
}

fn oos(on: bool) -> Change {
    Change::OutOfService(on)
}

fn state(raw: u32) -> PropertyValue {
    PropertyValue::Enumerated(raw)
}

fn real(value: f32) -> PropertyValue {
    PropertyValue::Real(value)
}

fn signed(value: i32) -> PropertyValue {
    PropertyValue::Signed(value)
}

impl CovParity {
    /// Each of `rows` on both sides, in order.
    async fn rows(&mut self, rows: Vec<Row>) {
        for (what, object, change, notifications, expected) in rows {
            let what = format!("{object:?}: {what}");
            let result = self.local(&what, object, change, notifications).await;
            assert_eq!(result, expected, "{what}");
        }
    }

    /// WriteProperty of `value` to `property` of `object` from the
    /// subscriber to both hosts: the reply, which must match, and how many
    /// notifications followed, which must match too. The rows read back
    /// alike afterwards.
    async fn write_property(
        &mut self,
        what: &str,
        object: ObjectIdentifier,
        property: P,
        value: PropertyValue,
    ) -> (Apdu, usize) {
        let mut encoded = BytesMut::new();
        encode_property_value(&mut encoded, &value).unwrap();
        let mut body = BytesMut::new();
        WritePropertyRequest {
            object_identifier: object,
            property_identifier: property,
            property_array_index: None,
            property_value: encoded.to_vec(),
            priority: None,
        }
        .encode(&mut body)
        .unwrap();
        let service = ConfirmedServiceChoice::WRITE_PROPERTY;
        let answer = self.exchange(what, &direct(1), service, &body).await;
        self.same_rows(what, object, &[]).await;
        answer
    }

    /// `property` of `object` as the server reads it.
    async fn server_reads(&self, object: ObjectIdentifier, property: P) -> PropertyValue {
        let db = self.parity.server.database().read().await;
        let object = db.get(&object).expect("the server holds the object");
        object.read_property(property, None).unwrap()
    }
}

/// Status_Flags with `flags` (IN_ALARM, FAULT, OVERRIDDEN, OUT_OF_SERVICE).
fn flags(flags: u8) -> PropertyValue {
    PropertyValue::BitString {
        unused_bits: 4,
        data: vec![flags << 4],
    }
}

/// A Binary Output: no application value; a command stored in service and
/// out of it, with its Value_Source; a state out of range; the
/// application's Reliability in service and not out of it.
#[tokio::test(start_paused = true)]
async fn an_output_matches() {
    let mut cov = CovParity::start_local().await;
    let null = PropertyValue::Null;
    cov.rows(vec![
        ("an input", bo1(), input(state(1)), 0, unsupported()),
        ("command at 8", bo1(), at(state(1), 8), 1, OK),
        ("state 2", bo1(), at(state(2), 8), 0, range()),
        ("a fault", bo1(), fault(Reliability::NO_SENSOR), 1, OK),
        ("out of service", bo1(), oos(true), 1, OK),
        (
            "a fault, OOS",
            bo1(),
            fault(Reliability::OVER_RANGE),
            0,
            denied(),
        ),
        ("command above, at 6, OOS", bo1(), at(state(0), 6), 1, OK),
        ("back in service", bo1(), oos(false), 1, OK),
        ("relinquish 6", bo1(), at(null.clone(), 6), 1, OK),
        ("relinquish 8, to the default", bo1(), at(null, 8), 1, OK),
    ])
    .await;
    cov.stop().await;
}

/// A Binary and a Multi-state Input, as the Analog Input's rows: the
/// application's value and Reliability in service, neither out of
/// service, where a command simulates the value; a command refused in
/// service; a state out of range.
#[tokio::test(start_paused = true)]
async fn the_other_inputs_match() {
    let mut cov = CovParity::start_local().await;
    // A binary state is an Enumerated, a multi-state one an Unsigned.
    let unsigned = |raw: u32| PropertyValue::Unsigned(u64::from(raw));
    let inputs: [(ObjectIdentifier, Kind, u32, u32); 2] =
        [(bi1(), state, 1, 0), (msi1(), unsigned, 2, 3)];
    for (object, kind, value, other) in inputs {
        let (value, other) = (kind(value), kind(other));
        let (over, under) = (
            fault(Reliability::OVER_RANGE),
            fault(Reliability::NO_SENSOR),
        );
        cov.rows(vec![
            ("event detection off", object, detection(false), 0, OK),
            (
                "a command, in service",
                object,
                at(value.clone(), 8),
                0,
                denied(),
            ),
            ("an input", object, input(value), 1, OK),
            ("a fault", object, under, 1, OK),
            ("out of service", object, oos(true), 1, OK),
            ("an input, OOS", object, input(other.clone()), 0, denied()),
            ("a fault, OOS", object, over, 0, denied()),
            ("simulated", object, at(other, 8), 1, OK),
            ("back in service", object, oos(false), 1, OK),
            ("state 4", object, input(kind(4)), 0, range()),
        ])
        .await;
    }
    cov.stop().await;
}

/// The Values: a read-only one refuses a command in service and an
/// application value out of it; a writable one takes the application's
/// value; a writable Integer Value takes a command at any priority as its
/// value and a NULL as nothing; a commandable one written with no priority
/// commands slot 16; each takes the application's Reliability; a binary
/// state 2 is out of range.
#[tokio::test(start_paused = true)]
async fn the_values_match() {
    let mut cov = CovParity::start_local().await;
    let null = || Change::Command(PropertyValue::Null, None);
    let bv1 = oid(ObjectType::BINARY_VALUE, 1);
    cov.rows(vec![
        ("AV-2 event detection off", av2(), detection(false), 0, OK),
        (
            "AV-2 command, in service",
            av2(),
            at(real(1.0), 8),
            0,
            denied(),
        ),
        ("AV-2 input", av2(), input(real(3.0)), 1, OK),
        ("AV-2 fault", av2(), fault(Reliability::NO_SENSOR), 1, OK),
        ("AV-2 out of service", av2(), oos(true), 1, OK),
        ("AV-2 input, OOS", av2(), input(real(4.0)), 0, denied()),
        (
            "AV-2 fault, OOS",
            av2(),
            fault(Reliability::OVER_RANGE),
            0,
            denied(),
        ),
        ("AV-2 back in service", av2(), oos(false), 1, OK),
        ("BV-2 event detection off", bv2(), detection(false), 0, OK),
        ("BV-2 input", bv2(), input(state(1)), 1, OK),
        ("BV-2 fault", bv2(), fault(Reliability::NO_SENSOR), 1, OK),
        ("IV-3 input", iv3(), input(signed(2)), 1, OK),
        ("IV-3 command at 8", iv3(), at(signed(3), 8), 1, OK),
        ("IV-3 NULL", iv3(), null(), 0, OK),
        ("IV-3 fault", iv3(), fault(Reliability::NO_SENSOR), 1, OK),
        (
            "IV-2 no priority, slot 16",
            iv2(),
            Change::Command(signed(9), None),
            1,
            OK,
        ),
        ("IV-2 relinquish 16", iv2(), null(), 1, OK),
        ("BV-1 state 2", bv1, at(state(2), 8), 0, range()),
    ])
    .await;
    cov.stop().await;
}

/// A client's Reliability on a read-only Integer Value (Clause 12.43.9,
/// #2446), over WriteProperty: refused in service; out of service taken,
/// with Status_Flags' FAULT following it into the notifications, while
/// another datatype or a value outside BACnetReliability is refused; a
/// local write of it is taken too; and the return to service puts back
/// the application's fault.
#[tokio::test(start_paused = true)]
async fn a_clients_integer_value_reliability_matches() {
    let mut cov = CovParity::start_local().await;
    let r = P::RELIABILITY;
    let enumerated = |reliability: Reliability| PropertyValue::Enumerated(reliability.to_raw());
    let no_fault = enumerated(Reliability::NO_FAULT_DETECTED);
    let fault_row = (
        "the application's fault",
        iv1(),
        fault(Reliability::NO_SENSOR),
        1,
        OK,
    );
    cov.rows(vec![fault_row]).await;
    let (reply, sent) = cov
        .write_property("in service", iv1(), r, no_fault.clone())
        .await;
    let denied = (ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED);
    assert_eq!((refused(&reply), sent), (denied, 0));
    cov.rows(vec![("out of service", iv1(), oos(true), 1, OK)])
        .await;
    assert_eq!(
        cov.server_reads(iv1(), P::STATUS_FLAGS).await,
        flags(0b0101)
    );
    let (reply, sent) = cov
        .write_property("no fault", iv1(), r, no_fault.clone())
        .await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(sent, 1, "FAULT cleared");
    assert_eq!(
        cov.server_reads(iv1(), P::STATUS_FLAGS).await,
        flags(0b0001)
    );
    let open_loop = enumerated(Reliability::OPEN_LOOP);
    let (reply, sent) = cov.write_property("open loop", iv1(), r, open_loop).await;
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(sent, 1, "FAULT set");
    assert_eq!(
        cov.server_reads(iv1(), P::STATUS_FLAGS).await,
        flags(0b0101)
    );
    for (what, value, code) in [
        (
            "an Unsigned",
            PropertyValue::Unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            "26",
            PropertyValue::Enumerated(26),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
    ] {
        let (reply, sent) = cov.write_property(what, iv1(), r, value).await;
        assert_eq!((refused(&reply), sent), ((ErrorClass::PROPERTY, code), 0));
    }
    let local = Change::Write(r, no_fault);
    cov.rows(vec![("a local write", iv1(), local, 1, OK)]).await;
    cov.rows(vec![("back in service", iv1(), oos(false), 1, OK)])
        .await;
    let restored = cov.server_reads(iv1(), r).await;
    assert_eq!(restored, enumerated(Reliability::NO_SENSOR));
    assert_eq!(
        cov.server_reads(iv1(), P::STATUS_FLAGS).await,
        flags(0b0100)
    );
    cov.stop().await;
}
