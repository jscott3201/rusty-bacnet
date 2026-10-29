//! Writes to a foreign device's FD_BBMD_Address and FD_Subscription_Lifetime
//! wait for activation (Clause 12.56.13, 12.56.37, 12.56.38, #1825).

use super::*;
use crate::database::ObjectDatabase;
use crate::property_metadata::PropertyWriteCapability;
use bacnet_encoding::constructed::{decode_host_n_port, encode_host_n_port};
use bacnet_types::bip_port::{BipPortMode, ForeignDeviceChanges, ForeignRegistration};
use bacnet_types::constructed::{BACnetHostAddress, BACnetHostNPort};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use bytes::BytesMut;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::{Arc, Mutex};

/// A BBMD address [`Registration`] refuses, as a transport would one at its
/// own address.
const REFUSED: [u8; 4] = [10, 0, 0, 99];

/// A registration that applies what is activated, refuses [`REFUSED`] and a
/// zero time-to-live, records each check, and refuses every activation
/// while `fail` is set.
struct Registration {
    bbmd: Mutex<BACnetHostNPort>,
    ttl: Mutex<u16>,
    checked: Mutex<Vec<ForeignDeviceChanges>>,
    fail: Mutex<bool>,
}

impl Registration {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            bbmd: Mutex::new(host([10, 0, 0, 1])),
            ttl: Mutex::new(300),
            checked: Mutex::new(Vec::new()),
            fail: Mutex::new(false),
        })
    }
}

fn refusal() -> Error {
    Error::protocol(ErrorClass::PROPERTY, ErrorCode::VALUE_OUT_OF_RANGE)
}

fn is(error: &Error, expected: ErrorCode) -> bool {
    matches!(error, Error::Remote(RemoteError::Error { class, code })
        if *class == ErrorClass::PROPERTY && *code == expected)
}

