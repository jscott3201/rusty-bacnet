//! A peer's write of UTC_Offset or Daylight_Savings_Status through the
//! `Responder` (#2395): refused as read-only without the clock's opt-in;
//! with it, the write moves the responder's own clock as `set_zone` does,
//! keeps its synchronization, names the Device among the objects the request
//! changed, and moves the recipient windows with local time.
use super::{date, time};
use crate::clock::{Basis, DeviceClock, Zone};
use crate::responder::{Inbound, Limits, Outcome, Responder, SERVICES};
use bacnet_encoding::apdu::{Apdu, ConfirmedRequest, decode_apdu, encode_apdu};
use bacnet_encoding::primitives::encode_property_value;
use bacnet_objects::DeviceIdentity;
use bacnet_objects::database::ObjectDatabase;
use bacnet_services::common::BACnetPropertyValue;
use bacnet_services::wpm::{WriteAccessSpecification, WritePropertyMultipleRequest};
use bacnet_services::write_property::WritePropertyRequest;
use bacnet_time::Instant;
use bacnet_types::bitstring::DaysOfWeek;
use bacnet_types::enums::{
    ConfirmedServiceChoice, ErrorClass, ErrorCode, ObjectType, PropertyIdentifier as P,
};
use bacnet_types::primitives::{ObjectIdentifier, PropertyValue, Time};
use bytes::{Bytes, BytesMut};
use std::sync::Arc;

const DEVICE: u32 = 77;

fn device() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::DEVICE, DEVICE).unwrap()
}

/// A responder whose clock, five hours west, a host set to 13:15 UTC on
/// Thursday 2024-07-04 (08:15 local), taking a peer's zone writes when
/// `writable`.
fn responder(writable: bool) -> (Responder, ObjectDatabase) {
    let identity = DeviceIdentity::new(DEVICE, 0x105)
        .unwrap()
        .with_segmentation(bacnet_types::enums::Segmentation::NO_SEGMENTATION)
        .with_services(SERVICES);
    let mut db = identity.build_database().unwrap();
    let clock = DeviceClock::new(
        Zone::new(300, false).unwrap(),
        Arc::new(|| Instant::from_secs(1_000)),
    );
    let clock = if writable {
        clock.with_writable_zone()
    } else {
        clock
    };
    let mut responder = Responder::new(identity, Limits::default())
        .unwrap()
        .with_clock(clock, &mut db);
    responder
        .set_time(&mut db, Basis::Utc, date(2024, 7, 4, 4), time(13, 15, 0, 0))
        .unwrap();
    (responder, db)
}

fn confirmed(service: ConfirmedServiceChoice, body: BytesMut) -> Bytes {
    let mut apdu = BytesMut::new();
    encode_apdu(
        &mut apdu,
        &Apdu::ConfirmedRequest(ConfirmedRequest {
            segmented: false,
            more_follows: false,
            segmented_response_accepted: false,
            max_segments: None,
            max_apdu_length: 1476,
            invoke_id: 3,
            sequence_number: None,
            proposed_window_size: None,
            service_choice: service,
            service_request: body.freeze(),
        }),
    )
    .unwrap();
    apdu.freeze()
}

fn octets(value: &PropertyValue) -> Vec<u8> {
    let mut encoded = BytesMut::new();
    encode_property_value(&mut encoded, value).unwrap();
    encoded.to_vec()
}

fn write_property(property: P, value: PropertyValue) -> Bytes {
    let mut body = BytesMut::new();
    WritePropertyRequest {
        object_identifier: device(),
        property_identifier: property,
        property_array_index: None,
        property_value: octets(&value),
        priority: None,
    }
    .encode(&mut body)
    .unwrap();
    confirmed(ConfirmedServiceChoice::WRITE_PROPERTY, body)
}

/// `apdu` through the responder: its reply and the objects it changed.
fn handle(
    responder: &mut Responder,
    db: &mut ObjectDatabase,
    apdu: Bytes,
) -> (Apdu, Vec<ObjectIdentifier>) {
    let mut out = BytesMut::new();
    let written = responder.handle(db, Inbound::new(apdu, 1476), &mut out);
    assert_eq!(written.outcome, Outcome::Reply);
    (decode_apdu(out.freeze()).unwrap(), written.objects)
}

fn refused(reply: &Apdu, class: ErrorClass, code: ErrorCode) -> bool {
    matches!(reply, Apdu::Error(error) if error.error_class == class && error.error_code == code)
}

