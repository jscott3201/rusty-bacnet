//! Writes to a BBMD port's BDT and registration setting wait for
//! activation (Clause 12.56.13, 12.56.34, 12.56.35, #1578).

use super::*;
use crate::database::ObjectDatabase;
use bacnet_encoding::constructed::{decode_bdt_entry_list, encode_bdt_entry_list};
use bacnet_types::bip_port::{BbmdChanges, BbmdTables, BipPortMode};
use bacnet_types::constructed::{BACnetBDTEntry, BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bytes::BytesMut;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

/// The address [`Tables`] refuses in a BDT, as a transport's policy would.
const REFUSED: [u8; 4] = [10, 0, 0, 99];

/// Tables that apply what is activated, refuse a BDT naming [`REFUSED`],
/// and refuse every activation while `fail` is set.
#[derive(Default)]
struct Tables {
    bdt: Mutex<Vec<BACnetBDTEntry>>,
    accepts: Mutex<bool>,
    fail: Mutex<bool>,
    fail_check: Mutex<bool>,
}

fn refusal() -> Error {
    Error::Protocol {
        class: ErrorClass::PROPERTY,
        code: ErrorCode::VALUE_OUT_OF_RANGE,
    }
}

fn is_refusal(error: &Error) -> bool {
    matches!(error, Error::Protocol { class, code }
        if *class == ErrorClass::PROPERTY
            && *code == ErrorCode::VALUE_OUT_OF_RANGE)
}

impl BbmdTables for Tables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        self.bdt.lock().unwrap().clone()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        *self.accepts.lock().unwrap()
    }
    fn foreign_device_table(&self) -> Vec<bacnet_types::constructed::BACnetFDTEntry> {
        Vec::new()
    }
    fn check_broadcast_distribution_table(&self, table: &[BACnetBDTEntry]) -> Result<(), Error> {
        let refused = BACnetHostAddress::Ip(IpAddr::V4(REFUSED.into()));
        if *self.fail_check.lock().unwrap()
            || table.iter().any(|row| row.bbmd_address.host == refused)
        {
            return Err(refusal());
        }
        Ok(())
    }
    fn activate(&self, changes: &BbmdChanges) -> Result<(), Error> {
        if *self.fail.lock().unwrap() {
            return Err(refusal());
        }
        if let Some(table) = &changes.broadcast_distribution_table {
            *self.bdt.lock().unwrap() = table.clone();
        }
        if let Some(accept) = changes.accept_foreign_device_registrations {
            *self.accepts.lock().unwrap() = accept;
        }
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

fn row(ip: [u8; 4]) -> BACnetBDTEntry {
    BACnetBDTEntry {
        bbmd_address: BACnetHostNPort {
            host: BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::from(ip))),
            port: 0xBAC0,
        },
        broadcast_mask: Some([0xFF; 4]),
    }
}

/// A BBMD port over `tables`, with the lease that keeps it registered.
fn bbmd_port(tables: &Arc<Tables>) -> (ObjectDatabase, Arc<()>) {
    let mut db = ObjectDatabase::new();
    let config = BipPortConfig {
        ip_address: [127, 0, 0, 1],
        ..Default::default()
    };
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "NP", config).unwrap(),
    ))
    .unwrap();
    let (_, lease) = db
        .reserve_bip_port_internal(oid(), [127, 0, 0, 1], 47808)
        .unwrap();
    let tables: Arc<dyn BbmdTables> = tables.clone();
    let mode = BipPortMode::Bbmd {
        tables: Some(tables),
    };
    db.publish_bip_port_internal(oid(), [127, 0, 0, 1], 47808, 1476, mode)
        .unwrap();
    db.set_network_port_activation_internal(oid(), true)
        .unwrap();
    (db, lease)
}

