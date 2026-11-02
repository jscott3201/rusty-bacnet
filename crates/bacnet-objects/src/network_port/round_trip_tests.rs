//! On the object API a registered port takes back the value a read just
//! returned, in every mode, on every row it lets clients write (#2272).
//!
//! A BBMD's BDT reads as a list with one encoded entry per element, while
//! WriteProperty hands the object the whole list's octets at once; the
//! write takes both. The catalogue-wide check of the other objects is
//! `read_write_round_trip_tests`.

use super::*;
use crate::database::ObjectDatabase;
use bacnet_encoding::constructed::encode_bdt_entry;
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use bytes::BytesMut;
use std::sync::Arc;

use super::{foreign_pending_tests, multicast_tests, nat_pending_tests, pending_tests};

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

fn code(result: &Result<(), Error>) -> Option<ErrorCode> {
    match result {
        Err(Error::Remote(RemoteError::Error { class, code }))
            if *class == ErrorClass::PROPERTY =>
        {
            Some(*code)
        }
        _ => None,
    }
}

/// Write every writable row the value it reads, and check that it reads
/// the same afterwards. Returns the rows written; a refusal fails.
fn round_trip(db: &mut ObjectDatabase) -> Vec<P> {
    let port = db.get_mut(&oid()).unwrap();
    let rows = port.property_metadata().into_owned();
    let mut written = Vec::new();
    let mut failures = Vec::new();
    for row in rows {
        let property = row.property_identifier;
        if !port.is_writable_property(property) {
            continue;
        }
        let value = port.read_property(property, None).unwrap();
        let result = port.write_property(property, None, value.clone(), None);
        // Each port here is registered, and its owner keeps it in service:
        // the row is writable only on a port nothing has bound.
        if property == P::OUT_OF_SERVICE {
            assert_eq!(code(&result), Some(ErrorCode::WRITE_ACCESS_DENIED));
            continue;
        }
        match result {
            Ok(()) => {
                let again = port.read_property(property, None).unwrap();
                if again != value {
                    failures.push(format!("{property}: read {value:?}, then {again:?}"));
                }
                written.push(property);
            }
            Err(error) => failures.push(format!("{property} [{value:?}]: {error:?}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    written
}

#[test]
fn a_bbmd_takes_back_every_row_it_reads() {
    let tables = Arc::new(pending_tests::Tables::default());
    *tables.bdt.lock().unwrap() = vec![
        pending_tests::row([10, 0, 0, 2]),
        pending_tests::row([127, 0, 0, 1]),
    ];
    let (mut db, _lease) = pending_tests::bbmd_port(&tables);
    let written = round_trip(&mut db);
    for row in [
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        P::BBMD_ACCEPT_FD_REGISTRATIONS,
    ] {
        assert!(written.contains(&row), "{row} not written: {written:?}");
    }
    // What was read back went into use unchanged.
    let before = tables.bdt.lock().unwrap().clone();
    assert_eq!(db.activate_network_port_changes().unwrap(), 1);
    assert_eq!(*tables.bdt.lock().unwrap(), before);
}

#[test]
fn a_bbmd_behind_nat_takes_back_every_row_it_reads() {
    let tables = nat_pending_tests::Tables::new(true);
    let (mut db, _lease) = nat_pending_tests::bbmd_port(&tables, true);
    let written = round_trip(&mut db);
    for row in [
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        P::BACNET_IP_NAT_TRAVERSAL,
        P::BACNET_IP_GLOBAL_ADDRESS,
    ] {
        assert!(written.contains(&row), "{row} not written: {written:?}");
    }
}

#[test]
fn a_foreign_device_takes_back_every_row_it_reads() {
    let registration = foreign_pending_tests::Registration::new();
    let (mut db, _lease) = foreign_pending_tests::foreign_port(&registration, true);
    let written = round_trip(&mut db);
    for row in [P::FD_BBMD_ADDRESS, P::FD_SUBSCRIPTION_LIFETIME] {
        assert!(written.contains(&row), "{row} not written: {written:?}");
    }
}

#[test]
fn a_multicast_port_takes_back_every_row_it_reads() {
    let group = multicast_tests::Group::new(multicast_tests::GROUP);
    let (mut db, _lease) = multicast_tests::port(Some(&group), true);
    let written = round_trip(&mut db);
    assert!(
        written.contains(&P::BACNET_IP_MULTICAST_ADDRESS),
        "{written:?}"
    );
}

fn entry(ip: [u8; 4]) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_bdt_entry(&mut buf, &pending_tests::row(ip)).unwrap();
    PropertyValue::ApplicationData(buf.to_vec())
}

fn write_bdt(db: &mut ObjectDatabase, value: PropertyValue) -> Result<(), Error> {
    db.get_mut(&oid()).unwrap().write_property(
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        None,
        value,
        None,
    )
}

fn read_bdt(db: &ObjectDatabase) -> PropertyValue {
    db.get(&oid())
        .unwrap()
        .read_property(P::BBMD_BROADCAST_DISTRIBUTION_TABLE, None)
        .unwrap()
}

/// The BDT takes a list of encoded entries, the shape a read returns, as
/// it takes the whole list's octets: the same table either way, an empty
/// list included.
#[test]
fn a_bdt_written_in_the_read_shape_is_the_table_read() {
    let tables = Arc::new(pending_tests::Tables::default());
    let (mut db, _lease) = pending_tests::bbmd_port(&tables);
    let table = PropertyValue::List(vec![entry([10, 0, 0, 2]), entry([127, 0, 0, 1])]);
    write_bdt(&mut db, table.clone()).unwrap();
    assert_eq!(read_bdt(&db), table);

    // The same octets in one piece, as WriteProperty hands them over.
    let PropertyValue::List(entries) = &table else {
        unreachable!()
    };
    let joined: Vec<u8> = entries
        .iter()
        .flat_map(|entry| match entry {
            PropertyValue::ApplicationData(bytes) => bytes.clone(),
            _ => unreachable!(),
        })
        .collect();
    write_bdt(&mut db, PropertyValue::List(vec![])).unwrap();
    assert_eq!(read_bdt(&db), PropertyValue::List(vec![]));
    write_bdt(&mut db, PropertyValue::ApplicationData(joined)).unwrap();
    assert_eq!(read_bdt(&db), table);

    assert_eq!(db.activate_network_port_changes().unwrap(), 1);
    assert_eq!(
        *tables.bdt.lock().unwrap(),
        vec![
            pending_tests::row([10, 0, 0, 2]),
            pending_tests::row([127, 0, 0, 1])
        ]
    );
}

/// A list that isn't encoded entries is refused as before, and changes
/// nothing: another datatype in it is INVALID_DATA_TYPE, octets that don't
/// decode as entries INVALID_DATA_ENCODING.
#[test]
fn a_bdt_list_of_anything_else_is_refused() {
    let tables = Arc::new(pending_tests::Tables::default());
    let (mut db, _lease) = pending_tests::bbmd_port(&tables);
    let before = read_bdt(&db);
    let cases = [
        (
            PropertyValue::List(vec![entry([10, 0, 0, 2]), PropertyValue::Boolean(true)]),
            ErrorCode::INVALID_DATA_TYPE,
        ),
        (PropertyValue::Boolean(true), ErrorCode::INVALID_DATA_TYPE),
        (
            PropertyValue::List(vec![
                entry([10, 0, 0, 2]),
                PropertyValue::ApplicationData(vec![0x0E]),
            ]),
            ErrorCode::INVALID_DATA_ENCODING,
        ),
    ];
    for (value, expected) in cases {
        let result = write_bdt(&mut db, value.clone());
        assert_eq!(code(&result), Some(expected), "{value:?}");
        assert_eq!(read_bdt(&db), before);
        assert_eq!(
            db.get(&oid())
                .unwrap()
                .read_property(P::CHANGES_PENDING, None)
                .unwrap(),
            PropertyValue::Boolean(false)
        );
    }
}
