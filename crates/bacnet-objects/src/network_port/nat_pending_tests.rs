//! BACnet_IP_NAT_Traversal and BACnet_IP_Global_Address on an IPv4 BBMD
//! port whose transport can run behind a NAT router (Clause 12.56.32,
//! 12.56.33, Table 12-71 footnotes 9 and 10, #940): present only there,
//! read live, and written values waiting for activation, checked with the
//! other values waiting.

use super::*;
use crate::database::ObjectDatabase;
use crate::property_metadata::PropertyWriteCapability;
use bacnet_encoding::constructed::{decode_host_n_port, encode_host_n_port};
use bacnet_types::bip_port::{BbmdChanges, BbmdTables, BipPortMode};
use bacnet_types::constructed::{
    BACnetBDTEntry, BACnetFDTEntry, BACnetHostAddress, BACnetHostNPort,
};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use bytes::BytesMut;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

const UNSET: BACnetHostNPort = BACnetHostNPort {
    host: BACnetHostAddress::None,
    port: 0,
};

fn global() -> BACnetHostNPort {
    BACnetHostNPort {
        host: BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1))),
        port: 47809,
    }
}

/// A BBMD that can run behind NAT, as the transport's live view would
/// answer: NAT needs a global address, and only an IPv4 host or the unset
/// sentinel is a global address.
struct Tables {
    nat: Mutex<bool>,
    global: Mutex<BACnetHostNPort>,
    /// Whether it can run behind NAT at all.
    capable: bool,
    activated: Mutex<Vec<BbmdChanges>>,
}

impl Tables {
    fn new(capable: bool) -> Arc<Self> {
        Arc::new(Self {
            nat: Mutex::new(false),
            global: Mutex::new(UNSET),
            capable,
            activated: Mutex::default(),
        })
    }
}

fn refusal() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
}