/// The Device's local hour and minute, UTC_Offset and daylight saving, as
/// the database serves them, which the responder's own clock must match.
fn reading(responder: &Responder, db: &ObjectDatabase) -> (u8, u8, i16, bool) {
    let frame = db.clock_frame().unwrap();
    let own = responder.clock().unwrap();
    assert_eq!(own.clock().frame(own.now()), Some(frame));
    (
        frame.local_time.hour,
        frame.local_time.minute,
        frame.utc_offset,
        frame.daylight_savings_status,
    )
}

#[test]
fn without_the_opt_in_both_stay_read_only() {
    let (mut responder, mut db) = responder(false);
    for (property, value) in [
        (P::UTC_OFFSET, PropertyValue::Signed(360)),
        (P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true)),
    ] {
        let (reply, objects) = handle(&mut responder, &mut db, write_property(property, value));
        assert!(
            refused(&reply, ErrorClass::PROPERTY, ErrorCode::WRITE_ACCESS_DENIED),
            "{reply:?}"
        );
        assert!(objects.is_empty());
    }
    assert_eq!(reading(&responder, &db), (8, 15, 300, false));
}

/// Daylight saving written after a synchronization moves local time an
/// hour, and the clock stays synchronized; an offset moves it by the
/// difference. Each names the Device for the host to save the zone.
#[test]
fn a_written_zone_moves_the_responders_clock_and_keeps_its_synchronization() {
    let (mut responder, mut db) = responder(true);
    let apdu = write_property(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true));
    let (reply, objects) = handle(&mut responder, &mut db, apdu);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(objects, [device()]);
    assert_eq!(reading(&responder, &db), (9, 15, 300, true));
    let own = responder.clock().unwrap();
    assert!(own.clock().is_synced());
    assert_eq!(own.clock().zone(), Zone::new(300, true).unwrap());

    let apdu = write_property(P::UTC_OFFSET, PropertyValue::Signed(360));
    let (reply, objects) = handle(&mut responder, &mut db, apdu);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(objects, [device()]);
    assert_eq!(reading(&responder, &db), (8, 15, 360, true));
    assert!(responder.clock().unwrap().clock().is_synced());
}

/// A value off the grid, past a day or of another datatype is refused and
/// changes nothing; a NULL succeeds and changes nothing.
#[test]
fn a_bad_zone_write_is_refused_and_changes_nothing() {
    let (mut responder, mut db) = responder(true);
    for (property, value, code) in [
        (
            P::UTC_OFFSET,
            PropertyValue::Signed(10),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::UTC_OFFSET,
            PropertyValue::Signed(1455),
            ErrorCode::VALUE_OUT_OF_RANGE,
        ),
        (
            P::UTC_OFFSET,
            PropertyValue::Real(60.0),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (
            P::DAYLIGHT_SAVINGS_STATUS,
            PropertyValue::Unsigned(1),
            ErrorCode::INVALID_DATA_TYPE,
        ),
    ] {
        let (reply, objects) = handle(&mut responder, &mut db, write_property(property, value));
        assert!(refused(&reply, ErrorClass::PROPERTY, code), "{reply:?}");
        assert!(objects.is_empty());
    }
    for property in [P::UTC_OFFSET, P::DAYLIGHT_SAVINGS_STATUS] {
        let (reply, objects) = handle(
            &mut responder,
            &mut db,
            write_property(property, PropertyValue::Null),
        );
        assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
        assert!(objects.is_empty());
    }
    assert_eq!(reading(&responder, &db), (8, 15, 300, false));
}

/// A WritePropertyMultiple of both: the second write sees the first, so the
/// clock ends in the pair the request wrote.
#[test]
fn a_write_property_multiple_writes_both_halves() {
    let (mut responder, mut db) = responder(true);
    let attempt = |property, value: PropertyValue| BACnetPropertyValue {
        property_identifier: property,
        property_array_index: None,
        value: octets(&value),
        priority: None,
    };
    let mut body = BytesMut::new();
    WritePropertyMultipleRequest {
        list_of_write_access_specs: vec![WriteAccessSpecification {
            object_identifier: device(),
            list_of_properties: vec![
                attempt(P::UTC_OFFSET, PropertyValue::Signed(-60)),
                attempt(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true)),
            ],
        }],
    }
    .encode(&mut body)
    .unwrap();
    let apdu = confirmed(ConfirmedServiceChoice::WRITE_PROPERTY_MULTIPLE, body);
    let (reply, objects) = handle(&mut responder, &mut db, apdu);
    assert!(matches!(reply, Apdu::SimpleAck(_)), "{reply:?}");
    assert_eq!(objects, [device()]);
    assert_eq!(reading(&responder, &db), (15, 15, -60, true));
}