impl ForeignRegistration for Registration {
    fn bbmd(&self) -> BACnetHostNPort {
        self.bbmd.lock().unwrap().clone()
    }
    fn subscription_lifetime(&self) -> u16 {
        *self.ttl.lock().unwrap()
    }
    fn check(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        self.checked.lock().unwrap().push(changes.clone());
        let refused = changes
            .bbmd
            .as_ref()
            .is_some_and(|bbmd| bbmd.host == BACnetHostAddress::Ip(IpAddr::V4(REFUSED.into())));
        if refused || changes.subscription_lifetime == Some(0) {
            return Err(refusal());
        }
        Ok(())
    }
    fn activate(&self, changes: &ForeignDeviceChanges) -> Result<(), Error> {
        if *self.fail.lock().unwrap() {
            return Err(refusal());
        }
        self.check(changes)?;
        if let Some(bbmd) = &changes.bbmd {
            *self.bbmd.lock().unwrap() = bbmd.clone();
        }
        if let Some(ttl) = changes.subscription_lifetime {
            *self.ttl.lock().unwrap() = ttl;
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

fn host(ip: [u8; 4]) -> BACnetHostNPort {
    BACnetHostNPort {
        host: BACnetHostAddress::Ip(IpAddr::V4(Ipv4Addr::from(ip))),
        port: 0xBAC0,
    }
}

/// A foreign-device port over `registration`, with the lease that keeps it
/// registered, activating changes when `activation` is set.
fn foreign_port(registration: &Arc<Registration>, activation: bool) -> (ObjectDatabase, Arc<()>) {
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
    let registration: Arc<dyn ForeignRegistration> = registration.clone();
    let mode = BipPortMode::Foreign { registration };
    db.publish_bip_port_internal(oid(), [127, 0, 0, 1], 47808, 1476, mode)
        .unwrap();
    db.set_network_port_activation_internal(oid(), activation)
        .unwrap();
    (db, lease)
}

fn read(db: &ObjectDatabase, property: P) -> PropertyValue {
    db.get(&oid())
        .unwrap()
        .read_property(property, None)
        .unwrap()
}

fn bbmd_read(db: &ObjectDatabase) -> BACnetHostNPort {
    let PropertyValue::ApplicationData(bytes) = read(db, P::FD_BBMD_ADDRESS) else {
        panic!("an encoded BACnetHostNPort");
    };
    decode_host_n_port(&bytes, 0).unwrap().0
}

fn write(db: &mut ObjectDatabase, property: P, value: PropertyValue) -> Result<(), Error> {
    db.get_mut(&oid())
        .unwrap()
        .write_property(property, None, value, None)
}

fn written(address: &BACnetHostNPort) -> PropertyValue {
    let mut bytes = BytesMut::new();
    encode_host_n_port(&mut bytes, address).unwrap();
    PropertyValue::ApplicationData(bytes.to_vec())
}

fn pending(db: &ObjectDatabase) -> bool {
    read(db, P::CHANGES_PENDING) == PropertyValue::Boolean(true)
}

#[test]
fn written_values_read_back_while_the_registration_keeps_the_old_ones() {
    let registration = Registration::new();
    let (mut db, _lease) = foreign_port(&registration, true);
    assert!(!pending(&db));
    write(&mut db, P::FD_BBMD_ADDRESS, written(&host([10, 0, 1, 1]))).unwrap();
    write(
        &mut db,
        P::FD_SUBSCRIPTION_LIFETIME,
        PropertyValue::Unsigned(120),
    )
    .unwrap();
    assert!(pending(&db));
    assert_eq!(bbmd_read(&db), host([10, 0, 1, 1]));
    assert_eq!(
        read(&db, P::FD_SUBSCRIPTION_LIFETIME),
        PropertyValue::Unsigned(120)
    );
    // The transport still registers where it did.
    assert_eq!(registration.bbmd(), host([10, 0, 0, 1]));
    assert_eq!(registration.subscription_lifetime(), 300);
    // The second write was checked together with the first.
    let last = registration
        .checked
        .lock()
        .unwrap()
        .last()
        .cloned()
        .unwrap();
    assert_eq!(last.bbmd, Some(host([10, 0, 1, 1])));
    assert_eq!(last.subscription_lifetime, Some(120));

    assert_eq!(db.activate_network_port_changes().unwrap(), 1);
    assert!(!pending(&db));
    assert_eq!(registration.bbmd(), host([10, 0, 1, 1]));
    assert_eq!(registration.subscription_lifetime(), 120);
    assert_eq!(bbmd_read(&db), host([10, 0, 1, 1]));
}

#[test]
fn a_refused_value_changes_neither_the_property_nor_the_registration() {
    let registration = Registration::new();
    let (mut db, _lease) = foreign_port(&registration, true);
    let error = write(&mut db, P::FD_BBMD_ADDRESS, written(&host(REFUSED))).unwrap_err();
    assert!(is(&error, ErrorCode::VALUE_OUT_OF_RANGE), "{error:?}");
    let error = write(
        &mut db,
        P::FD_SUBSCRIPTION_LIFETIME,
        PropertyValue::Unsigned(0),
    )
    .unwrap_err();
    assert!(is(&error, ErrorCode::VALUE_OUT_OF_RANGE), "{error:?}");
    // Unsigned16.
    let error = write(
        &mut db,
        P::FD_SUBSCRIPTION_LIFETIME,
        PropertyValue::Unsigned(65_536),
    )
    .unwrap_err();
    assert!(is(&error, ErrorCode::VALUE_OUT_OF_RANGE), "{error:?}");
    let error = write(
        &mut db,
        P::FD_SUBSCRIPTION_LIFETIME,
        PropertyValue::Boolean(true),
    )
    .unwrap_err();
    assert!(is(&error, ErrorCode::INVALID_DATA_TYPE), "{error:?}");
    let error = write(
        &mut db,
        P::FD_BBMD_ADDRESS,
        PropertyValue::ApplicationData(vec![0x0E, 0x1C]),
    )
    .unwrap_err();
    assert!(is(&error, ErrorCode::INVALID_DATA_ENCODING), "{error:?}");
    let error = db
        .get_mut(&oid())
        .unwrap()
        .write_property(
            P::FD_SUBSCRIPTION_LIFETIME,
            Some(1),
            PropertyValue::Unsigned(60),
            None,
        )
        .unwrap_err();
    assert!(is(&error, ErrorCode::PROPERTY_IS_NOT_AN_ARRAY), "{error:?}");
    assert!(!pending(&db));
    assert_eq!(bbmd_read(&db), host([10, 0, 0, 1]));
    assert_eq!(
        read(&db, P::FD_SUBSCRIPTION_LIFETIME),
        PropertyValue::Unsigned(300)
    );
    assert_eq!(registration.bbmd(), host([10, 0, 0, 1]));
    assert_eq!(registration.subscription_lifetime(), 300);
}

#[test]
fn a_refused_activation_keeps_the_values_pending() {
    let registration = Registration::new();
    let (mut db, _lease) = foreign_port(&registration, true);
    write(
        &mut db,
        P::FD_SUBSCRIPTION_LIFETIME,
        PropertyValue::Unsigned(90),
    )
    .unwrap();
    db.check_network_port_changes().unwrap();
    *registration.fail.lock().unwrap() = true;
    assert!(db.activate_network_port_changes().is_err());
    assert!(pending(&db));
    assert_eq!(
        read(&db, P::FD_SUBSCRIPTION_LIFETIME),
        PropertyValue::Unsigned(90)
    );
    assert_eq!(registration.subscription_lifetime(), 300);
}

#[test]
fn without_an_activation_owner_the_rows_are_read_only() {
    let registration = Registration::new();
    let (mut db, _lease) = foreign_port(&registration, false);
    for (property, value) in [
        (P::FD_BBMD_ADDRESS, written(&host([10, 0, 1, 1]))),
        (P::FD_SUBSCRIPTION_LIFETIME, PropertyValue::Unsigned(120)),
    ] {
        let error = write(&mut db, property, value).unwrap_err();
        assert!(is(&error, ErrorCode::WRITE_ACCESS_DENIED), "{error:?}");
    }
    assert!(!pending(&db));
    assert!(registration.checked.lock().unwrap().is_empty());
    let writable = |db: &ObjectDatabase| {
        db.get(&oid())
            .unwrap()
            .property_metadata()
            .iter()
            .filter(|row| row.write_capability == PropertyWriteCapability::Always)
            .map(|row| row.property_identifier)
            .collect::<Vec<_>>()
    };
    assert!(!writable(&db).contains(&P::FD_BBMD_ADDRESS));
    db.set_network_port_activation_internal(oid(), true)
        .unwrap();
    let rows = writable(&db);
    assert!(rows.contains(&P::FD_BBMD_ADDRESS) && rows.contains(&P::FD_SUBSCRIPTION_LIFETIME));
}

#[test]
fn a_fixed_registration_refuses_every_written_value() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(
            1,
            "NP",
            BipPortConfig {
                ip_address: [127, 0, 0, 1],
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    let (_, _lease) = db
        .reserve_bip_port_internal(oid(), [127, 0, 0, 1], 47808)
        .unwrap();
    let mode = BipPortMode::fixed_foreign(host([10, 0, 0, 1]), 300);
    db.publish_bip_port_internal(oid(), [127, 0, 0, 1], 47808, 1476, mode)
        .unwrap();
    db.set_network_port_activation_internal(oid(), true)
        .unwrap();
    let error = write(
        &mut db,
        P::FD_SUBSCRIPTION_LIFETIME,
        PropertyValue::Unsigned(60),
    )
    .unwrap_err();
    assert!(is(&error, ErrorCode::WRITE_ACCESS_DENIED), "{error:?}");
    assert!(!pending(&db));
}
