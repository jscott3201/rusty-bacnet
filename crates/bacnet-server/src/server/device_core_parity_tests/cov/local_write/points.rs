//! The rest of the local-write matrix (#2446): an Output, the Inputs other
//! than the Analog Input and the Values in each access mode, from `cases`,
//! and a client's simulated Reliability on an Integer Value, written over
//! WriteProperty to both hosts, which only these two hosts take.
use super::cases::{fault, iv1, oos};
use super::*;
use bacnet_encoding::primitives::encode_property_value;
use bacnet_services::write_property::WritePropertyRequest;

/// One row: what it is, the object, the change, how many notifications it
/// calls for and the result.
type Row = (&'static str, ObjectIdentifier, Change, usize, Outcome);

const OK: Outcome = Ok(());

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

#[tokio::test(start_paused = true)]
async fn an_output_matches() {
    CovParity::run_local(cases::an_output()).await;
}

#[tokio::test(start_paused = true)]
async fn the_other_inputs_match() {
    CovParity::run_local(cases::the_other_inputs()).await;
}

#[tokio::test(start_paused = true)]
async fn the_values_match() {
    CovParity::run_local(cases::the_values()).await;
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
