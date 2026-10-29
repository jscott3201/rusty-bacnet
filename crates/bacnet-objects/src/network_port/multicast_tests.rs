//! BACnet_IP_Multicast_Address on an IPv4 port whose transport can use
//! B/IP multicast (Clause 12.56.26, Table 12-71 footnote 7, #941): present
//! only there, read live, and a written group waiting for activation.

use super::*;
use crate::database::ObjectDatabase;
use crate::property_metadata::{PropertyConformance, PropertyWriteCapability};
use bacnet_types::bip_port::{BipPortMode, IpMulticast};
use bacnet_types::enums::{ErrorClass, ErrorCode, PropertyIdentifier as P};
use bacnet_types::error::RemoteError;
use std::sync::{Arc, Mutex};

const GROUP: [u8; 4] = [239, 255, 94, 1];
const OFF: [u8; 4] = [0; 4];

/// A transport's group, as its live view would answer: only a class D
/// address or 0.0.0.0 is taken.
struct Group {
    group: Mutex<[u8; 4]>,
    activated: Mutex<Vec<[u8; 4]>>,
    /// Whether activation fails even after the check passed.
    fail_activation: bool,
}

impl Group {
    fn new(group: [u8; 4]) -> Arc<Self> {
        Arc::new(Self {
            group: Mutex::new(group),
            activated: Mutex::default(),
            fail_activation: false,
        })
    }
}

fn error(code: ErrorCode) -> Error {
    Error::protocol(ErrorClass::PROPERTY, code)
}

impl IpMulticast for Group {
    fn group(&self) -> [u8; 4] {
        *self.group.lock().unwrap()
    }
    fn check(&self, group: [u8; 4]) -> Result<(), Error> {
        if group == OFF || (224..=239).contains(&group[0]) {
            Ok(())
        } else {
            Err(error(ErrorCode::VALUE_OUT_OF_RANGE))
        }
    }
    fn activate(&self, group: [u8; 4]) -> Result<(), Error> {
        self.check(group)?;
        if self.fail_activation {
            return Err(error(ErrorCode::WRITE_ACCESS_DENIED));
        }
        *self.group.lock().unwrap() = group;
        self.activated.lock().unwrap().push(group);
        Ok(())
    }
    fn can_change(&self) -> bool {
        true
    }
}

fn oid() -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, 1).unwrap()
}

/// A NORMAL IPv4 port lent `group`, activating changes when `activation`
/// says so.
fn port(group: Option<&Arc<Group>>, activation: bool) -> (ObjectDatabase, Arc<()>) {
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
    db.publish_bip_port_internal(oid(), [127, 0, 0, 1], 47808, 1476, BipPortMode::Normal)
        .unwrap();
    let view = group.map(|group| Arc::clone(group) as Arc<dyn IpMulticast>);
    db.set_network_port_multicast_internal(oid(), view).unwrap();
    db.set_network_port_activation_internal(oid(), activation)
        .unwrap();
    (db, lease)
}

fn read(db: &ObjectDatabase, property: P) -> Result<PropertyValue, Error> {
    db.get(&oid()).unwrap().read_property(property, None)
}

fn group_read(db: &ObjectDatabase) -> Vec<u8> {
    match read(db, P::BACNET_IP_MULTICAST_ADDRESS) {
        Ok(PropertyValue::OctetString(octets)) => octets,
        other => panic!("an OCTET STRING, got {other:?}"),
    }
}

fn write(db: &mut ObjectDatabase, value: PropertyValue) -> Result<(), Error> {
    db.get_mut(&oid())
        .unwrap()
        .write_property(P::BACNET_IP_MULTICAST_ADDRESS, None, value, None)
}

fn octets(group: [u8; 4]) -> PropertyValue {
    PropertyValue::OctetString(group.to_vec())
}

fn code(result: Result<impl std::fmt::Debug, Error>) -> ErrorCode {
    match result {
        Err(Error::Remote(RemoteError::Error { code, .. })) => code,
        other => panic!("expected a protocol error, got {other:?}"),
    }
}

fn row(db: &ObjectDatabase) -> Option<crate::property_metadata::PropertyMetadata> {
    db.get(&oid())
        .unwrap()
        .property_metadata()
        .iter()
        .find(|row| row.property_identifier == P::BACNET_IP_MULTICAST_ADDRESS)
        .copied()
}

fn changes_pending(db: &ObjectDatabase) -> bool {
    read(db, P::CHANGES_PENDING).unwrap() == PropertyValue::Boolean(true)
}

fn activate(db: &mut ObjectDatabase) -> Result<bool, Error> {
    db.activate_network_port_changes().map(|ports| ports == 1)
}

#[test]
fn a_port_that_can_use_multicast_serves_its_group_live() {
    let group = Group::new(OFF);
    let (db, _lease) = port(Some(&group), true);
    let row = row(&db).expect("listed");
    assert_eq!(row.conformance, PropertyConformance::RequiredRead);
    assert!(
        db.get(&oid())
            .unwrap()
            .property_list()
            .contains(&P::BACNET_IP_MULTICAST_ADDRESS)
    );
    // 0.0.0.0 while off (Clause 12.56.26), then the group, read live.
    assert_eq!(group_read(&db), OFF);
    *group.group.lock().unwrap() = GROUP;
    assert_eq!(group_read(&db), GROUP);
}