impl BbmdTables for Tables {
    fn broadcast_distribution_table(&self) -> Vec<BACnetBDTEntry> {
        Vec::new()
    }
    fn accepts_foreign_device_registrations(&self) -> bool {
        false
    }
    fn foreign_device_table(&self) -> Vec<BACnetFDTEntry> {
        Vec::new()
    }
    fn check_broadcast_distribution_table(&self, _: &[BACnetBDTEntry]) -> Result<(), Error> {
        Ok(())
    }
    fn nat_traversal(&self) -> Option<bool> {
        self.capable.then(|| *self.nat.lock().unwrap())
    }
    fn global_address(&self) -> Option<BACnetHostNPort> {
        self.capable.then(|| self.global.lock().unwrap().clone())
    }
    /// A value alone: only an IPv4 host (or the unset sentinel) is a
    /// global address.
    fn check_value(&self, changes: &BbmdChanges) -> Result<(), Error> {
        match changes.global_address.as_ref().map(|g| (&g.host, g.port)) {
            None | Some((BACnetHostAddress::None, 0)) => Ok(()),
            Some((BACnetHostAddress::Ip(IpAddr::V4(_)), port)) if port != 0 => Ok(()),
            Some(_) => Err(refusal()),
        }
    }
    fn check(&self, changes: &BbmdChanges) -> Result<(), Error> {
        let global = changes
            .global_address
            .clone()
            .unwrap_or_else(|| self.global.lock().unwrap().clone());
        let usable = match (&global.host, global.port) {
            (BACnetHostAddress::None, 0) => false,
            (BACnetHostAddress::Ip(IpAddr::V4(_)), port) if port != 0 => true,
            _ => return Err(refusal()),
        };
        let nat = changes.nat_traversal.unwrap_or(*self.nat.lock().unwrap());
        if nat && !usable {
            return Err(refusal());
        }
        Ok(())
    }
    fn activate(&self, changes: &BbmdChanges) -> Result<(), Error> {
        self.check(changes)?;
        if let Some(nat) = changes.nat_traversal {
            *self.nat.lock().unwrap() = nat;
        }
        if let Some(global) = &changes.global_address {
            *self.global.lock().unwrap() = global.clone();
        }
        self.activated.lock().unwrap().push(changes.clone());
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

/// An IPv4 port in BBMD mode over `tables`, activating changes when
/// `activation` says so.
fn bbmd_port(tables: &Arc<Tables>, activation: bool) -> (ObjectDatabase, Arc<()>) {
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
    db.set_network_port_activation_internal(oid(), activation)
        .unwrap();
    (db, lease)
}

fn read(db: &ObjectDatabase, property: P) -> Result<PropertyValue, Error> {
    db.get(&oid()).unwrap().read_property(property, None)
}

fn global_read(db: &ObjectDatabase) -> BACnetHostNPort {
    let Ok(PropertyValue::ApplicationData(octets)) = read(db, P::BACNET_IP_GLOBAL_ADDRESS) else {
        panic!("an encoded BACnetHostNPort");
    };
    let (address, end) = decode_host_n_port(&octets, 0).unwrap();
    assert_eq!(end, octets.len());
    address
}

fn written(address: &BACnetHostNPort) -> PropertyValue {
    let mut buf = BytesMut::new();
    encode_host_n_port(&mut buf, address).unwrap();
    PropertyValue::ApplicationData(buf.to_vec())
}

fn write(db: &mut ObjectDatabase, property: P, value: PropertyValue) -> Result<(), Error> {
    db.get_mut(&oid())
        .unwrap()
        .write_property(property, None, value, None)
}

fn listed(db: &ObjectDatabase) -> Vec<P> {
    db.get(&oid()).unwrap().property_list().to_vec()
}

fn is_unknown(result: Result<PropertyValue, Error>) -> bool {
    matches!(result, Err(Error::Remote(RemoteError::Error { code, .. })) if code == ErrorCode::UNKNOWN_PROPERTY)
}

#[test]
fn a_nat_capable_ipv4_bbmd_serves_both_rows() {
    let tables = Tables::new(true);
    let (db, _lease) = bbmd_port(&tables, true);
    let list = listed(&db);
    assert!(list.contains(&P::BACNET_IP_NAT_TRAVERSAL));
    assert!(list.contains(&P::BACNET_IP_GLOBAL_ADDRESS));
    assert_eq!(
        read(&db, P::BACNET_IP_NAT_TRAVERSAL).unwrap(),
        PropertyValue::Boolean(false)
    );
    // Unset until configured (Clause 12.56.33).
    assert_eq!(global_read(&db), UNSET);
    // Read live.
    *tables.nat.lock().unwrap() = true;
    *tables.global.lock().unwrap() = global();
    assert_eq!(
        read(&db, P::BACNET_IP_NAT_TRAVERSAL).unwrap(),
        PropertyValue::Boolean(true)
    );
    assert_eq!(global_read(&db), global());
}

#[test]
fn a_bbmd_that_cant_run_behind_nat_has_neither_row() {
    let tables = Tables::new(false);
    let (mut db, _lease) = bbmd_port(&tables, true);
    let list = listed(&db);
    assert!(!list.contains(&P::BACNET_IP_NAT_TRAVERSAL));
    assert!(!list.contains(&P::BACNET_IP_GLOBAL_ADDRESS));
    assert!(is_unknown(read(&db, P::BACNET_IP_NAT_TRAVERSAL)));
    assert!(is_unknown(read(&db, P::BACNET_IP_GLOBAL_ADDRESS)));
    let error = write(
        &mut db,
        P::BACNET_IP_NAT_TRAVERSAL,
        PropertyValue::Boolean(true),
    );
    assert!(
        matches!(error, Err(Error::Remote(RemoteError::Error { code, .. })) if code == ErrorCode::UNKNOWN_PROPERTY),
        "{error:?}"
    );
}

#[test]
fn a_normal_port_has_neither_row() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(1, "NP", BipPortConfig::default()).unwrap(),
    ))
    .unwrap();
    assert!(!listed(&db).contains(&P::BACNET_IP_NAT_TRAVERSAL));
    assert!(is_unknown(read(&db, P::BACNET_IP_GLOBAL_ADDRESS)));
}