/// The recipient windows judge the written local time: 08:15 on a
/// Thursday, then 09:15 once daylight saving is written.
#[test]
fn the_recipient_window_follows_a_written_zone() {
    let (mut responder, mut db) = responder(true);
    let window = |responder: &Responder| {
        let own = responder.clock().unwrap();
        own.clock().window(own.now())
    };
    let thursday = DaysOfWeek::THURSDAY;
    let at = |hour| Time {
        hour,
        minute: 15,
        second: 0,
        hundredths: 0,
    };
    assert_eq!(window(&responder), Some((thursday, at(8))));
    let apdu = write_property(P::DAYLIGHT_SAVINGS_STATUS, PropertyValue::Boolean(true));
    handle(&mut responder, &mut db, apdu);
    assert_eq!(window(&responder), Some((thursday, at(9))));
}

/// A local write of the zone through the `Reporter` moves the clock at
/// once (#2395): its window and a local synchronization read the new zone,
/// and the host's later `set_zone`, newer, holds against the next peer
/// write, which can't bring the local write's zone back.
#[test]
fn a_local_write_moves_the_clock_at_once_and_the_hosts_zone_holds() {
    use crate::cov::CovPolicy;
    use crate::report::{ReportConfig, Reporter};
    use crate::write::local::{LocalChange, LocalCommandSource};

    let (responder, mut db) = responder(true);
    let mut reporter = Reporter::new(responder, ReportConfig::new(CovPolicy::default())).unwrap();
    let change = LocalChange::Property {
        property: P::DAYLIGHT_SAVINGS_STATUS,
        array_index: None,
        value: PropertyValue::Boolean(true),
        priority: None,
        source: LocalCommandSource::ServerDevice,
    };
    reporter
        .apply_local(&mut db, device(), change, Instant::from_secs(1_000))
        .unwrap();
    let own = reporter.clock().unwrap();
    assert_eq!(own.clock().zone(), Zone::new(300, true).unwrap());
    let at = |hour| Time {
        hour,
        minute: 15,
        second: 0,
        hundredths: 0,
    };
    assert_eq!(
        own.clock().window(own.now()),
        Some((DaysOfWeek::THURSDAY, at(9)))
    );

    // The host's zone is newer; an unrelated peer write after it leaves it.
    reporter
        .set_zone(&mut db, Zone::new(0, false).unwrap())
        .unwrap();
    let mut out = BytesMut::new();
    let description = write_property(P::DESCRIPTION, PropertyValue::CharacterString("x".into()));
    let peer = crate::cov::SubscriberEndpoint::new(&[7], None);
    let request = Inbound::new(description, 1476);
    reporter.handle(&mut db, &peer, request, Instant::from_secs(1_000), &mut out);
    let frame = db.clock_frame().unwrap();
    assert_eq!(
        (frame.utc_offset, frame.daylight_savings_status),
        (0, false)
    );
    assert_eq!(
        reporter.clock().unwrap().clock().zone(),
        Zone::new(0, false).unwrap()
    );
}

/// A peer's write, then the host's `set_zone` before the responder runs
/// again: the host's zone holds (it is newer), and no stale written zone
/// is applied later.
#[test]
fn the_hosts_zone_drops_a_written_zone_not_yet_taken() {
    let (mut responder, mut db) = responder(true);
    // A write made on the database directly, as a host's own local write
    // through `write::local::apply`, leaves the zone for the responder.
    db.get_mut(&device())
        .unwrap()
        .write_property(P::UTC_OFFSET, None, PropertyValue::Signed(-60), None)
        .unwrap();
    assert_eq!(db.clock_frame().unwrap().utc_offset, -60);
    responder
        .set_zone(&mut db, Zone::new(0, true).unwrap())
        .unwrap();
    let apdu = write_property(P::DESCRIPTION, PropertyValue::CharacterString("x".into()));
    handle(&mut responder, &mut db, apdu);
    assert_eq!(reading(&responder, &db), (14, 15, 0, true));
}