#[test]
fn a_port_that_cant_has_no_row() {
    let (db, _lease) = port(None, true);
    assert!(row(&db).is_none());
    assert_eq!(
        code(read(&db, P::BACNET_IP_MULTICAST_ADDRESS)),
        ErrorCode::UNKNOWN_PROPERTY
    );
    // Nor does an unregistered port.
    let config = BipPortConfig::default();
    let object = NetworkPortObject::new_bip(1, "NP", config).unwrap();
    assert_eq!(
        code(object.read_property(P::BACNET_IP_MULTICAST_ADDRESS, None)),
        ErrorCode::UNKNOWN_PROPERTY
    );
}

#[test]
fn without_activation_the_row_is_read_only() {
    let group = Group::new(OFF);
    let (mut db, _lease) = port(Some(&group), false);
    assert_eq!(
        row(&db).unwrap().write_capability,
        PropertyWriteCapability::ReadOnly
    );
    assert_eq!(
        code(write(&mut db, octets(GROUP))),
        ErrorCode::WRITE_ACCESS_DENIED
    );
    assert!(!changes_pending(&db));
}

#[test]
fn a_written_group_waits_for_activation() {
    let group = Group::new(OFF);
    let (mut db, _lease) = port(Some(&group), true);
    assert_eq!(
        row(&db).unwrap().write_capability,
        PropertyWriteCapability::Always
    );
    write(&mut db, octets(GROUP)).unwrap();
    assert!(changes_pending(&db));
    // The written value reads back; the transport keeps its group.
    assert_eq!(group_read(&db), GROUP);
    assert_eq!(group.group(), OFF);
    db.check_network_port_changes().unwrap();
    assert!(activate(&mut db).unwrap());
    assert_eq!(group.group(), GROUP);
    assert_eq!(*group.activated.lock().unwrap(), [GROUP]);
    assert!(!changes_pending(&db));
    assert!(!activate(&mut db).unwrap(), "nothing left to activate");
    // 0.0.0.0 turns it off the same way.
    write(&mut db, octets(OFF)).unwrap();
    assert!(activate(&mut db).unwrap());
    assert_eq!(group.group(), OFF);
}

#[test]
fn a_bad_value_is_refused_when_written() {
    let group = Group::new(OFF);
    let (mut db, _lease) = port(Some(&group), true);
    assert_eq!(
        code(write(&mut db, octets([10, 0, 0, 1]))),
        ErrorCode::VALUE_OUT_OF_RANGE
    );
    assert_eq!(
        code(write(&mut db, PropertyValue::OctetString(vec![239, 0, 0]))),
        ErrorCode::VALUE_OUT_OF_RANGE
    );
    assert_eq!(
        code(write(&mut db, PropertyValue::Unsigned(1))),
        ErrorCode::INVALID_DATA_TYPE
    );
    assert_eq!(
        code(db.get_mut(&oid()).unwrap().write_property(
            P::BACNET_IP_MULTICAST_ADDRESS,
            Some(1),
            octets(GROUP),
            None
        )),
        ErrorCode::PROPERTY_IS_NOT_AN_ARRAY
    );
    assert!(!changes_pending(&db));
}

#[test]
fn a_refused_activation_leaves_the_group_waiting() {
    let group = Arc::new(Group {
        group: Mutex::new(OFF),
        activated: Mutex::default(),
        fail_activation: true,
    });
    let (mut db, _lease) = port(Some(&group), true);
    write(&mut db, octets(GROUP)).unwrap();
    assert_eq!(code(activate(&mut db)), ErrorCode::WRITE_ACCESS_DENIED);
    assert!(changes_pending(&db));
    assert_eq!(group_read(&db), GROUP);
    assert_eq!(group.group(), OFF);
}

#[test]
fn a_new_publication_drops_a_written_group() {
    let group = Group::new(OFF);
    let (mut db, _lease) = port(Some(&group), true);
    write(&mut db, octets(GROUP)).unwrap();
    db.set_network_port_multicast_internal(oid(), Some(group.clone() as Arc<dyn IpMulticast>))
        .unwrap();
    assert!(!changes_pending(&db));
    assert_eq!(group_read(&db), OFF);
}

#[test]
fn an_ipv6_port_never_serves_the_row() {
    let mut port = NetworkPortObject::new_bip6(1, "NP6", Bip6PortConfig::default()).unwrap();
    port.set_multicast_internal(Some(Group::new(GROUP) as Arc<dyn IpMulticast>));
    assert!(
        !port
            .property_list()
            .contains(&P::BACNET_IP_MULTICAST_ADDRESS)
    );
    assert_eq!(
        code(port.read_property(P::BACNET_IP_MULTICAST_ADDRESS, None)),
        ErrorCode::UNKNOWN_PROPERTY
    );
}