/// Clause 12.56.32 and 12.56.33: a write sets Changes_Pending, reads back
/// as written while the BBMD keeps its setting, and takes effect on
/// activation.
#[test]
fn written_nat_rows_read_back_and_wait_for_activation() {
    let tables = Tables::new(true);
    let (mut db, _lease) = bbmd_port(&tables, true);
    for row in [P::BACNET_IP_NAT_TRAVERSAL, P::BACNET_IP_GLOBAL_ADDRESS] {
        let capability = db
            .get(&oid())
            .unwrap()
            .property_metadata()
            .iter()
            .find(|meta| meta.property_identifier == row)
            .unwrap()
            .write_capability;
        assert_eq!(capability, PropertyWriteCapability::Always, "{row}");
    }
    // Any order: NAT first is taken, and with no global address yet the
    // pair fails only the check activation makes.
    write(
        &mut db,
        P::BACNET_IP_NAT_TRAVERSAL,
        PropertyValue::Boolean(true),
    )
    .unwrap();
    assert!(db.check_network_port_changes().is_err());
    write(&mut db, P::BACNET_IP_GLOBAL_ADDRESS, written(&global())).unwrap();
    assert_eq!(
        read(&db, P::CHANGES_PENDING).unwrap(),
        PropertyValue::Boolean(true)
    );
    assert_eq!(global_read(&db), global());
    assert_eq!(
        read(&db, P::BACNET_IP_NAT_TRAVERSAL).unwrap(),
        PropertyValue::Boolean(true)
    );
    assert!(!*tables.nat.lock().unwrap(), "not yet in effect");
    assert_eq!(*tables.global.lock().unwrap(), UNSET);

    db.check_network_port_changes().unwrap();
    assert_eq!(db.activate_network_port_changes().unwrap(), 1);
    assert!(*tables.nat.lock().unwrap());
    assert_eq!(*tables.global.lock().unwrap(), global());
    // Behind NAT the address is required (Table 12-71, footnote 10).
    let conformance = db
        .get(&oid())
        .unwrap()
        .property_metadata()
        .iter()
        .find(|meta| meta.property_identifier == P::BACNET_IP_GLOBAL_ADDRESS)
        .unwrap()
        .conformance;
    assert_eq!(
        conformance,
        crate::property_metadata::PropertyConformance::RequiredRead
    );
    let activated = tables.activated.lock().unwrap();
    assert_eq!(activated.len(), 1, "both in one activation");
    assert_eq!(activated[0].nat_traversal, Some(true));
    drop(activated);
    assert_eq!(
        read(&db, P::CHANGES_PENDING).unwrap(),
        PropertyValue::Boolean(false)
    );
}

#[test]
fn a_global_address_the_bbmd_cant_take_is_refused_when_written() {
    let tables = Tables::new(true);
    let (mut db, _lease) = bbmd_port(&tables, true);
    let name = BACnetHostNPort {
        host: BACnetHostAddress::Name("nat.example".into()),
        port: 47809,
    };
    assert!(write(&mut db, P::BACNET_IP_GLOBAL_ADDRESS, written(&name)).is_err());
    for wrong in [
        PropertyValue::Boolean(true),
        PropertyValue::ApplicationData(vec![0x0E]),
    ] {
        assert!(write(&mut db, P::BACNET_IP_GLOBAL_ADDRESS, wrong).is_err());
    }
    assert!(
        write(
            &mut db,
            P::BACNET_IP_NAT_TRAVERSAL,
            PropertyValue::Unsigned(1)
        )
        .is_err()
    );
    assert_eq!(
        read(&db, P::CHANGES_PENDING).unwrap(),
        PropertyValue::Boolean(false)
    );
}

/// Without an owner that activates changes the rows are read-only.
#[test]
fn without_activation_the_nat_rows_are_read_only() {
    let tables = Tables::new(true);
    let (mut db, _lease) = bbmd_port(&tables, false);
    let error = write(&mut db, P::BACNET_IP_GLOBAL_ADDRESS, written(&global())).unwrap_err();
    assert!(
        matches!(error, Error::Remote(RemoteError::Error { code, .. }) if code == ErrorCode::WRITE_ACCESS_DENIED),
        "{error:?}"
    );
    let capability = db
        .get(&oid())
        .unwrap()
        .property_metadata()
        .iter()
        .find(|meta| meta.property_identifier == P::BACNET_IP_NAT_TRAVERSAL)
        .unwrap()
        .write_capability;
    assert_eq!(capability, PropertyWriteCapability::ReadOnly);
}
