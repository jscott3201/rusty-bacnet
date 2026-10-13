use super::*;
use crate::{
    network_port::{BipPortConfig, NetworkPortObject},
    property_metadata::PropertyWriteCapability,
};
use bacnet_types::bip_port::BipPortMode;
use bacnet_types::{enums::PropertyIdentifier as P, primitives::PropertyValue};
fn oid(instance: u32) -> ObjectIdentifier {
    ObjectIdentifier::new(ObjectType::NETWORK_PORT, instance).unwrap()
}
fn object(instance: u32) -> NetworkPortObject {
    NetworkPortObject::new_bip(
        instance,
        format!("port-{instance}"),
        BipPortConfig {
            ip_address: [127, 0, 0, 1],
            udp_port: 0,
            network_number: 17,
            apdu_length: 73,
            ..Default::default()
        },
    )
    .unwrap()
}
#[test]
fn registered_port_reservation_publication_and_final_release() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object(2))).unwrap();
    db.add(Box::new(object(1))).unwrap();
    let (_, owner) = db
        .reserve_bip_port_internal(oid(2), [127, 0, 0, 1], 0)
        .unwrap();
    assert_eq!(db.registered_bip_port_internal(), None);
    assert!(db
        .reserve_bip_port_internal(oid(1), [127, 0, 0, 1], 0)
        .is_err());
    assert!(db.remove(&oid(2)).is_err());
    assert!(db.add(Box::new(object(2))).is_err());
    assert!(db
        .with_object_adapter(&oid(2), |_| panic!("must refuse before callback"))
        .is_err());
    let selected = db.get_mut(&oid(2)).unwrap();
    assert!(selected
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .is_err());
    assert_eq!(
        selected.read_property(P::OUT_OF_SERVICE, None).unwrap(),
        PropertyValue::Boolean(false)
    );
    assert_eq!(
        selected
            .property_metadata()
            .iter()
            .find(|row| row.property_identifier == P::OUT_OF_SERVICE)
            .unwrap()
            .write_capability,
        PropertyWriteCapability::ReadOnly
    );
    assert!(db.remove(&oid(1)).unwrap().is_some());
    assert!(db
        .publish_bip_port_internal(oid(2), [127, 0, 0, 2], 1234, 1476, BipPortMode::Normal)
        .is_err());
    assert_eq!(db.registered_bip_port_internal(), None);
    db.publish_bip_port_internal(oid(2), [127, 0, 0, 1], 1234, 1476, BipPortMode::Normal)
        .unwrap();
    assert_eq!(db.registered_bip_port_internal(), Some(oid(2)));
    let selected = db.get(&oid(2)).unwrap();
    for (property, expected) in [
        (P::BACNET_IP_UDP_PORT, PropertyValue::Unsigned(1234)),
        (
            P::MAC_ADDRESS,
            PropertyValue::OctetString(vec![127, 0, 0, 1, 4, 210]),
        ),
        (P::APDU_LENGTH, PropertyValue::Unsigned(1476)),
        (P::NETWORK_NUMBER, PropertyValue::Unsigned(17)),
        (P::NETWORK_NUMBER_QUALITY, PropertyValue::Enumerated(3)),
        (P::CHANGES_PENDING, PropertyValue::Boolean(false)),
    ] {
        assert_eq!(selected.read_property(property, None).unwrap(), expected);
    }
    drop(owner);
    assert_eq!(db.registered_bip_port_internal(), None);
    db.get_mut(&oid(2))
        .unwrap()
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    assert!(db.remove(&oid(2)).unwrap().is_some());
}
#[test]
fn mismatched_or_unavailable_port_is_not_reserved() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object(1))).unwrap();
    for (selected, ip, udp) in [
        (oid(2), [127, 0, 0, 1], 0),
        (oid(1), [127, 0, 0, 2], 0),
        (oid(1), [127, 0, 0, 1], 1234),
    ] {
        assert!(db.reserve_bip_port_internal(selected, ip, udp).is_err());
    }
    db.get_mut(&oid(1))
        .unwrap()
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    assert!(db
        .reserve_bip_port_internal(oid(1), [127, 0, 0, 1], 0)
        .is_err());
    assert!(db.remove(&oid(1)).unwrap().is_some());
}