fn read(db: &ObjectDatabase, property: P) -> PropertyValue {
    db.get(&oid())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

fn bdt_read(db: &ObjectDatabase) -> Vec<BACnetBDTEntry> {
    let PropertyValue::List(items) = read(db, P::BBMD_BROADCAST_DISTRIBUTION_TABLE) else {
        panic!("a list");
    };
    let bytes: Vec<u8> = items
        .into_iter()
        .flat_map(|item| match item {
            PropertyValue::ApplicationData(bytes) => bytes,
            other => panic!("framed entries: {other:?}"),
        })
        .collect();
    decode_bdt_entry_list(&bytes).unwrap()
}

fn write(db: &mut ObjectDatabase, property: P, value: PropertyValue) -> Result<(), Error> {
    db.get_mut(&oid())
        .unwrap()
        .write_property(property, None, value, None)
}

fn written_bdt(table: &[BACnetBDTEntry]) -> PropertyValue {
    let mut bytes = BytesMut::new();
    encode_bdt_entry_list(&mut bytes, table).unwrap();
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn changes_pending(db: &ObjectDatabase) -> bool {
    read(db, P::CHANGES_PENDING) == PropertyValue::Boolean(true)
}

/// Clause 12.56.34: while a written BDT is pending, the property reads as
/// written and the BBMD keeps the BDT in use; activation puts it into use.
#[test]
fn a_written_bdt_reads_back_and_waits_for_activation() {
    let tables = Arc::new(Tables::default());
    *tables.bdt.lock().unwrap() = vec![row([127, 0, 0, 1])];
    let (mut db, _lease) = bbmd_port(&tables);
    assert!(!changes_pending(&db));

    let table = vec![row([10, 0, 0, 2]), row([127, 0, 0, 1])];
    write(
        &mut db,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        written_bdt(&table),
    )
    .unwrap();
    write(
        &mut db,
        P::BBMD_ACCEPT_FD_REGISTRATIONS,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert!(changes_pending(&db));
    assert_eq!(bdt_read(&db), table);
    assert_eq!(
        read(&db, P::BBMD_ACCEPT_FD_REGISTRATIONS),
        PropertyValue::Boolean(true)
    );
    assert_eq!(*tables.bdt.lock().unwrap(), [row([127, 0, 0, 1])]);
    assert!(!*tables.accepts.lock().unwrap());

    assert_eq!(db.activate_network_port_changes().unwrap(), 1);
    assert!(!changes_pending(&db));
    assert_eq!(*tables.bdt.lock().unwrap(), table);
    assert!(*tables.accepts.lock().unwrap());
    assert_eq!(bdt_read(&db), table);
    // Nothing left to activate.
    assert_eq!(db.activate_network_port_changes().unwrap(), 0);
}

/// A BDT the transport's policy refuses is refused when written, and
/// changes neither the property nor Changes_Pending.
#[test]
fn a_refused_bdt_changes_nothing() {
    let tables = Arc::new(Tables::default());
    *tables.bdt.lock().unwrap() = vec![row([127, 0, 0, 1])];
    let (mut db, _lease) = bbmd_port(&tables);
    let error = write(
        &mut db,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        written_bdt(&[row(REFUSED)]),
    )
    .unwrap_err();
    assert!(is_refusal(&error), "{error:?}");
    assert!(!changes_pending(&db));
    assert_eq!(bdt_read(&db), [row([127, 0, 0, 1])]);
}

/// Values of the wrong shape are refused before anything waits.
#[test]
fn malformed_writes_are_refused() {
    let tables = Arc::new(Tables::default());
    let (mut db, _lease) = bbmd_port(&tables);
    let object = db.get_mut(&oid()).unwrap();
    for (property, index, value) in [
        (
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
            None,
            PropertyValue::Boolean(true),
        ),
        (
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
            None,
            PropertyValue::ApplicationData(vec![0x0E]),
        ),
        (
            P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
            Some(1),
            written_bdt(&[]),
        ),
        (
            P::BBMD_ACCEPT_FD_REGISTRATIONS,
            None,
            PropertyValue::Unsigned(1),
        ),
        (
            P::BBMD_ACCEPT_FD_REGISTRATIONS,
            Some(1),
            PropertyValue::Boolean(true),
        ),
    ] {
        assert!(
            object.write_property(property, index, value, None).is_err(),
            "{property} [{index:?}]"
        );
    }
    assert!(!changes_pending(&db));
}

/// When the transport refuses activation, the values stay pending.
#[test]
fn a_refused_activation_keeps_the_changes_pending() {
    let tables = Arc::new(Tables::default());
    let (mut db, _lease) = bbmd_port(&tables);
    write(
        &mut db,
        P::BBMD_ACCEPT_FD_REGISTRATIONS,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    *tables.fail.lock().unwrap() = true;
    let error = db.activate_network_port_changes().unwrap_err();
    assert!(is_refusal(&error), "{error:?}");
    assert!(changes_pending(&db));
    *tables.fail.lock().unwrap() = false;
    assert_eq!(db.activate_network_port_changes().unwrap(), 1);
    assert!(*tables.accepts.lock().unwrap());
}

/// A port published again, as by a new owner, starts with nothing pending.
#[test]
fn publishing_again_drops_pending_values() {
    let tables = Arc::new(Tables::default());
    let (mut db, lease) = bbmd_port(&tables);
    write(
        &mut db,
        P::BBMD_ACCEPT_FD_REGISTRATIONS,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    drop(lease);
    let (_, _lease) = db
        .reserve_bip_port_internal(oid(), [127, 0, 0, 1], 47808)
        .unwrap();
    assert!(!changes_pending(&db));
}

/// Without an owner that activates changes, the rows refuse writes and are
/// listed read-only: a written value could never take effect.
#[test]
fn without_activation_the_rows_refuse_writes() {
    use crate::property_metadata::PropertyWriteCapability;
    let tables = Arc::new(Tables::default());
    let (mut db, _lease) = bbmd_port(&tables);
    let capability = |db: &ObjectDatabase, property: P| {
        db.get(&oid())
            .unwrap()
            .property_metadata()
            .iter()
            .find(|row| row.property_identifier == property)
            .unwrap()
            .write_capability
    };
    for property in [
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        P::BBMD_ACCEPT_FD_REGISTRATIONS,
    ] {
        assert_eq!(capability(&db, property), PropertyWriteCapability::Always);
    }
    db.set_network_port_activation_internal(oid(), false)
        .unwrap();
    for (property, value) in [
        (P::BBMD_BROADCAST_DISTRIBUTION_TABLE, written_bdt(&[])),
        (
            P::BBMD_ACCEPT_FD_REGISTRATIONS,
            PropertyValue::Boolean(true),
        ),
    ] {
        let error = write(&mut db, property, value).unwrap_err();
        assert!(
            matches!(error, Error::Protocol { class, code }
                if class == ErrorClass::PROPERTY && code == ErrorCode::WRITE_ACCESS_DENIED),
            "{error:?}"
        );
        assert_eq!(capability(&db, property), PropertyWriteCapability::ReadOnly);
    }
    assert!(!changes_pending(&db));
}

/// The dry run before a ReinitializeDevice asks the transport again, and
/// changes nothing.
#[test]
fn the_dry_run_rechecks_a_pending_bdt() {
    let tables = Arc::new(Tables::default());
    let (mut db, _lease) = bbmd_port(&tables);
    db.check_network_port_changes().unwrap();
    write(
        &mut db,
        P::BBMD_BROADCAST_DISTRIBUTION_TABLE,
        written_bdt(&[row([10, 0, 0, 2])]),
    )
    .unwrap();
    db.check_network_port_changes().unwrap();
    // The transport's policy changed since the write.
    *tables.fail_check.lock().unwrap() = true;
    let error = db.check_network_port_changes().unwrap_err();
    assert!(is_refusal(&error), "{error:?}");
    assert!(changes_pending(&db));
    assert!(tables.bdt.lock().unwrap().is_empty());
}