struct ForwardingPort {
    inner: NetworkPortObject,
    callbacks: Arc<std::sync::atomic::AtomicUsize>,
}
impl BACnetObject for ForwardingPort {
    fn object_identifier(&self) -> ObjectIdentifier {
        self.inner.object_identifier()
    }
    fn object_name(&self) -> &str {
        self.inner.object_name()
    }
    fn read_property(&self, _: P, _: Option<u32>) -> Result<PropertyValue, Error> {
        self.callbacks
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        panic!("wrapper reads must not be consulted for admission")
    }
    fn write_property(
        &mut self,
        _: P,
        _: Option<u32>,
        _: PropertyValue,
        _: Option<u8>,
    ) -> Result<(), Error> {
        panic!("wrapper writes must not be called")
    }
    fn property_list(&self) -> std::borrow::Cow<'static, [P]> {
        self.inner.property_list()
    }
}
#[test]
fn registered_port_forwarding_wrapper_is_refused_before_callbacks() {
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut db = ObjectDatabase::new();
    db.add(Box::new(ForwardingPort {
        inner: object(1),
        callbacks: calls.clone(),
    }))
    .unwrap();
    assert!(
        db.reserve_bip_port_internal(oid(1), [127, 0, 0, 1], 0)
            .is_err(),
        "a wrapper must not manufacture builtin registration"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(db.remove(&oid(1)).unwrap().is_some());
}

#[test]
fn registered_port_rejects_actual_identifier_different_from_selected_key() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(object(1))).unwrap();
    // Deliberately violate the adapter's documented identity-preservation
    // precondition. This is defensive admission, not a supported mutation path.
    db.with_object_adapter(&oid(1), |slot| *slot = Box::new(object(2)))
        .unwrap();
    assert!(db
        .reserve_bip_port_internal(oid(1), [127, 0, 0, 1], 0)
        .is_err());
    assert_eq!(db.registered_bip_port_internal(), None);
    assert_eq!(db.get(&oid(1)).unwrap().object_identifier(), oid(2));
    // Failed admission neither takes a lease nor changes the object's state.
    db.get_mut(&oid(1))
        .unwrap()
        .write_property(P::OUT_OF_SERVICE, None, PropertyValue::Boolean(true), None)
        .unwrap();
    assert!(db.remove(&oid(1)).unwrap().is_some());
}

#[test]
fn network_number_new_reservation_resets_learned_state_from_configured_provenance() {
    let mut db = ObjectDatabase::new();
    db.add(Box::new(
        NetworkPortObject::new_bip(
            2,
            "selected",
            BipPortConfig {
                ip_address: [127, 0, 0, 1],
                udp_port: 0,
                ..Default::default()
            },
        )
        .unwrap(),
    ))
    .unwrap();
    let (_, lease) = db
        .reserve_bip_port_internal(oid(2), [127, 0, 0, 1], 0)
        .unwrap();
    db.publish_bip_port_internal(oid(2), [127, 0, 0, 1], 40000, 1476, BipPortMode::Normal)
        .unwrap();
    assert_eq!(
        db.network_number_internal(oid(2), Some((19, 1)))
            .unwrap()
            .snapshot(),
        (19, 2)
    );
    assert_eq!(
        db.configured_bip_port_internal(&oid(2))
            .unwrap()
            .network_number,
        0
    );
    assert!(db.network_number_internal(oid(1), Some((99, 1))).is_none());
    drop(lease);
    assert!(db.network_number_internal(oid(2), Some((99, 1))).is_none());
    let (_, lease) = db
        .reserve_bip_port_internal(oid(2), [127, 0, 0, 1], 40000)
        .unwrap();
    db.publish_bip_port_internal(oid(2), [127, 0, 0, 1], 40000, 1476, BipPortMode::Normal)
        .unwrap();
    assert_eq!(
        db.network_number_internal(oid(2), None).unwrap().snapshot(),
        (0, 0)
    );
    drop(lease);
}
